//! Exposes the target triple so the crate can find a Tauri sidecar named
//! `xray-<target-triple>` next to the executable.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets TARGET for build scripts");
    println!("cargo:rustc-env=VOLNA_NET_TARGET={target}");
    println!("cargo:rerun-if-changed=build.rs");
}
