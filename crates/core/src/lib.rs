//! `volna-core`: everything the Volna UI calls, minus desktop hosting
//! (`docs/flutter-ui-plan.md`, phase 6). YouTube Music, the network, «Моя
//! волна», the library, lyrics, scrobbling, the Yandex import, cover colours,
//! the stream service with its loopback audio server, settings, and the
//! player's queue/wave logic on an [`volna_player::Output`] the host gives.
//!
//! One surface for every host: [`Core::call`] (command name + JSON args →
//! JSON, the same names and payloads as the daemon IPC), events through
//! [`Core::set_event_sink`], and [`Core::proto`] for `volna://` images.
//! The desktop daemon wraps it with the socket, GStreamer, MPRIS and the
//! tray; the mobile apps reach it through the C ABI in `ffi` (feature `ffi`).

pub mod api;
pub mod commands;
mod core;
pub mod dispatch;
#[cfg(feature = "ffi")]
pub mod ffi;
pub mod js;
pub mod media;
pub mod network;
pub mod player;
pub mod protocol;
pub mod settings;
pub mod shape;
pub mod state;
pub mod wave;
#[cfg(feature = "innertube")]
pub mod youtube;

pub use crate::core::{Core, CoreConfig, Dirs, IDENTIFIER};
pub use state::{AppState, StreamBackend};

/// Name of the custom URI scheme for images and audio.
pub const SCHEME: &str = "volna";
