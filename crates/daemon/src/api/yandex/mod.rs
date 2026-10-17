//! Import from Yandex Music.

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod browser;
pub mod client;
