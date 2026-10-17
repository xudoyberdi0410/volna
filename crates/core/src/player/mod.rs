//! The player behind the `player_*` commands: the `volna_player` engine with
//! the app's hooks (streams, «Моя волна», radio, history, scrobbling, the
//! journal), on whatever [`Output`] the host gives it — GStreamer in the
//! desktop daemon, ExoPlayer / AVPlayer through [`platform`] on mobile.

pub mod commands;
pub mod convert;
pub mod forward;
pub mod hooks;
pub mod journal;
pub mod platform;

use std::path::Path;
use std::sync::Arc;

use volna_player::{Output, Player, PlayerHandle};

use crate::api::library::Library;
use crate::state::AppState;
use crate::wave::WaveState;
use journal::Journal;

pub struct PlayerState {
    /// `None` when the output could not start; commands answer `unavailable:`.
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

/// Starts the engine on `output` (restoring the queue paused). Call inside
/// the tokio runtime, after the audio server is bound.
pub fn setup<O: Output>(
    data: &Path,
    config: &Path,
    state: &AppState,
    wave: &WaveState,
    library: &Arc<Library>,
    scrobble: &volna_scrobble::Scrobbling,
    output: Option<O>,
) -> PlayerState {
    let journal = Arc::new(Journal::open(&data.join("player-journal.jsonl")));
    let handle = output.map(|output| {
        let hooks = hooks::AppHooks {
            stream_base: state.stream_base.clone(),
            streams: state.streams.clone(),
            wave: wave.engine(),
            ytm: wave.ytm().clone(),
            journal: journal.clone(),
            library: library.clone(),
            scrobble: scrobble.clone(),
        };
        let paths = volna_player::persist::Paths {
            queue: data.join("player-queue.json"),
            settings: config.join("player.json"),
        };
        Player::spawn_with(hooks, output, paths)
    });
    PlayerState { handle, journal }
}
