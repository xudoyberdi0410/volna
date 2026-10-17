//! The native player inside the app (`docs/native-player.md` §4.5): the
//! `volna_player` engine with the app's hooks, the §3.3 commands, events
//! to the window while it exists, the window lifecycle, the tray and MPRIS.

pub mod commands;
mod convert;
mod forward;
mod hooks;
mod journal;
#[cfg(target_os = "linux")]
mod mpris;
mod tray;
pub mod window;

use std::sync::Arc;

use tauri::Manager;
use volna_player::{Player, PlayerHandle};

use crate::state::AppState;
use crate::wave::WaveState;
use journal::Journal;

pub struct PlayerState {
    /// `None` when GStreamer could not start; commands answer `unavailable:`.
    handle: Option<PlayerHandle>,
    pub journal: Arc<Journal>,
    pub close: window::CloseGate,
}

impl PlayerState {
    pub fn handle(&self) -> Result<&PlayerHandle, String> {
        self.handle
            .as_ref()
            .ok_or_else(|| "unavailable: the audio engine (GStreamer) did not start".to_owned())
    }
}

/// Starts the engine (restoring the queue paused), the event forwarder, the
/// tray and MPRIS. Call in `setup` after the audio server is bound.
pub fn setup(app: &tauri::App, state: &AppState, wave: &WaveState) -> anyhow::Result<PlayerState> {
    let data = app.path().app_data_dir()?;
    let config = app.path().app_config_dir()?;
    let journal = Arc::new(Journal::open(&data.join("player-journal.jsonl")));
    let handle = match volna_player::init_gstreamer() {
        Ok(()) => {
            let hooks = hooks::AppHooks {
                stream_base: state.stream_base.clone(),
                streams: state.streams.clone(),
                wave: wave.engine(),
                ytm: wave.ytm().clone(),
                journal: journal.clone(),
            };
            let paths = volna_player::persist::Paths {
                queue: data.join("player-queue.json"),
                settings: config.join("player.json"),
            };
            let _rt = tauri::async_runtime::handle().inner().enter();
            Some(Player::spawn(hooks, paths))
        }
        Err(e) => {
            tracing::error!("{e}; playback is disabled");
            None
        }
    };
    forward::start(app.handle(), handle.clone(), &journal);
    tray::start(app.handle(), handle.clone());
    #[cfg(target_os = "linux")]
    if let Some(h) = &handle {
        mpris::start(app.handle(), h.clone(), state.images.clone());
    }
    Ok(PlayerState {
        handle,
        journal,
        close: window::CloseGate::default(),
    })
}
