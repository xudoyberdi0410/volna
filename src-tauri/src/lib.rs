//! Volna desktop app: Tauri shell that wires the `volna://` protocol to the
//! workspace crates (`ytm`, `net`, `streams`, `wave`).

mod api_emu;
mod commands;
mod media;
mod network;
mod protocol;
mod settings;
mod state;

use tauri::Manager;
use volna_streams::LoopbackServer;

pub use state::AppState;

/// Name of the custom URI scheme served to the webview.
pub const SCHEME: &str = "volna";

/// Builds and runs the Tauri application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,volna=debug,volna_lib=debug,volna_streams=debug".into()),
        )
        .init();

    // Bound before the webview exists so its URL can go into an init script.
    let audio_server = LoopbackServer::bind()
        .inspect_err(|e| tracing::error!("cannot bind the audio server: {e}"))
        .ok();
    let stream_base = audio_server.as_ref().map(LoopbackServer::stream_base);

    let app = tauri::Builder::default()
        .plugin(stream_base_plugin(stream_base.as_deref()))
        .setup(move |app| {
            let cache_dir = app.path().app_cache_dir()?;
            let mut state = AppState::new(&cache_dir)?;
            if let (Some(server), Some(streams)) = (audio_server, state.streams.clone()) {
                if let Some(base) = &stream_base {
                    state.api.set_stream_base(base.clone());
                }
                state.stream_base = stream_base;
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = server.serve(streams).await {
                        tracing::error!("audio server stopped: {e}");
                    }
                });
            }
            let network = network::NetworkState::new(settings::SettingsStore::new(
                &app.path().app_config_dir()?,
            ));
            app.manage(state);
            app.manage(network);
            // Restoring may start xray; the window does not wait for it.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                handle
                    .state::<network::NetworkState>()
                    .restore(&state)
                    .await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ytdlp_version,
            commands::ytdlp_update,
            commands::stream_prefetch,
            commands::stream_base,
            network::net_settings,
            network::net_apply,
            network::net_validate,
            network::net_health,
            network::net_xray_logs,
        ])
        .register_asynchronous_uri_scheme_protocol(SCHEME, |ctx, request, responder| {
            let state = ctx
                .app_handle()
                .try_state::<AppState>()
                .map(|s| s.inner().clone());
            // Handlers do network I/O; never block the webview thread.
            tauri::async_runtime::spawn(async move {
                let response = match state {
                    Some(state) => protocol::handle(&state, request).await,
                    None => protocol::json_error(
                        http::StatusCode::SERVICE_UNAVAILABLE,
                        "app is still starting",
                    ),
                };
                responder.respond(response);
            });
        })
        .build(tauri::generate_context!())
        .expect("error while building Volna");

    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event
            && let Some(state) = handle.try_state::<AppState>()
        {
            // Stops xray if a vless link was active.
            tauri::async_runtime::block_on(state.net.shutdown());
        }
    });
}

/// Publishes the audio server URL to the page as `window.__VOLNA__.streamBase`
/// (`http://127.0.0.1:<port>/<token>/stream`; append `/<videoId>[?q=low]`).
/// `<audio>` must use it: WebKitGTK cannot play media from `volna://`.
fn stream_base_plugin(stream_base: Option<&str>) -> tauri::plugin::TauriPlugin<tauri::Wry> {
    let value = serde_json::to_string(&stream_base).expect("a string serialises");
    let script = format!(
        "window.__VOLNA__ = Object.assign(window.__VOLNA__ || {{}}, {{ streamBase: {value} }});"
    );
    tauri::plugin::Builder::new("volna-stream")
        .js_init_script(script)
        .build()
}
