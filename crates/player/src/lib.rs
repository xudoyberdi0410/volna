//! Native audio player of Volna (`docs/native-player.md`): GStreamer
//! `playbin3` decks, the queue, «Моя волна» continuation hooks, the
//! `volnadsp` EQ element and the visualizer analysis. No Tauri here: the app
//! drives it through [`PlayerHandle`] and implements [`Hooks`].
//!
//! ```no_run
//! # async fn demo<H: volna_player::Hooks>(hooks: H, paths: volna_player::persist::Paths) {
//! volna_player::init_gstreamer().unwrap();
//! let player = volna_player::Player::spawn(hooks, paths);
//! let mut events = player.subscribe();
//! player.play().await.unwrap();
//! # }
//! ```

pub mod analysis;
mod deck;
pub mod dsp;
mod engine;
pub mod hooks;
pub mod model;
pub mod persist;
pub mod queue;
pub mod tracker;

use std::fmt;

use tokio::sync::{broadcast, mpsc, oneshot, watch};

pub use engine::{EnqueueAt, FrameSink, NowPlaying};
pub use hooks::{FailureInfo, Hooks, MoreRequest, PlayEventOutcome};
pub use model::*;
pub use queue::{Queue, QueueState, Removed};
pub use tracker::{PlayEvent, PlayTracker};

use engine::{Command, Engine};

/// Crosses IPC as `"<code>: <message>"` (§3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    pub(crate) fn new_owned(code: String, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

/// What the player broadcasts (§3.4, plus what MPRIS needs).
#[derive(Debug, Clone)]
pub enum PlayerEvent {
    State(PlaybackState),
    Position(PositionEvent),
    Queue(QueueChanged),
    Error(PlayerError),
    /// `external`: changed outside the UI (MPRIS, tray).
    Settings {
        settings: PlayerSettings,
        dsp: DspConfig,
        external: bool,
    },
    Sleep {
        ends_at: Option<f64>,
    },
    Wave {
        active: bool,
    },
    QueueLow {
        remaining: usize,
    },
    /// A seek (for the MPRIS `Seeked` signal).
    Seeked {
        uid: Option<String>,
        position: f64,
    },
}

/// Registers `volnadsp` and initialises GStreamer. Call once, early.
pub fn init_gstreamer() -> Result<(), String> {
    gst::init().map_err(|e| format!("GStreamer: {e}"))?;
    dsp::register().map_err(|e| format!("volnadsp: {e}"))?;
    for name in [
        "playbin3",
        "autoaudiosink",
        "audioconvert",
        "audioresample",
        "capsfilter",
    ] {
        if gst::ElementFactory::find(name).is_none() {
            return Err(format!("GStreamer element {name} is missing"));
        }
    }
    if gst::ElementFactory::find("scaletempo").is_none() {
        tracing::warn!("scaletempo is missing: speed changes will shift the pitch");
    }
    Ok(())
}

pub struct Player;

impl Player {
    /// Starts the actor on the current tokio runtime, restoring the saved
    /// queue (paused) and settings from `paths`.
    pub fn spawn<H: Hooks>(hooks: H, paths: persist::Paths) -> PlayerHandle {
        let ch = Engine::spawn(hooks, paths);
        PlayerHandle {
            cmd: ch.cmd,
            events: ch.events,
            now: ch.now,
        }
    }
}

/// Cheap to clone; every method maps to a §3.3 command.
#[derive(Clone)]
pub struct PlayerHandle {
    cmd: mpsc::UnboundedSender<Command>,
    events: broadcast::Sender<PlayerEvent>,
    now: watch::Receiver<NowPlaying>,
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

impl PlayerHandle {
    async fn call<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T, Error> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(make(tx))
            .map_err(|_| Error::new("unavailable", "the player stopped"))?;
        rx.await
            .map_err(|_| Error::new("unavailable", "the player stopped"))?
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.events.subscribe()
    }

    /// The latest light state (no round trip).
    pub fn now(&self) -> NowPlaying {
        self.now.borrow().clone()
    }

    pub fn watch_now(&self) -> watch::Receiver<NowPlaying> {
        self.now.clone()
    }

