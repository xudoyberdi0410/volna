//! YouTube Music InnerTube client (`WEB_REMIX`).
//!
//! Domain models and the offline parsers for InnerTube responses; the HTTP
//! client follows.

pub mod models;
mod parse;

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
