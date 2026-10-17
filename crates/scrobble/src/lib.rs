//! Scrobbling for Volna: Last.fm, Libre.fm, ListenBrainz and Maloja.
//!
//! A port of `ui/js/multi-scrobbler.js` and the four service modules next to
//! it. Every service is independent: a failing one never blocks the others.
//! Finished plays and loves go to an offline queue in SQLite first and are
//! sent from there, retried with backoff and batched where the API allows
//! (Last.fm and Libre.fm up to 50, ListenBrainz `import`). "Now playing" is
//! best effort and never queued.
//!
//! No GStreamer, no sockets, no daemon types: HTTP goes through [`Http`]
//! (the daemon passes the `crates/net` client), state lives in one SQLite
//! file. The crate moves into `volna-core` for mobile as is.

pub mod audioscrobbler;
pub mod config;
pub mod error;
pub mod http;
pub mod legacy;
pub mod listenbrainz;
pub mod maloja;
pub mod queue;
mod service;
pub mod track;

pub use config::{Config, Service};
pub use error::Error;
pub use http::{Http, ReqwestHttp};
pub use service::{Scrobbler, Scrobbling, ServiceStatus, Status, should_scrobble};
pub use track::{Play, Track};

/// Services reject (or ignore) older plays, as `SCROBBLE_MAX_AGE_MS` in the UI.
pub const MAX_AGE_S: i64 = 14 * 24 * 3600;

/// Seconds since the Unix epoch.
pub fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
