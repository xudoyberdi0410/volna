//! The window process side of the split: connect to the daemon (starting it
//! when needed), forward `invoke` calls and `volna://` requests, re-emit
//! daemon events, and the close handshake.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, JavaScriptChannelId};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use volna_daemon::ipc::{self, Client, Frame, Msg, Role};

use crate::{SCHEME, webview_memory, yandex};

/// Commands this process answers itself; everything else goes to the daemon.
const LOCAL: &[&str] = &["yandex_export", "app_ready_to_close"];
/// How long the page gets to flush after `app:will-close`.
const CLOSE_GRACE: Duration = Duration::from_millis(700);

static APP: OnceLock<AppHandle> = OnceLock::new();
/// The page's visualizer channel; frames from the daemon go here.
static VIS: Mutex<Option<Channel<InvokeResponseBody>>> = Mutex::new(None);
/// Answered by `app_ready_to_close`.
static READY: Mutex<Option<oneshot::Sender<()>>> = Mutex::new(None);

fn exit_now() {
    match APP.get() {
        Some(app) => app.exit(0),
        None => std::process::exit(0),
    }
}

fn on_push(f: Frame) {
    match f.msg {
        Msg::Event { name, payload } => {
            if let Some(app) = APP.get()
                && let Err(e) = app.emit_to("main", &name, payload)
            {
                tracing::debug!("cannot emit {name}: {e}");
            }
        }
        Msg::Vis => {
            let mut vis = VIS.lock().expect("vis channel");
            if let Some(ch) = vis.as_ref()
                && ch.send(InvokeResponseBody::Raw(f.body)).is_err()
            {
                *vis = None;
            }
        }
        Msg::Show => {
            if let Some(w) = APP.get().and_then(|a| a.get_webview_window("main")) {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        Msg::Quit => exit_now(),
        other => tracing::debug!("unexpected IPC message {other:?}"),
    }
}

/// `volna` next to this executable (`VOLNA_DAEMON_BIN` overrides).
fn daemon_exe() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("VOLNA_DAEMON_BIN") {
        return p.into();
    }
    let name = if cfg!(windows) { "volna.exe" } else { "volna" };
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| name.into())
}

