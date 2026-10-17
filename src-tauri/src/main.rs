// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Rust allocations go to mimalloc. With glibc malloc every tokio worker got
/// its own 64 MB arena that kept the peak of the JSON it once parsed: after
/// half an hour of use the arenas held ~170 MB, nearly all of it free.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Tokio workers. The work is network-bound: a few threads are plenty, and
/// each one is a stack plus per-thread allocator state.
const MAX_WORKERS: usize = 4;

fn main() {
    tune_memory();
    let workers =
        std::thread::available_parallelism().map_or(MAX_WORKERS, |n| n.get().min(MAX_WORKERS));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("cannot start the tokio runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    volna_lib::run();
}

fn tune_memory() {
    // mi_option_purge_delay (15 in mimalloc v2 and v3) = 0: hand freed pages
    // back to the OS at once. Otherwise they wait for the owning thread's
    // next allocation, which an idle worker never makes.
    const MI_OPTION_PURGE_DELAY: std::ffi::c_int = 15;
    unsafe extern "C" {
        fn mi_option_set(option: std::ffi::c_int, value: std::ffi::c_long);
    }
    // SAFETY: a setter of a global option, valid at any time.
    unsafe { mi_option_set(MI_OPTION_PURGE_DELAY, 0) };

    #[cfg(target_os = "linux")]
    {
        // With THP set to "always" (Arch, CachyOS, ...) every touched 2 MB
        // region of an allocator or of WebKit's heaps is backed in full:
        // ~35 MB more RSS for this process. A player gains nothing from it.
        const PR_SET_THP_DISABLE: std::ffi::c_int = 41;
        unsafe extern "C" {
            fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
        }
        // SAFETY: affects only this process's memory policy.
        unsafe {
            prctl(
                PR_SET_THP_DISABLE,
                1 as std::ffi::c_ulong,
                0usize,
                0usize,
                0usize,
            )
        };
    }

    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // GTK, WebKit and GStreamer still allocate through glibc; ask it to
        // return free memory now and then.
        unsafe extern "C" {
            fn malloc_trim(pad: usize) -> std::ffi::c_int;
        }
        let spawned = std::thread::Builder::new()
            .name("malloc-trim".into())
            .stack_size(64 * 1024)
            .spawn(|| {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    // SAFETY: glibc's malloc_trim is thread-safe.
                    unsafe { malloc_trim(0) };
                }
            });
        if let Err(e) = spawned {
            eprintln!("cannot start the malloc trimmer: {e}");
        }
    }
}