    pub async fn snapshot(&self) -> Result<PlayerSnapshot, Error> {
        self.call(Command::Snapshot).await
    }
    pub async fn play(&self) -> Result<(), Error> {
        self.call(Command::Play).await
    }
    pub async fn pause(&self) -> Result<(), Error> {
        self.call(Command::Pause).await
    }
    pub async fn toggle(&self) -> Result<(), Error> {
        self.call(Command::Toggle).await
    }
    pub async fn stop(&self) -> Result<(), Error> {
        self.call(Command::Stop).await
    }
    pub async fn next(&self) -> Result<(), Error> {
        self.call(Command::Next).await
    }
    pub async fn prev(&self) -> Result<(), Error> {
        self.call(Command::Prev).await
    }
    pub async fn seek(&self, position: f64, resume: bool) -> Result<(), Error> {
        self.call(|reply| Command::Seek {
            position,
            resume,
            reply,
        })
        .await
    }
    pub async fn play_uid(&self, uid: String, position: Option<f64>) -> Result<(), Error> {
        self.call(|reply| Command::PlayUid {
            uid,
            position,
            reply,
        })
        .await
    }
    pub async fn set_queue(
        &self,
        tracks: Vec<TrackInput>,
        start: usize,
        play: bool,
        position: Option<f64>,
    ) -> Result<u64, Error> {
        self.call(|reply| Command::SetQueue {
            tracks,
            start,
            play,
            position,
            reply,
        })
        .await
    }
    pub async fn enqueue(&self, tracks: Vec<TrackInput>, at: EnqueueAt) -> Result<u64, Error> {
        self.call(|reply| Command::Enqueue { tracks, at, reply })
            .await
    }
    pub async fn remove(&self, uid: String) -> Result<u64, Error> {
        self.call(|reply| Command::Remove { uid, reply }).await
    }
    pub async fn move_to(&self, uid: String, to: usize) -> Result<u64, Error> {
        self.call(|reply| Command::Move { uid, to, reply }).await
    }
    pub async fn clear(&self, keep_current: bool) -> Result<u64, Error> {
        self.call(|reply| Command::Clear {
            keep_current,
            reply,
        })
        .await
    }
    pub async fn set_shuffle(&self, on: bool) -> Result<u64, Error> {
        self.call(|reply| Command::SetShuffle { on, reply }).await
    }
    pub async fn set_repeat(&self, mode: Repeat) -> Result<u64, Error> {
        self.call(|reply| Command::SetRepeat { mode, reply }).await
    }
    /// `external`: the change comes from MPRIS or the tray, not the UI.
    pub async fn set_settings(
        &self,
        patch: serde_json::Map<String, serde_json::Value>,
        external: bool,
    ) -> Result<PlayerSettings, Error> {
        self.call(|reply| Command::SetSettings {
            patch,
            external,
            reply,
        })
        .await
    }
    pub async fn set_dsp(&self, config: DspConfig) -> Result<(), Error> {
        self.call(|reply| Command::SetDsp { config, reply }).await
    }
    pub async fn set_sleep_timer(&self, minutes: Option<f64>) -> Result<Option<f64>, Error> {
        self.call(|reply| Command::SetSleep { minutes, reply })
            .await
    }
    pub async fn set_continuation(
        &self,
        kind: ContinuationKind,
        seeds: Vec<TrackInput>,
    ) -> Result<(), Error> {
        self.call(|reply| Command::SetContinuation { kind, seeds, reply })
            .await
    }
    pub async fn wave_start(&self, count: usize) -> Result<u64, Error> {
        self.call(|reply| Command::WaveStart { count, reply }).await
    }
    pub async fn wave_stop(&self) -> Result<(), Error> {
        self.call(Command::WaveStop).await
    }
    /// Drops the upcoming wave entries and fetches fresh ones (no-op
    /// without an active wave).
    pub async fn wave_requeue(&self) -> Result<u64, Error> {
        self.call(Command::WaveRequeue).await
    }
    pub async fn wave_dislike(&self) -> Result<(), Error> {
        self.call(Command::WaveDislike).await
    }
    pub async fn visualizer_start(&self, fps: u32, sink: FrameSink) -> Result<(), Error> {
        self.call(|reply| Command::VisualizerStart { fps, sink, reply })
            .await
    }
    pub async fn visualizer_stop(&self) -> Result<(), Error> {
        self.call(Command::VisualizerStop).await
    }
    /// Saves state, closes the current play and releases the decks.
    pub async fn shutdown(&self) -> Result<(), Error> {
        self.call(Command::Shutdown).await
    }
}
