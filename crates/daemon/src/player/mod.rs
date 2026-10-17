//! The native player inside the daemon (`docs/native-player.md` §4.5, §9):
//! the `volna_core` player (queue, wave, hooks, journal, §3.3 commands) on
//! the GStreamer output, plus the tray and MPRIS.

#[cfg(target_os = "linux")]
pub mod mpris;
pub mod tray;

pub use volna_core::player::{PlayerState, commands, convert, forward, hooks, journal};

/// The GStreamer output, or `None` (logged) when GStreamer cannot start.
pub fn gstreamer_output() -> Option<volna_player::GstOutput> {
    match volna_player::init_gstreamer() {
        Ok(()) => Some(volna_player::GstOutput::new()),
        Err(e) => {
            tracing::error!("{e}; playback is disabled");
            None
        }
    }
}
