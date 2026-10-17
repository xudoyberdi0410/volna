//! Volna without the webview (`docs/native-player.md` §9): the `volna`
//! daemon hosts `volna-core` (commands, network, «Моя волна», library,
//! streams and the player logic) with the desktop parts: the IPC socket,
//! GStreamer output, MPRIS, the tray and the `volna://` handler. The
//! `volna-window` process (the Tauri app in `src-tauri`) shows the UI and
//! forwards everything here over [`ipc`].
//!
//! `api_emu` still lives in `src-tauri/src/api_emu` and is compiled into
//! this crate from that path.

#[path = "../../../src-tauri/src/api_emu/mod.rs"]
pub mod api_emu;
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

/// Name of the custom URI scheme served to the webview.
pub const SCHEME: &str = "volna";
