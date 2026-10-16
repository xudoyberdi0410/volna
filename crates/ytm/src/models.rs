//! Domain types returned by [`crate::YtmClient`].
//!
//! These are deliberately flat and renderer-agnostic: whatever shape the
//! InnerTube JSON had, callers only ever see these structs.

use serde::{Deserialize, Serialize};

/// One image of an entity, as served by `lh3.googleusercontent.com` /
/// `i.ytimg.com`. Width/height are 0 when YouTube did not report them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// Link to an artist (or an uploader channel for user videos).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    /// Channel id (`UC…`). `None` for unlinked names such as "Various Artists".
    pub id: Option<String>,
    pub name: String,
}

/// Link to an album.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    /// Album browse id (`MPREb_…`).
    pub id: Option<String>,
    pub name: String,
}

/// What kind of video a track is backed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Audio-only "Art Track" (`MUSIC_VIDEO_TYPE_ATV`) — a proper song.
    #[default]
    Song,
    /// Official music video (`MUSIC_VIDEO_TYPE_OMV`).
    OfficialVideo,
    /// Any other YouTube video (`MUSIC_VIDEO_TYPE_UGC`, …).
    Video,
}

/// A playable item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    /// Missing in a few listings (e.g. the "all" search tab).
    pub duration_s: Option<u32>,
    pub thumbnails: Vec<Thumbnail>,
    pub explicit: bool,
    pub kind: TrackKind,
    /// Release year when the listing shows one.
    pub year: Option<u16>,
    /// Raw play/view counter as displayed ("57M plays").
    pub views: Option<String>,
    /// `false` when YouTube greys the row out (region lock, removed, …).
    pub playable: bool,
}

/// Album / single / EP flavour, read from the subtitle label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlbumKind {
    #[default]
    Album,
    Single,
    Ep,
}

/// Album as it appears in lists (search, artist page, carousels).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumSummary {
    /// `MPREb_…`
    pub browse_id: String,
    /// Audio playlist id (`OLAK5uy_…`) when the listing exposes it.
    pub playlist_id: Option<String>,
    pub title: String,
    pub kind: AlbumKind,
    pub artists: Vec<ArtistRef>,
    pub year: Option<u16>,
    pub thumbnails: Vec<Thumbnail>,
    pub explicit: bool,
}

/// Artist as it appears in lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistSummary {
    /// Channel id (`UC…`).
    pub id: String,
    pub name: String,
    pub thumbnails: Vec<Thumbnail>,
    /// Raw audience text ("7.75M monthly audience", "298K subscribers").
    pub audience: Option<String>,
}

/// Playlist as it appears in lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistSummary {
    /// Playlist id without the `VL` browse prefix (`PL…`, `RDCLAK…`, `OLAK…`).
    pub id: String,
    pub title: String,
    pub author: Option<ArtistRef>,
    pub thumbnails: Vec<Thumbnail>,
    pub track_count: Option<u32>,
    /// Remaining subtitle text that did not map to a field
    /// (e.g. "Alicia Keys, Ed Sheeran, Adele" on home-feed playlists).
    pub subtitle: Option<String>,
}

/// Anything that can show up in a mixed listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Track(Track),
    Album(AlbumSummary),
    Artist(ArtistSummary),
    Playlist(PlaylistSummary),
}