/// Connects to the daemon, starting it (in the tray) when none runs.
async fn connect() -> Result<(Client, Option<String>), String> {
    let mut started = false;
    for _ in 0..100 {
        match ipc::connect().await {
            Ok(stream) => {
                return Client::start(stream, Role::Window, Box::new(on_push), Box::new(exit_now))
                    .await
                    .map_err(|e| e.to_string());
            }
            Err(_) if !started => {
                started = true;
                let exe = daemon_exe();
                tracing::info!("starting the daemon: {}", exe.display());
                std::process::Command::new(&exe)
                    .arg("--tray")
                    .spawn()
                    .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    Err("the Volna daemon did not start".into())
}

/// `window.__VOLNA__.streamBase`, as before the split (the daemon's audio server).
fn stream_base_plugin(stream_base: Option<&str>) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let value = serde_json::to_string(&stream_base).expect("a string serialises");
    let script = format!(
        "window.__VOLNA__ = Object.assign(window.__VOLNA__ || {{}}, {{ streamBase: {value} }});"
    );
    tauri::plugin::Builder::new("volna-stream")
        .js_init_script(script)
        .build()
}

#[tauri::command]
fn app_ready_to_close() {
    if let Some(tx) = READY.lock().expect("close gate").take() {
        let _ = tx.send(());
    }
}

/// The close button: let the page flush, then end this process.
fn on_close_requested(window: &tauri::Window, api: &tauri::CloseRequestApi) {
    if window.label() != "main" {
        return;
    }
    api.prevent_close();
    let mut ready = READY.lock().expect("close gate");
    if ready.is_some() {
        return;
    }
    let (tx, rx) = oneshot::channel();
    *ready = Some(tx);
    drop(ready);
    let app = window.app_handle().clone();
    if let Err(e) = app.emit_to("main", "app:will-close", serde_json::json!({})) {
        tracing::debug!("cannot emit app:will-close: {e}");
    }
    tauri::async_runtime::spawn(async move {
        let flushed = tokio::time::timeout(CLOSE_GRACE, rx).await.is_ok();
        tracing::info!(flushed, "window closed; the daemon keeps playing");
        app.exit(0);
    });
}

/// `player_visualizer_start {channel, fps}`: keep the channel here, ask the
/// daemon for frames.
fn visualizer_args(webview: tauri::Webview, args: &Value) -> Value {
    let channel = args
        .get("channel")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<JavaScriptChannelId>().ok())
        .map(|id| id.channel_on::<_, InvokeResponseBody>(webview));
    *VIS.lock().expect("vis channel") = channel;
    serde_json::json!({ "fps": args.get("fps").cloned().unwrap_or(Value::Null) })
}

/// The daemon's view of the yt-dlp cookies, for the Yandex sign-in.
fn local_state() -> anyhow::Result<crate::AppState> {
    let dirs = volna_daemon::daemon::Dirs::new()?;
    let state = crate::AppState::new(&dirs.cache)?;
    let settings = volna_daemon::settings::SettingsStore::new(&dirs.config).load();
    if let (Ok(settings), Some(ytdlp)) = (settings, state.ytdlp())
        && let Err(e) = ytdlp.set_cookies(settings.ytdlp_cookies)
    {
        tracing::warn!("yt-dlp cookies for the Yandex sign-in: {e}");
    }
    Ok(state)
}

pub fn run() {
    let (client, stream_base) = match tauri::async_runtime::block_on(connect()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("{e}");
            std::process::exit(1);
        }
    };
    let client = Arc::new(client);
    let local: Box<dyn Fn(tauri::ipc::Invoke) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![
            yandex::yandex_export,
            app_ready_to_close
        ]);
    let fwd = client.clone();
    let proto = client.clone();

    let app = tauri::Builder::default()
        .plugin(stream_base_plugin(stream_base.as_deref()))
        .setup(|app| {
            let _ = APP.set(app.handle().clone());
            app.manage(local_state()?);
            app.manage(yandex::YandexState::default());
            webview_memory::apply(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                on_close_requested(window, api);
            }
        })
        .invoke_handler(move |invoke| {
            let cmd = invoke.message.command().to_owned();
            if LOCAL.contains(&cmd.as_str()) {
                return local(invoke);
            }
            let mut args = match invoke.message.payload() {
                InvokeBody::Json(v) => v.clone(),
                InvokeBody::Raw(_) => Value::Null,
            };
            match cmd.as_str() {
                "player_visualizer_start" => {
                    args = visualizer_args(invoke.message.webview(), &args)
                }
                "player_visualizer_stop" => *VIS.lock().expect("vis channel") = None,
                _ => {}
            }
            let (client, resolver) = (fwd.clone(), invoke.resolver);
            tauri::async_runtime::spawn(async move {
                match client.call(&cmd, args).await {
                    Ok(v) => resolver.resolve(v),
                    Err(e) => resolver.reject(e),
                }
            });
            true
        })
        .register_asynchronous_uri_scheme_protocol(SCHEME, move |_ctx, request, responder| {
            let client = proto.clone();
            tauri::async_runtime::spawn(async move {
                let (parts, body) = request.into_parts();
                let headers = parts
                    .headers
                    .iter()
                    .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_owned())))
                    .collect();
                let resp = client
                    .proto(
                        parts.method.to_string(),
                        parts.uri.to_string(),
                        headers,
                        body,
                    )
                    .await;
                let response = match resp {
                    Ok((status, headers, body)) => {
                        let mut b = http::Response::builder().status(status);
                        for (k, v) in headers {
                            b = b.header(k, v);
                        }
                        b.body(body)
                    }
                    Err(e) => http::Response::builder()
                        .status(http::StatusCode::SERVICE_UNAVAILABLE)
                        .header(http::header::CONTENT_TYPE, "application/json")
                        .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                        .body(serde_json::json!({ "error": e }).to_string().into_bytes()),
                };
                responder.respond(response.unwrap_or_else(|_| http::Response::new(Vec::new())));
            });
        })
        .build(tauri::generate_context!())
        .expect("error while building the Volna window");
    app.run(|_, _| {});
}
