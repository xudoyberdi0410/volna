//! The engine against real GStreamer: generated WAV files played into a
//! synchronised fakesink, with fake hooks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use tokio::sync::broadcast;
use volna_player::persist::Paths;
use volna_player::*;

static INIT: Once = Once::new();

fn init() {
    INIT.call_once(|| {
        // SAFETY: set once before any thread reads it.
        unsafe { std::env::set_var("VOLNA_PLAYER_SINK", "fakesink sync=true") };
        init_gstreamer().expect("GStreamer");
    });
}

/// A WAV of `secs` seconds of a sine at `freq`.
fn make_wav(dir: &Path, name: &str, secs: f64, freq: f64) -> PathBuf {
    use gst::prelude::*;
    let path = dir.join(format!("{name}.wav"));
    let buffers = (secs * 48000.0 / 1024.0).ceil() as i32;
    let desc = format!(
        "audiotestsrc num-buffers={buffers} samplesperbuffer=1024 freq={freq} ! audio/x-raw,rate=48000,channels=2 ! wavenc ! filesink location={}",
        path.display()
    );
    let p = gst::parse::launch(&desc).unwrap();
    p.set_state(gst::State::Playing).unwrap();
    let bus = p.bus().unwrap();
    bus.timed_pop_filtered(gst::ClockTime::from_seconds(10), &[gst::MessageType::Eos, gst::MessageType::Error])
        .unwrap();
    p.set_state(gst::State::Null).unwrap();
    path
}

#[derive(Default)]
struct FakeState {
    uris: HashMap<String, String>,
    events: Vec<String>,
    more: Vec<TrackInput>,
    requeue_more: Vec<TrackInput>,
    more_calls: usize,
    requeue_next_end: bool,
}

#[derive(Clone, Default)]
struct Fake(Arc<Mutex<FakeState>>);

