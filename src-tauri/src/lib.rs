//! Volna desktop app: Tauri shell that wires the `volna://` protocol to the
//! workspace crates (`ytm`, `net`, `streams`, `wave`).

mod protocol;

/// Name of the custom URI scheme served to the webview.
pub const SCHEME: &str = "volna";

/// Builds and runs the Tauri application.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,volna=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol(SCHEME, |_ctx, request, responder| {
            // Handlers will do network I/O later; never block the webview thread.
            tauri::async_runtime::spawn(async move {
                responder.respond(protocol::handle(request).await);
            });
        })
        .run(tauri::generate_context!())
        .expect("error while running Volna");
}
