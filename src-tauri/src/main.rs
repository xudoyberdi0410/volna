// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! `volna-window`: the UI process, started by the `volna` daemon.

/// Rust allocations go to mimalloc (see `volna_daemon::memory`).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    volna_daemon::memory::tune_memory();
    let runtime = volna_daemon::memory::runtime();
    tauri::async_runtime::set(runtime.handle().clone());
    volna_lib::run();
}
