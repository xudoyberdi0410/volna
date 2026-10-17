//! The native player inside the daemon (`docs/native-player.md` §4.5, §9):
//! the `volna_player` engine with the app's hooks, the §3.3 commands, events
//! to the window process while one is connected, the tray and MPRIS.

pub mod commands;
pub mod convert;
pub mod forward;
pub mod hooks;
pub mod journal;
#[cfg(target_os = "linux")]
pub mod mpris;
pub mod tray;

use std::path::Path;
use std::sync::Arc;

use volna_player::{Player, PlayerHandle};

use crate::state::AppState;
use crate::wave::WaveState;
use journal::Journal;

pub struct PlayerState {
    /// `None` when GStreamer could not start; commands answer `unavailable:`.
    handle: Option<PlayerHandle>,
    pub journal: Arc<Journal>,
}

impl PlayerState {
    pub fn handle(&self) -> Result<&PlayerHandle, String> {
        self.handle
            .as_ref()
            .ok_or_else(|| "unavailable: the audio engine (GStreamer) did not start".to_owned())
    }

    pub fn handle_opt(&self) -> Option<&PlayerHandle> {
        self.handle.as_ref()
    }
}

/// Starts the engine (restoring the queue paused). Call inside the tokio
/// runtime, after the audio server is bound.
pub fn setup(
    data: &Path,
    config: &Path,
    state: &AppState,
    wave: &WaveState,
    scrobble: &volna_scrobble::Scrobbling,
) -> PlayerState {
    let journal = Arc::new(Journal::open(&data.join("player-journal.jsonl")));
    let handle = match volna_player::init_gstreamer() {
        Ok(()) => {
            let hooks = hooks::AppHooks {
                stream_base: state.stream_base.clone(),
                streams: state.streams.clone(),
                wave: wave.engine(),
                ytm: wave.ytm().clone(),
                journal: journal.clone(),
                scrobble: scrobble.clone(),
            };
            let paths = volna_player::persist::Paths {
                queue: data.join("player-queue.json"),
                settings: config.join("player.json"),
            };
            Some(Player::spawn(hooks, paths))
        }
        Err(e) => {
            tracing::error!("{e}; playback is disabled");
            None
        }
    };
    PlayerState { handle, journal }
}
