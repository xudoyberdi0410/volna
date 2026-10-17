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
        // Compatibility: the window is its own binary.
        let exe = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join("volna-window")))
            .unwrap_or_else(|| "volna-window".into());
        let status = std::process::Command::new(exe).status();
        std::process::exit(status.ok().and_then(|s| s.code()).unwrap_or(1));
    }
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
