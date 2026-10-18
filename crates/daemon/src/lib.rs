//! Volna without a GUI toolkit (`docs/native-player.md` §9): the `volna`
//! daemon hosts `volna-core` (commands, network, «Моя волна», library,
//! streams and the player logic) with the desktop parts: the IPC socket,
//! GStreamer output, MPRIS, the tray and the `volna://` handler. The
//! `volna-window` process (the Flutter app in `app/`) shows the UI and talks
//! to the daemon over [`ipc`].

pub mod daemon;
pub mod dispatch;
pub mod ipc;
pub mod memory;
pub mod player;
pub mod protocol;
pub mod state;

// Moved to `volna-core`; re-exported under their old paths.
pub use volna_core::{api, commands, media, network, settings, wave};

pub use state::AppState;

/// Name of the custom URI scheme served to the window.
pub const SCHEME: &str = "volna";
