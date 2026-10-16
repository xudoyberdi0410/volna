//! Exposes the target triple so the crate can find a Tauri sidecar named
//! `yt-dlp-<target-triple>` next to the executable.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=VOLNA_STREAMS_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
