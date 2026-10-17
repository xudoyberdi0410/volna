//! `volna-window`: the webview process (`docs/native-player.md` §9).
//!
//! A thin Tauri shell around the UI: every command the page invokes and
//! every `volna://` request is forwarded to the `volna` daemon
//! (`crates/daemon`), and the daemon's events are re-emitted to the page.
//! Only the Yandex sign-in (it opens its own window) and the close handshake
//! run here. Closing the window ends this process; the daemon keeps playing.

mod shell;
mod webview_memory;
mod yandex;

/// `yandex.rs` reads `crate::state::AppState`.
pub use volna_daemon::state;
pub use volna_daemon::{AppState, SCHEME};

/// Builds and runs the window process.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,volna_lib=debug".into()),
        )
        .init();
    shell::run();
}
