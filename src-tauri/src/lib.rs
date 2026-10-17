//! Volna desktop app: Tauri shell that wires the `volna://` protocol to the
//! workspace crates (`ytm`, `net`, `streams`, `wave`).

mod api_emu;
mod commands;
mod media;
mod network;
mod player;
mod protocol;
mod settings;
mod state;
mod wave;
mod webview_memory;
mod yandex;

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

    let mut builder = tauri::Builder::default();
    // First: a second launch only shows the running instance's window.
    // VOLNA_MULTI_INSTANCE=1 lets dev and test copies run next to it.
    if std::env::var_os("VOLNA_MULTI_INSTANCE").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
            player::window::show_or_create(app)
        }));
    }
    let app = builder
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
            let wave = wave::WaveState::open(&app.path().app_data_dir()?, state.api.ytm())?;
            let player = player::setup(app, &state, &wave)?;
            app.manage(player);
            app.manage(state);
            app.manage(network);
            app.manage(wave);
            app.manage(yandex::YandexState::default());
            webview_memory::apply(app.handle());
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
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                player::window::on_close_requested(window, api)
            }
            tauri::WindowEvent::Destroyed if window.label() == "main" => {
                player::window::on_destroyed(window.app_handle())
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::ytdlp_version,
            commands::ytdlp_update,
            commands::stream_prefetch,
            commands::stream_base,
            commands::stream_failure,
            network::net_settings,
            network::net_apply,
            network::net_validate,
            network::net_health,
            network::net_audio_check,
            network::net_set_cookies,
            network::net_xray_logs,
            wave::wave_status,
            wave::wave_set_settings,
            wave::wave_activities,
            wave::wave_artist_search,
            wave::wave_related_artists,
            wave::wave_cold_start,
            wave::wave_start,
            wave::wave_next,
            wave::wave_requeue,
            wave::wave_stop,
            wave::wave_event,
            wave::wave_sync_likes,
            yandex::yandex_export,
            player::commands::player_snapshot,
            player::commands::player_play,
            player::commands::player_pause,
            player::commands::player_toggle,
            player::commands::player_stop,
            player::commands::player_next,
            player::commands::player_prev,
            player::commands::player_seek,
            player::commands::player_play_uid,
            player::commands::player_set_queue,
            player::commands::player_enqueue,
            player::commands::player_remove,
            player::commands::player_move,
            player::commands::player_clear,
            player::commands::player_set_shuffle,
            player::commands::player_set_repeat,
            player::commands::player_set_settings,
            player::commands::player_set_dsp,
            player::commands::player_set_sleep_timer,
            player::commands::player_set_continuation,
            player::commands::player_wave_start,
            player::commands::player_wave_stop,
            player::commands::player_wave_dislike,
            player::commands::player_wave_requeue,
            player::commands::player_visualizer_start,
            player::commands::player_visualizer_stop,
            player::commands::player_journal,
            player::commands::player_journal_ack,
            player::commands::app_ready_to_close,
            player::commands::app_quit,
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
        // Closing the window keeps Volna in the tray; only quit exits.
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = &event
            && !player::window::quitting()
        {
            api.prevent_exit();
        }
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
