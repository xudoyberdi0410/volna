//! Lyrics for Volna: the provider chain of the old UI (`@uimaxbai/am-lyrics`
//! `AmLyrics.fetchLyrics`) in Rust — lrc.red → BiniLyrics → Unison →
//! lyricsplus → LRCLIB → Genius → YouTube Music — with TTML/LRC parsing and
//! a SQLite cache.
//!
//! Every request goes through the `reqwest::Client` the caller passes (the
//! app's proxy-aware client). No desktop dependencies: this crate moves into
//! `volna-core` for mobile.

pub mod cache;
pub mod chain;
mod error;
pub mod kpoe;
pub mod lrc;
pub mod model;
pub mod providers;
pub mod rank;
pub mod service;
pub mod ttml;

pub use cache::{Cache, Entry};
pub use chain::{Fetched, Fetcher};
pub use error::{Error, Result};
pub use model::{Line, Lyrics, Synced, TrackQuery, Word};
pub use providers::Provider;
pub use service::{Key, LyricsService, SourceInfo};
