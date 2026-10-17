//! The window comes and goes; the player stays (§4.5).
//!
//! Close: the UI gets `app:will-close` and about 700 ms to flush, then the
//! webview is destroyed (WebKitWebProcess exits) and Volna stays in the
//! tray. Show (tray, MPRIS Raise, second launch) builds a fresh `main`
//! window from the config; the page boots and rehydrates (§3.6). Quit is
//! the only way out: tray, MPRIS or `app_quit`.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use super::PlayerState;

/// Set once a real exit started; closing the window then just closes it.
static QUITTING: AtomicBool = AtomicBool::new(false);
static CREATING: AtomicBool = AtomicBool::new(false);

const CLOSE_GRACE: Duration = Duration::from_millis(700);

pub fn quitting() -> bool {
    QUITTING.load(Ordering::SeqCst)
}

#[derive(Default)]
pub struct CloseGate {
    closing: AtomicBool,
    ready: Mutex<Option<oneshot::Sender<()>>>,
}

impl CloseGate {
    /// `app_ready_to_close`.
    pub fn ready(&self) {
        if let Some(tx) = self.ready.lock().expect("close gate").take() {
            let _ = tx.send(());
        }
    }
}

/// The close button: hide into the tray instead of exiting.
pub fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    if window.label() != "main" || quitting() {
        return;
    }
    api.prevent_close();
    let app = window.app_handle().clone();
    let Some(player) = app.try_state::<PlayerState>() else {
        let _ = window.destroy();
        return;
    };
    if player.close.closing.swap(true, Ordering::SeqCst) {
        return;
    }
    let (tx, rx) = oneshot::channel();
    *player.close.ready.lock().expect("close gate") = Some(tx);
    if let Err(e) = app.emit_to("main", "app:will-close", serde_json::json!({})) {
        tracing::debug!("cannot emit app:will-close: {e}");
    }
    tauri::async_runtime::spawn(async move {
        let flushed = tokio::time::timeout(CLOSE_GRACE, rx).await.is_ok();
        tracing::info!(flushed, "closing the window; Volna stays in the tray");
        if let Some(w) = app.get_webview_window("main") {
            #[cfg(target_os = "linux")]
            release_web_process(&w);
            let _ = w.destroy();
        }
        if let Some(player) = app.try_state::<PlayerState>() {
            player.close.closing.store(false, Ordering::SeqCst);
        }
    });
}

/// WebKitGTK keeps a closed page's WebProcess in its process cache for
/// reuse (hundreds of MB, for up to half an hour). Ending it here is the
/// point of closing the window; a new window starts a fresh one.
#[cfg(target_os = "linux")]
fn release_web_process(w: &tauri::WebviewWindow) {
    let result = w.with_webview(|wv| {
        use webkit2gtk::WebViewExt;
        wv.inner().terminate_web_process();
    });
    if let Err(e) = result {
        tracing::debug!("cannot end the web process: {e}");
    }
}

/// The `main` webview is gone: nothing should keep feeding it.
pub fn on_destroyed(app: &AppHandle) {
    if let Some(player) = app.try_state::<PlayerState>()
        && let Ok(h) = player.handle()
    {
        let h = h.clone();
        tauri::async_runtime::spawn(async move {
            let _ = h.visualizer_stop().await;
        });
    }
    #[cfg(target_os = "linux")]
    trim_heap(app);
}

/// With no page left, ends WebKit's NetworkProcess (it belongs to the web
/// context Tauri keeps; WebKit starts a new one for the next window) and
/// returns freed heap to the system.
#[cfg(target_os = "linux")]
fn trim_heap(app: &AppHandle) {
    unsafe extern "C" {
        fn malloc_trim(pad: usize) -> i32;
        fn kill(pid: i32, sig: i32) -> i32;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if app.get_webview_window("main").is_none() {
            for pid in webkit_children("WebKitNetworkPr") {
                tracing::debug!(pid, "ending the WebKit network process");
                // SAFETY: plain syscall on our own child process.
                unsafe { kill(pid, 15) };
            }
        }
        // SAFETY: glibc call without preconditions; a no-op under other allocators.
        unsafe { malloc_trim(0) };
    });
}

/// Child processes of ours whose `comm` starts with `name`.
#[cfg(target_os = "linux")]
fn webkit_children(name: &str) -> Vec<i32> {
    let me = std::process::id().to_string();
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return vec![];
    };
    dir.filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
        .filter(|pid| {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            // pid (comm) state ppid ...
            let Some((comm, rest)) = stat.split_once(") ") else {
                return false;
            };
            comm.split_once(" (")
                .is_some_and(|(_, c)| c.starts_with(name))
                && rest.split_whitespace().nth(1) == Some(me.as_str())
        })
        .collect()
}

/// Shows the window, creating it if it was closed.
pub fn show_or_create(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Building from a task, not from a main-thread callback (Windows deadlocks).
    tauri::async_runtime::spawn(async move {
        if app.get_webview_window("main").is_none()
            && let Some(cfg) = app.config().app.windows.first().cloned()
        {
            match tauri::WebviewWindowBuilder::from_config(&app, &cfg).and_then(|b| b.build()) {
                Ok(w) => {
                    let _ = w.set_focus();
                    crate::webview_memory::apply(&app);
                    tracing::info!("window created");
                }
                Err(e) => tracing::error!("cannot create the window: {e}"),
            }
        }
        CREATING.store(false, Ordering::SeqCst);
    });
}

/// The real exit: save the player, then leave (xray stops on `RunEvent::Exit`).
pub fn quit(app: &AppHandle) {
    if QUITTING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Some(player) = app.try_state::<PlayerState>()
            && let Ok(h) = player.handle()
        {
            let h = h.clone();
            if tokio::time::timeout(Duration::from_secs(5), h.shutdown())
                .await
                .is_err()
            {
                tracing::warn!("player shutdown timed out");
            }
        }
        app.exit(0);
    });
}
