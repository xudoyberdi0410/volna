//! The engine on a fake platform player: what a mobile app sees.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use volna_player::{
    FailureInfo, Hooks, MoreRequest, PlayEvent, PlayEventOutcome, Player, PlayerTrack, Quality,
    StreamInfo, TrackInput,
};

use super::forward::{EventSink, event_payload};
use super::platform::{CommandPlayer, PlatformOutput, PlayerCommand, PlayerReport};

fn vid(n: usize) -> String {
    format!("trackid{n:04}")
}

fn input(n: usize) -> TrackInput {
    TrackInput {
        id: vid(n),
        title: format!("t{n}"),
        duration: Some(200.0),
        ..Default::default()
    }
}

/// The platform plays for a moment: a position past the start, then the
/// engine's ticks (every 250 ms) turn it into a play.
async fn listen(out: &PlatformOutput) {
    out.report(PlayerReport::Position {
        position_ms: 1_000,
        duration_ms: None,
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
}

/// A platform player that only records what it is told.
fn fake_player() -> (PlatformOutput, UnboundedReceiver<PlayerCommand>) {
    let (tx, rx) = unbounded_channel();
    let out = PlatformOutput::with_player(Arc::new(CommandPlayer(move |c| {
        let _ = tx.send(c);
    })));
    (out, rx)
}

/// The next command other than a volume change.
async fn next_cmd(rx: &mut UnboundedReceiver<PlayerCommand>) -> PlayerCommand {
    loop {
        let c = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("no player command")
            .expect("player gone");
        if !matches!(c, PlayerCommand::SetVolume { .. }) {
            return c;
        }
    }
}

fn loaded_id(c: &PlayerCommand) -> String {
    match c {
        PlayerCommand::Load { url, .. } => url.rsplit('/').next().unwrap().to_owned(),
        other => panic!("expected a load, got {other:?}"),
    }
}

#[derive(Default)]
struct Wave {
    next_calls: usize,
    served: usize,
    plays: Vec<String>,
}

/// Hooks with a fake wave: 3 tracks to start, 5 more per refill.
#[derive(Clone, Default)]
struct FakeHooks(Arc<Mutex<Wave>>);

impl FakeHooks {
    fn batch(&self, n: usize) -> Vec<TrackInput> {
        let mut w = self.0.lock().unwrap();
        let out = (w.served..w.served + n).map(input).collect();
        w.served += n;
        out
    }
}

impl Hooks for FakeHooks {
    fn stream_uri(&self, t: &PlayerTrack, _: Quality) -> Option<String> {
        Some(format!("http://127.0.0.1:9/{}", t.id))
    }
    fn prefetch(&self, _: &PlayerTrack, _: Quality) {}
    async fn failure(&self, _: &str) -> Option<FailureInfo> {
        None
    }
    async fn stream_info(&self, _: &str, _: Quality) -> Option<StreamInfo> {
        None
    }
    async fn more(&self, req: MoreRequest) -> Result<Vec<TrackInput>, String> {
        match req {
            MoreRequest::WaveStart { count } => Ok(self.batch(count)),
            MoreRequest::WaveNext { .. } => {
                self.0.lock().unwrap().next_calls += 1;
                Ok(self.batch(5))
            }
            _ => Ok(vec![]),
        }
    }
    async fn on_play_event(&self, ev: PlayEvent) -> PlayEventOutcome {
        if let PlayEvent::End { track, ended, .. } = ev {
            self.0
                .lock()
                .unwrap()
                .plays
                .push(format!("{} {ended}", track.id));
        }
        PlayEventOutcome::default()
    }
    async fn wave_stop(&self) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn wave_plays_through_the_platform_player_and_refills() {
    let dir = tempfile::tempdir().unwrap();
    let (out, mut cmds) = fake_player();
    let hooks = FakeHooks::default();
    let paths = volna_player::persist::Paths {
        queue: dir.path().join("q.json"),
        settings: dir.path().join("s.json"),
    };
    let player = Player::spawn_with(hooks.clone(), out.clone(), paths);
    let mut events = player.subscribe();

    player.wave_start(3).await.unwrap();
    let first = next_cmd(&mut cmds).await;
    assert_eq!(loaded_id(&first), vid(0));
    out.report(PlayerReport::Ready {
        duration_ms: Some(200_000),
    });
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Play);
    out.report(PlayerReport::Playing);
    listen(&out).await;

    // Two tracks left after the first: the wave tops the queue up.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let s = player.snapshot().await.unwrap();
        if s.queue.items.len() == 8 && !s.queue.continuation.fetching {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "no refill");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(hooks.0.lock().unwrap().next_calls, 1);

    // The platform says the track ended: the next one loads.
    out.report(PlayerReport::Position {
        position_ms: 199_000,
        duration_ms: Some(200_000),
    });
    out.report(PlayerReport::Ended);
    // The finished item is unloaded first.
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Stop);
    let second = next_cmd(&mut cmds).await;
    assert_eq!(loaded_id(&second), vid(1));
    out.report(PlayerReport::Ready { duration_ms: None });
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Play);
    out.report(PlayerReport::Playing);
    let snap = player.snapshot().await.unwrap();
    assert_eq!(snap.state.track_id.as_deref(), Some(vid(1).as_str()));
    assert!(snap.wave.active);

    // The same `player:*` events as on desktop.
    let mut names = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let Some((name, _)) = event_payload(ev) {
            names.push(name);
        }
    }
    for want in ["player:state", "player:queue", "player:position", "player:wave"] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    player.shutdown().await.unwrap();
    assert_eq!(
        hooks.0.lock().unwrap().plays.first().map(String::as_str),
        Some(format!("{} true", vid(0)).as_str())
    );
}

