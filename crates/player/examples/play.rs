//! `cargo run -p volna-player --example play -- [flags] <uri|path>...`
//!
//! Plays files or URIs through the real engine and sink. Flags:
//! `--crossfade <s>`, `--no-gapless`, `--eq <dB>` (1 kHz peak), `--mono`,
//! `--widen <0..2>`, `--crossfeed`, `--speed <x>`, `--seek <s>`, `--vis`.
//! Keys on stdin: `n` next, `p` prev, `t` toggle, `s <s>` seek, `q` quit.

use std::collections::HashMap;
use std::io::BufRead;
use std::sync::Arc;

use volna_player::persist::Paths;
use volna_player::*;

struct Files(Arc<HashMap<String, String>>);

impl Hooks for Files {
    fn stream_uri(&self, t: &PlayerTrack, _: Quality) -> Option<String> {
        self.0.get(&t.id).cloned()
    }
    fn prefetch(&self, _: &PlayerTrack, _: Quality) {}
    async fn failure(&self, _: &str) -> Option<FailureInfo> {
        None
    }
    async fn stream_info(&self, _: &str, _: Quality) -> Option<StreamInfo> {
        None
    }
    async fn more(&self, _: MoreRequest) -> Result<Vec<TrackInput>, String> {
        Ok(vec![])
    }
    async fn on_play_event(&self, ev: PlayEvent) -> PlayEventOutcome {
        println!("play event: {ev:?}");
        PlayEventOutcome::default()
    }
    async fn wave_stop(&self) {}
}

fn uri(arg: &str) -> String {
    if arg.contains("://") {
        arg.to_owned()
    } else {
        let p = std::fs::canonicalize(arg).expect("file exists");
        format!("file://{}", p.display())
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,volna_player=debug".into()),
        )
        .init();
    init_gstreamer().expect("GStreamer");

    let mut args = std::env::args().skip(1).peekable();
    let mut settings = serde_json::Map::new();
    let mut dsp = DspConfig::default();
    let mut seek = None;
    let mut vis = false;
    let mut uris = Vec::new();
    while let Some(a) = args.next() {
        let mut val = || args.next().expect("flag value").parse::<f64>().expect("number");
        match a.as_str() {
            "--crossfade" => {
                settings.insert("crossfade".into(), serde_json::json!({"enabled": true, "seconds": val()}));
            }
            "--no-gapless" => {
                settings.insert("gapless".into(), false.into());
            }
            "--speed" => {
                settings.insert("speed".into(), val().into());
            }
            "--seek" => seek = Some(val()),
            "--eq" => {
                dsp.eq = EqConfig {
                    enabled: true,
                    preamp_db: 0.0,
                    bands: vec![EqBand {
                        freq: 1000.0,
                        gain_db: val(),
                        q: 1.0,
                        kind: BandType::Peaking,
                        channel: BandChannel::Stereo,
                    }],
                };
            }
            "--mono" => dsp.mono = true,
            "--widen" => {
                dsp.binaural.enabled = true;
                dsp.binaural.crossfeed.enabled = false;
                dsp.binaural.widening = WideningConfig {
                    enabled: true,
                    amount: val(),
                };
            }
            "--crossfeed" => {
                dsp.binaural.enabled = true;
                dsp.binaural.crossfeed.enabled = true;
                dsp.binaural.widening.enabled = false;
            }
            "--vis" => vis = true,
            other => uris.push(uri(other)),
        }
    }
    assert!(!uris.is_empty(), "give at least one file or URI");

    let mut map = HashMap::new();
    let tracks: Vec<TrackInput> = uris
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let id = format!("example{i:04}");
            map.insert(id.clone(), u.clone());
            TrackInput {
                id,
                title: u.rsplit('/').next().unwrap_or(u).to_owned(),
                ..Default::default()
            }
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("volna-play-{}", std::process::id()));
    let paths = Paths {
        queue: dir.join("queue.json"),
        settings: dir.join("player.json"),
    };
    let player = Player::spawn(Files(Arc::new(map)), paths);
    player.set_settings(settings, false).await.expect("settings");
    player.set_dsp(dsp).await.expect("dsp");
    if vis {
        player
            .visualizer_start(
                10,
                Box::new(|f| {
                    let bins = &f[24..536];
                    let level = bins.iter().map(|b| u32::from(*b)).sum::<u32>() / 512;
                    println!("vis: mean spectrum byte {level}");
                    true
                }),
            )
            .await
            .unwrap();
    }
    player.set_queue(tracks, 0, true, seek).await.expect("queue");

    let mut events = player.subscribe();
    tokio::spawn(async move {
        while let Ok(ev) = events.recv().await {
            match ev {
                PlayerEvent::Position(_) => {}
                PlayerEvent::State(s) => println!(
                    "state: {:?} uid {:?} {:.1}/{:?} xfade {} stream {:?}",
                    s.status, s.uid, s.position, s.duration, s.crossfading, s.stream
                ),
                PlayerEvent::Queue(q) => println!("queue: rev {} index {} order {:?}", q.rev, q.index, q.order),
                other => println!("{other:?}"),
            }
        }
    });

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let mut ended = player.watch_now();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            line = rx.recv(), if stdin_open => {
                let Some(line) = line else {
                    stdin_open = false;
                    continue;
                };
                let mut parts = line.split_whitespace();
                let r = match parts.next() {
                    Some("n") => player.next().await,
                    Some("p") => player.prev().await,
                    Some("t") => player.toggle().await,
                    Some("s") => player.seek(parts.next().and_then(|v| v.parse().ok()).unwrap_or(0.0), false).await,
                    Some("q") => break,
                    _ => Ok(()),
                };
                if let Err(e) = r {
                    println!("error: {e}");
                }
            }
            changed = ended.changed() => {
                if changed.is_err() || ended.borrow().state.status == Status::Ended {
                    break;
                }
            }
        }
    }
    player.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
