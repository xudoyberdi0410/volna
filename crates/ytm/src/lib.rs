//! YouTube Music InnerTube client (`WEB_REMIX`).
//!
//! Own implementation of the endpoints Volna needs: `search`, `browse`
//! (artist / album / playlist pages, home, moods & genres, lyrics and the
//! related tab) and `next` (track metadata, radio queues such as
//! `RDAMVM<videoId>`). All HTTP goes through a `reqwest::Client` handed in
//! by the caller (the shared client from `volna-net`), so the configured
//! proxy applies.
//!
//! The client is logged out. On first use it loads the music.youtube.com
//! shell once to read the current client version and visitor data from
//! `ytcfg`, and caches them (see [`ClientOptions::session_ttl`]). A rejected
//! request refreshes that session and is retried once.
//!
//! Parsing is defensive: renderers YouTube adds or renames are skipped,
//! never turned into errors or panics. JSON paths follow
//! [ytmusicapi](https://github.com/sigma67/ytmusicapi).
//!
//! ```no_run
//! # async fn demo() -> volna_ytm::Result<()> {
//! use volna_ytm::{SearchFilter, YtmClient};
//!
//! let ytm = YtmClient::new(reqwest::Client::new());
//! let page = ytm.search("Скриптонит", SearchFilter::Songs).await?;
//! let seed = page.items[0].as_track().unwrap();
//! let radio = ytm.radio(&seed.video_id).await?;
//! println!("{} tracks, more: {}", radio.tracks.len(), radio.continuation.is_some());
//! # Ok(()) }
//! ```

mod api;
mod client;
mod error;
pub mod models;
mod parse;
mod ytcfg;

pub use client::{ClientOptions, YtmClient};
pub use error::{Error, Result};
pub use models::*;

/// Offline parsers, exposed for tests and for callers that cache raw
/// InnerTube responses.
pub mod parsers {
    pub use crate::parse::browse::{parse_feed, parse_lyrics, parse_moods, parse_related};
    pub use crate::parse::pages::{
        album_id_from_playlist_page, parse_album, parse_artist, parse_playlist,
        parse_playlist_continuation,
    };
    pub use crate::parse::search::{parse_search, parse_search_continuation, parse_suggestions};
    pub use crate::parse::watch::{parse_watch, parse_watch_continuation};
}