/// No resolver needed: the fake platform never fetches.
struct NoBackend;

impl volna_streams::Backend for NoBackend {
    async fn resolve(
        &self,
        _: &volna_streams::VideoId,
        _: volna_streams::Quality,
        _: Option<&str>,
    ) -> volna_streams::Result<volna_streams::ResolvedStream> {
        Err(volna_streams::StreamError::YtDlpNotFound)
    }
}

/// Events as `(name, payload)`, like the window or the FFI callback gets them.
struct Collect(Mutex<Vec<(String, Value)>>);

impl EventSink for Collect {
    fn connected(&self) -> bool {
        true
    }
    fn emit(&self, name: &str, payload: Value) {
        self.0.lock().unwrap().push((name.to_owned(), payload));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn core_commands_drive_the_platform_player() {
    let dir = tempfile::tempdir().unwrap();
    let (out, mut cmds) = fake_player();
    let config = crate::CoreConfig {
        dirs: crate::Dirs::under(dir.path().to_owned(), None),
        proxy: false,
        audio_server: true,
        backend: Some(crate::StreamBackend::injected(NoBackend)),
        codec: volna_streams::Codec::default(),
    };
    let core = crate::Core::start(config, Some(out.clone())).await.unwrap();
    let events = Arc::new(Collect(Mutex::new(Vec::new())));
    core.set_event_sink(Some(events.clone()));
    let base = core.app.stream_base.clone().expect("audio server");

    core.call("player_set_settings", &json!({"patch": {"autoplay": false}}))
        .await
        .unwrap();
    let tracks: Vec<_> = (0..2).map(|n| json!({"id": vid(n), "title": format!("t{n}")})).collect();
    let rev = core
        .call(
            "player_set_queue",
            &json!({"tracks": tracks, "startIndex": 0, "play": true}),
        )
        .await
        .unwrap();
    assert!(rev["rev"].as_u64().is_some());

    // The platform player gets the loopback URL, never googlevideo.
    match next_cmd(&mut cmds).await {
        PlayerCommand::Load { url, start_ms } => {
            assert_eq!(url, format!("{base}/{}", vid(0)));
            assert_eq!(start_ms, 0);
        }
        other => panic!("{other:?}"),
    }
    out.report(PlayerReport::Ready {
        duration_ms: Some(180_000),
    });
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Play);
    out.report(PlayerReport::Playing);
    listen(&out).await;

    core.call("player_pause", &Value::Null).await.unwrap();
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Pause);
    out.report(PlayerReport::Paused);
    core.call("player_seek", &json!({"position": 42.5, "resume": true}))
        .await
        .unwrap();
    assert_eq!(
        next_cmd(&mut cmds).await,
        PlayerCommand::Seek {
            position_ms: 42_500
        }
    );
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Play);
    out.report(PlayerReport::Playing);

    out.report(PlayerReport::Ended);
    assert_eq!(next_cmd(&mut cmds).await, PlayerCommand::Stop);
    match next_cmd(&mut cmds).await {
        PlayerCommand::Load { url, .. } => assert!(url.ends_with(&vid(1)), "{url}"),
        other => panic!("{other:?}"),
    }
    let snap = core.call("player_snapshot", &Value::Null).await.unwrap();
    assert_eq!(snap["state"]["trackId"], vid(1));
    assert_eq!(snap["queue"]["index"], 1);

    // The first play reaches the journal (written in the background), as on desktop.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let journal = core.call("player_journal", &json!({"after": 0})).await.unwrap();
        let ended = journal
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "end" && e["track"]["id"] == vid(0));
        if ended {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "{journal}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let names: Vec<String> = events.0.lock().unwrap().iter().map(|e| e.0.clone()).collect();
    for want in ["player:state", "player:queue", "player:position", "player:journal"] {
        assert!(names.iter().any(|n| n == want), "{want} missing from {names:?}");
    }
    core.shutdown().await;
}
