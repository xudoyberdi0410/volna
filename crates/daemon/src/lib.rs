//! Volna without the webview (`docs/native-player.md` §9): the `volna`
//! daemon owns playback, the audio server, the network, «Моя волна», MPRIS,
//! the tray and the `volna://` handler; the `volna-window` process (the
//! Tauri app in `src-tauri`) shows the UI and forwards everything here over
//! [`ipc`].
//!
//! `api_emu` still lives in `src-tauri/src/api_emu` (it has uncommitted
//! edits there) and is compiled into this crate from that path.

pub mod api;
#[path = "../../../src-tauri/src/api_emu/mod.rs"]
pub mod api_emu;
pub mod commands;
pub mod daemon;
pub mod dispatch;
pub mod ipc;
pub mod media;
pub mod memory;
pub mod network;
pub mod player;
pub mod protocol;
pub mod settings;
pub mod state;
pub mod wave;

pub use state::AppState;

/// Name of the custom URI scheme served to the webview.
pub const SCHEME: &str = "volna";