impl Hooks for Fake {
    fn stream_uri(&self, t: &PlayerTrack, _: Quality) -> Option<String> {
        self.0.lock().unwrap().uris.get(&t.id).cloned()
    }
    fn prefetch(&self, _: &PlayerTrack, _: Quality) {}
    async fn failure(&self, id: &str) -> Option<FailureInfo> {
        id.starts_with("missing").then(|| FailureInfo {
            kind: ErrorKind::Unavailable,
            message: "gone".into(),
        })
    }
    async fn stream_info(&self, _: &str, q: Quality) -> Option<StreamInfo> {
        Some(StreamInfo {
            codec: Some("pcm".into()),
            bitrate_kbps: Some(1536.0),
            quality: q,
            mime: Some("audio/wav".into()),
        })
    }
    async fn more(&self, req: MoreRequest) -> Result<Vec<TrackInput>, String> {
        let mut s = self.0.lock().unwrap();
        s.more_calls += 1;
        match req {
            MoreRequest::WaveStart { .. } if s.more.is_empty() => {
                Err("no_candidates: nothing".into())
            }
            MoreRequest::WaveNext { .. } => Ok(vec![]),
            MoreRequest::WaveRequeue { .. } => Ok(std::mem::take(&mut s.requeue_more)),
            _ => Ok(std::mem::take(&mut s.more)),
        }
    }
    async fn on_play_event(&self, ev: PlayEvent) -> PlayEventOutcome {
        let mut s = self.0.lock().unwrap();
        let line = match &ev {
            PlayEvent::Start { track, .. } => format!("start {}", track.title),
            PlayEvent::History { track, .. } => format!("history {}", track.title),
            PlayEvent::End { track, ended, .. } => format!("end {} {ended}", track.title),
            PlayEvent::Dislike { track, .. } => format!("dislike {}", track.title),
        };
        s.events.push(line);
        PlayEventOutcome {
            requeue_wave: matches!(ev, PlayEvent::End { .. }) && std::mem::take(&mut s.requeue_next_end),
        }
    }
    async fn wave_stop(&self) {
        self.0.lock().unwrap().events.push("wave_stop".into());
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    paths: Paths,
    fake: Fake,
    player: PlayerHandle,
    rx: broadcast::Receiver<PlayerEvent>,
}

fn id(n: usize) -> String {
    format!("track{n:06}")
}

fn input(n: usize) -> TrackInput {
    TrackInput {
        id: id(n),
        title: format!("t{n}"),
        duration: None,
        ..Default::default()
    }
}

fn rig(lengths: &[f64]) -> Rig {
    init();
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::default();
    for (n, secs) in lengths.iter().enumerate() {
        let p = make_wav(dir.path(), &id(n), *secs, 220.0 * (n + 1) as f64);
        fake.0
            .lock()
            .unwrap()
            .uris
            .insert(id(n), format!("file://{}", p.display()));
    }
    let paths = Paths {
        queue: dir.path().join("player-queue.json"),
        settings: dir.path().join("player.json"),
    };
    let player = Player::spawn(fake.clone(), paths.clone());
    let rx = player.subscribe();
    Rig {
        _dir: dir,
        paths,
        fake,
        player,
        rx,
    }
}

impl Rig {
    /// Waits for an event matching `f` (draining the rest).
    async fn wait<T>(&mut self, secs: u64, mut f: impl FnMut(&PlayerEvent) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let ev = tokio::time::timeout_at(deadline, self.rx.recv())
                .await
                .expect("timed out waiting for an event");
            match ev {
                Ok(ev) => {
                    if let Some(t) = f(&ev) {
                        return t;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }

    async fn wait_status(&mut self, secs: u64, status: Status) -> PlaybackState {
        self.wait(secs, |e| match e {
            PlayerEvent::State(s) if s.status == status => Some(s.clone()),
            _ => None,
        })
        .await
    }

    fn events(&self) -> Vec<String> {
        self.fake.0.lock().unwrap().events.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn plays_through_the_queue_gaplessly_and_ends() {
    let mut r = rig(&[1.5, 1.5]);
    let rev = r.player.set_queue(vec![input(0), input(1)], 0, true, None).await.unwrap();
    assert!(rev >= 1);
    let s = r.wait_status(10, Status::Playing).await;
    let first = s.uid.clone().unwrap();
    // The second entry starts on its own.
    let s = r
        .wait(10, |e| match e {
            PlayerEvent::State(s) if s.uid.as_deref() != Some(first.as_str()) && s.uid.is_some() => Some(s.clone()),
            _ => None,
        })
        .await;
    assert_eq!(s.track_id.as_deref(), Some(id(1).as_str()));
    r.wait_status(10, Status::Ended).await;
    let snap = r.player.snapshot().await.unwrap();
    assert_eq!(snap.queue.index, 1);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ev = r.events();
    assert_eq!(ev, ["start t0", "end t0 true", "start t1", "end t1 true"], "{ev:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn transport_commands() {
    let mut r = rig(&[6.0, 6.0, 6.0]);
    r.player
        .set_queue(vec![input(0), input(1), input(2)], 0, false, None)
        .await
        .unwrap();
    let snap = r.player.snapshot().await.unwrap();
    assert_eq!(snap.state.status, Status::Paused);
    assert_eq!(snap.queue.items.len(), 3);
    r.player.play().await.unwrap();
    r.wait_status(10, Status::Playing).await;
    r.player.next().await.unwrap();
    let s = r.wait_status(10, Status::Playing).await;
    assert_eq!(s.track_id.as_deref(), Some(id(1).as_str()));
    r.player.seek(4.0, false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let snap = r.player.snapshot().await.unwrap();
    assert!(snap.state.position >= 4.0 && snap.state.position < 5.5, "{}", snap.state.position);
    // > 3 s: prev restarts the track.
    r.player.prev().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let snap = r.player.snapshot().await.unwrap();
    assert_eq!(snap.state.track_id.as_deref(), Some(id(1).as_str()));
    assert!(snap.state.position < 1.5);
    r.player.prev().await.unwrap();
    let s = r.wait_status(10, Status::Playing).await;
    assert_eq!(s.track_id.as_deref(), Some(id(0).as_str()));
    r.player.pause().await.unwrap();
    let snap = r.player.snapshot().await.unwrap();
    assert_eq!(snap.state.status, Status::Paused);
    let p1 = snap.state.position;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let p2 = r.player.snapshot().await.unwrap().state.position;
    assert!((p2 - p1).abs() < 0.3, "{p1} {p2}");
    // Shuffle/repeat and queue ops answer with consecutive revs.
    let rev = r.player.set_shuffle(true).await.unwrap();
    let rev2 = r.player.set_repeat(Repeat::All).await.unwrap();
    assert_eq!(rev2, rev + 1);
    let uid = r.player.snapshot().await.unwrap().queue.items[2].uid.clone();
    r.player.remove(uid.clone()).await.unwrap();
    assert!(r.player.remove(uid).await.unwrap_err().to_string().starts_with("unknown_uid: "));
    let err = r
        .player
        .enqueue(
            vec![TrackInput {
                id: "local:file".into(),
                ..Default::default()
            }],
            EnqueueAt::End,
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "bad_args");
}

#[tokio::test(flavor = "multi_thread")]
async fn autoplay_continues_at_the_end() {
    let mut r = rig(&[1.0, 1.0]);
    r.fake.0.lock().unwrap().more = vec![input(1)];
    let patch = serde_json::json!({"autoplay": true});
    r.player
        .set_settings(patch.as_object().unwrap().clone(), false)
        .await
        .unwrap();
    r.player.set_queue(vec![input(0)], 0, true, None).await.unwrap();
    let q = r
        .wait(10, |e| match e {
            PlayerEvent::Queue(q) if q.order.len() == 2 => Some(q.clone()),
            _ => None,
        })
        .await;
    assert_eq!(q.added[0].origin, Origin::Autoplay);
    let s = r
        .wait(10, |e| match e {
            PlayerEvent::State(s) if s.track_id.as_deref() == Some(id(1).as_str()) && s.status == Status::Playing => {
                Some(s.clone())
            }
            _ => None,
        })
        .await;
    assert!(s.uid.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn failures_skip_and_mark_unavailable() {
    let mut r = rig(&[2.0]);
    r.fake.0.lock().unwrap().uris.insert(
        "missing0000".into(),
        "file:///nonexistent/volna-missing.wav".into(),
    );
    let bad = TrackInput {
        id: "missing0000".into(),
        title: "bad".into(),
        ..Default::default()
    };
    r.player.set_queue(vec![bad, input(0)], 0, true, None).await.unwrap();
    let err = r
        .wait(10, |e| match e {
            PlayerEvent::Error(err) => Some(err.clone()),
            _ => None,
        })
        .await;
    assert_eq!(err.kind, ErrorKind::Unavailable);
    assert_eq!(err.action, ErrorAction::Skipped);
    assert_eq!(err.streak, 1);
    let snap = r.player.snapshot().await.unwrap();
    assert!(snap.queue.items[0].unavailable);
    let s = r.wait_status(10, Status::Playing).await;
    assert_eq!(s.track_id.as_deref(), Some(id(0).as_str()));
}

#[tokio::test(flavor = "multi_thread")]
async fn three_failures_in_a_row_stop() {
    let mut r = rig(&[]);
    let tracks: Vec<TrackInput> = (0..4)
        .map(|n| {
            let id = format!("missing{n:04}");
            r.fake
                .0
                .lock()
                .unwrap()
                .uris
                .insert(id.clone(), "file:///nonexistent/x.wav".into());
            TrackInput {
                id,
                ..Default::default()
            }
        })
        .collect();
    r.player.set_queue(tracks, 0, true, None).await.unwrap();
    let err = r
        .wait(20, |e| match e {
            PlayerEvent::Error(err) if err.action == ErrorAction::Stopped => Some(err.clone()),
            _ => None,
        })
        .await;
    assert_eq!(err.streak, 3);
    let snap = r.player.snapshot().await.unwrap();
    assert_eq!(snap.state.status, Status::Paused);
    assert_eq!(snap.queue.index, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn crossfade_overlaps_and_advances_once() {
    let mut r = rig(&[4.0, 4.0]);
    let patch = serde_json::json!({"crossfade": {"enabled": true, "seconds": 1}});
    r.player
        .set_settings(patch.as_object().unwrap().clone(), false)
        .await
        .unwrap();
    let mut a = input(0);
    a.duration = Some(4.0);
    let mut b = input(1);
    b.duration = Some(4.0);
    r.player.set_queue(vec![a, b], 0, true, None).await.unwrap();
    let s = r
        .wait(15, |e| match e {
            PlayerEvent::State(s) if s.crossfading => Some(s.clone()),
            _ => None,
        })
        .await;
    assert_eq!(s.track_id.as_deref(), Some(id(1).as_str()));
    r.wait(5, |e| match e {
        PlayerEvent::State(s) if !s.crossfading => Some(()),
        _ => None,
    })
    .await;
    r.wait_status(10, Status::Ended).await;
    let ev = r.events();
    assert_eq!(ev.iter().filter(|e| e.starts_with("start")).count(), 2, "{ev:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn wave_start_stop_and_requeue() {
    let mut r = rig(&[3.0, 3.0, 3.0, 3.0]);
    // Cold: no candidates.
    let err = r.player.wave_start(5).await.unwrap_err();
    assert_eq!(err.code, "no_candidates");
    r.fake.0.lock().unwrap().more = vec![input(0), input(1), input(2)];
    r.player.wave_start(5).await.unwrap();
    r.wait(5, |e| matches!(e, PlayerEvent::Wave { active: true }).then_some(()))
        .await;
    let snap = r.player.snapshot().await.unwrap();
    assert!(snap.wave.active);
    assert!(snap.queue.items.iter().all(|t| t.origin == Origin::Wave));
    r.wait_status(10, Status::Playing).await;
    // Let the play start (> 0.5 s) so that skipping it ends a play.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // A re-seed after the current play ends drops queued wave tracks.
    {
        let mut s = r.fake.0.lock().unwrap();
        s.requeue_next_end = true;
        s.requeue_more = vec![input(3)];
    }
    r.player.next().await.unwrap();
    r.wait(10, |e| match e {
        PlayerEvent::Queue(q) if q.added.iter().any(|t| t.id == id(3)) => Some(()),
        _ => None,
    })
    .await;
    let snap = r.player.snapshot().await.unwrap();
    let ids: Vec<&str> = snap.queue.items.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, [id(0), id(1), id(3)]);
    // Something else plays: the wave stops.
    r.player.enqueue(vec![input(2)], EnqueueAt::Next).await.unwrap();
    r.player.next().await.unwrap();
    r.wait(10, |e| matches!(e, PlayerEvent::Wave { active: false }).then_some(()))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(r.events().contains(&"wave_stop".to_owned()));
}

#[tokio::test(flavor = "multi_thread")]
async fn restores_paused_at_the_saved_position() {
    let mut r = rig(&[8.0]);
    r.player.set_queue(vec![input(0)], 0, true, None).await.unwrap();
    r.wait_status(10, Status::Playing).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    r.player.shutdown().await.unwrap();
    let fake = r.fake.clone();
    let player = Player::spawn(fake, r.paths.clone());
    let snap = player.snapshot().await.unwrap();
    assert_eq!(snap.state.status, Status::Paused);
    assert_eq!(snap.queue.items.len(), 1);
    assert!(snap.state.position > 1.0, "{}", snap.state.position);
    assert!(r.events().iter().any(|e| e == "end t0 false"));
    // Resumes where it was.
    let mut rx = player.subscribe();
    player.play().await.unwrap();
    loop {
        if let Ok(PlayerEvent::Position(p)) = rx.recv().await
            && p.playing
        {
            assert!(p.position > 1.0, "{}", p.position);
            break;
        }
    }
    player.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn visualizer_frames_flow_while_subscribed() {
    let mut r = rig(&[4.0]);
    let frames = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let sink_frames = frames.clone();
    r.player
        .visualizer_start(
            30,
            Box::new(move |f| {
                sink_frames.lock().unwrap().push(f);
                true
            }),
        )
        .await
        .unwrap();
    r.player.set_queue(vec![input(0)], 0, true, None).await.unwrap();
    r.wait_status(10, Status::Playing).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    r.player.visualizer_stop().await.unwrap();
    let n = frames.lock().unwrap().len();
    let loud = frames
        .lock()
        .unwrap()
        .iter()
        .filter(|f| f[10] & 1 == 1 && f[24..536].iter().any(|b| *b > 100))
        .count();
    assert!(n >= 10, "{n} frames");
    assert!(loud >= 5, "{loud} frames with a spectrum");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(frames.lock().unwrap().len(), n, "no frames after stop");
}
