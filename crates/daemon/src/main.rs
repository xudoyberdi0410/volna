// No console window for the daemon on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `volna`: the daemon. `volna` shows the window (starting the daemon if
//! needed), `volna --tray` starts it in the tray, `volna --quit` quits a
//! running one, `volna --window` runs the window process directly.

/// Rust allocations go to mimalloc (see `volna_daemon::memory`).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--window") {
        // Compatibility: the window is its own binary (the Flutter app).
        let exe = volna_daemon::daemon::window_exe();
        let status = std::process::Command::new(exe).status();
        std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
    }
    #[cfg(windows)]
    use_bundled_gstreamer();
    volna_daemon::memory::tune_memory();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "info,volna=debug,volna_daemon=debug,volna_streams=debug".into()
            }),
        )
        .init();
    let launch = volna_daemon::daemon::Launch::from_args(args);
    let runtime = volna_daemon::memory::runtime();
    if let Err(e) = runtime.block_on(volna_daemon::daemon::run(launch)) {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

/// Windows: the installer ships GStreamer itself, its DLLs next to this
/// executable and the plugins in `gstreamer-1.0/`, instead of a system install.
#[cfg(windows)]
fn use_bundled_gstreamer() {
    let Some(plugins) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("gstreamer-1.0")))
        .filter(|d| d.is_dir())
    else {
        return;
    };
    // SAFETY: called first thing in `main`, before any other thread exists.
    unsafe {
        std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", &plugins);
        std::env::set_var("GST_PLUGIN_PATH_1_0", &plugins);
        // gst-plugin-scanner.exe is not shipped; scan in-process.
        std::env::set_var("GST_REGISTRY_FORK", "no");
    }
}
