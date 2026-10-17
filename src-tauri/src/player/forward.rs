//! Player events → the `main` webview (§3.4), only while it exists.
//! Without a window nothing crosses IPC.

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::broadcast::error::RecvError;
use volna_player::{PlayerEvent, PlayerHandle};

use super::journal::Journal;

fn has_window(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some()
}

fn emit<S: serde::Serialize + Clone>(app: &AppHandle, name: &str, payload: S) {
    if let Err(e) = app.emit_to("main", name, payload) {
        tracing::debug!("cannot emit {name}: {e}");
    }
}

pub fn start(app: &AppHandle, handle: Option<PlayerHandle>, journal: &Journal) {
    {
        let app = app.clone();
        journal.set_listener(move |entry| {
            if has_window(&app) {
                emit(&app, "player:journal", entry.clone());
            }
        });
    }
    let Some(handle) = handle else {
        return;
    };
    let app = app.clone();
    let mut rx = handle.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                // The UI resyncs on a rev gap (§3.6).
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!("event forwarder lagged by {n}");
                    continue;
                }
                Err(RecvError::Closed) => break,
            };
            if !has_window(&app) {
                continue;
            }
            match ev {
                PlayerEvent::State(s) => emit(&app, "player:state", s),
                PlayerEvent::Position(p) => emit(&app, "player:position", p),
                PlayerEvent::Queue(q) => emit(&app, "player:queue", q),
                PlayerEvent::Error(e) => emit(&app, "player:error", e),
                PlayerEvent::Settings {
                    settings,
                    dsp,
                    external: true,
                } => emit(
                    &app,
                    "player:settings",
                    json!({ "settings": settings, "dsp": dsp }),
                ),
                PlayerEvent::Settings { .. } => {}
                PlayerEvent::Sleep { ends_at } => {
                    emit(&app, "player:sleep", json!({ "endsAt": ends_at }))
                }
                PlayerEvent::Wave { active } => {
                    emit(&app, "player:wave", json!({ "active": active }))
                }
                PlayerEvent::QueueLow { remaining } => {
                    emit(&app, "player:queue-low", json!({ "remaining": remaining }))
                }
                PlayerEvent::Seeked { .. } => {}
            }
        }
    });
}