impl Item {
    pub fn as_track(&self) -> Option<&Track> {
        match self {
            Item::Track(t) => Some(t),
            _ => None,
        }
    }
    pub fn as_album(&self) -> Option<&AlbumSummary> {
        match self {
            Item::Album(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_artist(&self) -> Option<&ArtistSummary> {
        match self {
            Item::Artist(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_playlist(&self) -> Option<&PlaylistSummary> {
        match self {
            Item::Playlist(p) => Some(p),
            _ => None,
        }
    }
}

/// A titled row of items (home feed, mood pages, artist carousels).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    /// Small text above the title ("Скриптонит" for "More from …"), if any.
    pub strapline: Option<String>,
    pub items: Vec<Item>,
}

/// Which result type to search for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SearchFilter {
    /// The unfiltered "YT Music" tab: a top result plus a few of each type.
    #[default]
    All,
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
}

/// One page of search results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchPage {
    /// The highlighted card ("Top result"); only for [`SearchFilter::All`].
    pub top_result: Option<Item>,
    pub items: Vec<Item>,
    /// Token for [`crate::YtmClient::search_more`]; only filtered searches page.
    pub continuation: Option<String>,
}

/// Autocomplete for the search box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SearchSuggestions {
    /// Completed queries ("скриптонит", "скриптонит привычка", …).
    pub queries: Vec<String>,
    /// Direct entity matches shown under the queries.
    pub items: Vec<Item>,
}

/// Full album page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Album {
    pub browse_id: String,
    pub playlist_id: Option<String>,
    pub title: String,
    pub kind: AlbumKind,
    pub artists: Vec<ArtistRef>,
    pub year: Option<u16>,
    pub description: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
    pub explicit: bool,
    pub track_count: Option<u32>,
    /// Sum of track durations, seconds.
    pub duration_s: u32,
    pub tracks: Vec<Track>,
    /// "Other versions" / "More from …" carousels.
    pub related: Vec<AlbumSummary>,
}

/// Full artist page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
    /// Raw subscriber text ("622K subscribers").
    pub subscribers: Option<String>,
    /// Playlist id of the shuffle mix (`RDAO…`), if offered.
    pub shuffle_playlist_id: Option<String>,
    /// Playlist id of the artist radio (`RDEM…`), if offered.
    pub radio_playlist_id: Option<String>,
    pub top_songs: Vec<Track>,
    /// Full "Top songs" playlist (`OLAK5uy_…`), page it with
    /// [`crate::YtmClient::playlist`].
    pub top_songs_playlist_id: Option<String>,
    pub albums: Vec<AlbumSummary>,
    pub singles: Vec<AlbumSummary>,
    pub videos: Vec<Track>,
    pub playlists: Vec<PlaylistSummary>,
    pub related: Vec<ArtistSummary>,
}

/// Playlist page (first page of tracks + continuation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    /// Without `VL` prefix.
    pub id: String,
    pub title: String,
    pub author: Option<ArtistRef>,
    pub description: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
    pub year: Option<u16>,
    /// Count as displayed by YouTube (may exceed the playable tracks).
    pub track_count: Option<u32>,
    pub tracks: Vec<Track>,
    /// Token for [`crate::YtmClient::playlist_more`].
    pub continuation: Option<String>,
}

/// A page of tracks fetched through a continuation token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub continuation: Option<String>,
}

/// A watch queue: radio, album playback, "up next".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchQueue {
    /// The queue's playlist id (`RDAMVM…` for a song radio).
    pub playlist_id: Option<String>,
    pub tracks: Vec<Track>,
    /// Token for [`crate::YtmClient::queue_more`]; radios are infinite.
    pub continuation: Option<String>,
    /// Browse id of the lyrics tab (`MPLY…`), when lyrics exist.
    pub lyrics_browse_id: Option<String>,
    /// Browse id of the "Related" tab (`MPTR…`).
    pub related_browse_id: Option<String>,
}

/// Metadata of a single track plus the ids of its watch-page tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackDetails {
    pub track: Track,
    pub lyrics_browse_id: Option<String>,
    pub related_browse_id: Option<String>,
}

/// Contents of a song's "Related" tab, grouped by type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Related {
    /// "You might also like".
    pub tracks: Vec<Track>,
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumSummary>,
    pub playlists: Vec<PlaylistSummary>,
    /// "About the artist" text.
    pub artist_description: Option<String>,
}

/// Plain-text lyrics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lyrics {
    pub text: String,
    /// Provider without the "Source: " prefix ("Musixmatch", "LyricFind").
    pub source: Option<String>,
}

/// A grid of moods or genres ("Moods & moments", "Genres").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoodCategory {
    pub title: String,
    pub moods: Vec<Mood>,
}

/// One mood/genre button; open it with [`crate::YtmClient::mood_playlists`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mood {
    pub title: String,
    pub params: String,
    /// Accent colour as ARGB.
    pub color: Option<u32>,
}
