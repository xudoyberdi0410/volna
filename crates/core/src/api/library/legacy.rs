//! The webview library (Monochrome IndexedDB + localStorage) → library rows.
//!
//! The old UI (`ui/js/legacy-export.js`, shipped last in pkgrel 9) sends
//! `{format: "volna-legacy-library", version: 1, exportedAt, stores: {<store>: …},
//! localStorage: {<key>: <string>}}`; Monochrome's own "Backup & Restore"
//! file (the stores at the top level) is accepted too.
//!
//! The stores hold TIDAL-shaped objects made by the removed `api_emu` (`shape.rs`), whose
//! ids are the YouTube Music ids themselves:
//! - track / video `id` = `videoId`; `album.id` = `MPREb_…` (or null);
//! - album `id` = browse id `MPREb_…`;
//! - artist `id` = channel id `UC…` (`MPLA` already stripped);
//! - playlist `uuid` = playlist id without `VL`; mixes are `RD…` playlist ids;
//! - covers are `volna://img/?u=<https source>&s=<px>` (Windows:
//!   `http://volna.localhost/img/?u=…`), mapped back to the `https://` source.
//!
//! So the mapping only inverts the shape, no lookups. Rows without a usable
//! id (local files, trackers, podcasts) are skipped and counted.

use serde::Serialize;
use serde_json::{Map, Value};
use volna_ytm::{
    AlbumKind, AlbumRef, AlbumSummary, ArtistRef, ArtistSummary, Item, PlaylistSummary, Thumbnail,
    Track, TrackKind,
};

use super::store::{Pin, PinKind};

pub const FORMAT: &str = "volna-legacy-library";

/// localStorage keys carried over: the scrobblers' settings and sessions.
/// Kept verbatim in `legacy_kv` for `scrobble_import_legacy`.
pub const LOCAL_STORAGE_PREFIXES: &[&str] = &["lastfm-", "librefm-", "listenbrainz-", "maloja-"];

/// A user playlist from `user_playlists`.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyPlaylist {
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub tracks: Vec<(Track, i64)>,
}

/// Everything a dump maps to.
#[derive(Debug, Default)]
pub struct Mapped {
    pub likes: Vec<(Item, i64)>,
    pub playlists: Vec<LegacyPlaylist>,
    pub history: Vec<(Track, i64)>,
    pub pins: Vec<Pin>,
    /// `legacy_kv` rows.
    pub kv: Vec<(String, String)>,
    pub report: Report,
}

/// What `legacy_import` answers.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub tracks: usize,
    pub albums: usize,
    pub artists: usize,
    pub playlists: usize,
    pub user_playlists: usize,
    pub playlist_tracks: usize,
    pub history: usize,
    pub pinned: usize,
    pub legacy_keys: usize,
    /// Rows that had no YouTube Music id.
    pub skipped: usize,
}

// ---- small JSON readers ------------------------------------------------------

fn text(v: &Value, key: &str) -> Option<String> {
    match v.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_owned()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn int(v: &Value, key: &str) -> Option<i64> {
    match v.get(key)? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<f64>().ok().map(|f| f as i64),
        _ => None,
    }
    .filter(|n| *n > 0)
}

fn flag(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A YouTube video id: 11 chars of `[A-Za-z0-9_-]`.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The `https://` source of a cover the UI held.
pub fn cover_source(cover: &str) -> Option<String> {
    let cover = cover.trim();
    if cover.starts_with("https://") {
        return Some(cover.to_owned());
    }
    if !(cover.starts_with("volna://") || cover.starts_with("http://volna.localhost/")) {
        return None;
    }
    let url = url::Url::parse(cover).ok()?;
    url.query_pairs()
        .find(|(k, _)| k == "u")
        .map(|(_, v)| v.into_owned())
        .filter(|u| u.starts_with("https://"))
}

/// A cover field: a URL string, or `{url}` / `{LARGE: {url}}` objects.
fn cover_of(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => cover_source(s),
        Value::Object(o) => o
            .get("url")
            .and_then(|u| cover_of(Some(u)))
            .or_else(|| o.values().find_map(|x| cover_of(Some(x)))),
        Value::Array(a) => a.iter().find_map(|x| cover_of(Some(x))),
        _ => None,
    }
}

fn thumbs(url: Option<String>) -> Vec<Thumbnail> {
    url.map(|url| {
        // `volna://img/?u=…&s=544` lost the size; lh3 URLs carry their own.
        Thumbnail {
            url,
            width: 0,
            height: 0,
        }
    })
    .into_iter()
    .collect()
}

fn year(v: &Value) -> Option<u16> {
    ["releaseDate", "streamStartDate"]
        .iter()
        .filter_map(|k| text(v, k))
        .find_map(|d| d.get(..4)?.parse::<u16>().ok())
        .filter(|y| *y > 1000)
}

fn artist_ref(v: &Value) -> Option<ArtistRef> {
    let name = text(v, "name")?;
    if name == "Unknown artist" {
        return None;
    }
    Some(ArtistRef {
        id: text(v, "id").map(|id| id.strip_prefix("MPLA").unwrap_or(&id).to_owned()),
        name,
    })
}

fn artists(v: &Value) -> Vec<ArtistRef> {
    let list: Vec<ArtistRef> = v
        .get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(artist_ref).collect())
        .unwrap_or_default();
    if !list.is_empty() {
        return list;
    }
    v.get("artist").and_then(artist_ref).into_iter().collect()
}

// ---- per store -------------------------------------------------------------

/// A track or video (`favorites_tracks`, `favorites_videos`, history and
/// playlist rows).
pub fn track(v: &Value, video: bool) -> Option<Track> {
    let id = text(v, "id")?;
    if !is_video_id(&id) {
        return None;
    }
    let album_v = v.get("album").filter(|a| a.is_object());
    let album = album_v.and_then(|a| {
        let id = text(a, "id");
        let name = text(a, "title").unwrap_or_default();
        // Videos get a placeholder `{title: "Video"}` album.
        if id.is_none() && (name.is_empty() || name == "Video") {
            return None;
        }
        Some(AlbumRef { id, name })
    });
    let cover = album_v
        .and_then(|a| cover_of(a.get("cover")))
        .or_else(|| cover_of(v.get("image")))
        .or_else(|| cover_of(v.get("cover")));
    let video = video || v.get("type").and_then(Value::as_str) == Some("video");
    Some(Track {
        video_id: id,
        title: text(v, "title").unwrap_or_default(),
        artists: artists(v),
        album,
        duration_s: int(v, "duration").map(|d| d as u32),
        thumbnails: thumbs(cover),
        explicit: flag(v, "explicit"),
        kind: if video {
            TrackKind::Video
        } else {
            TrackKind::Song
        },
        year: year(v).or_else(|| album_v.and_then(year)),
        views: None,
        playable: true,
    })
}

pub fn album(v: &Value) -> Option<AlbumSummary> {
    let id = text(v, "id")?;
    Some(AlbumSummary {
        browse_id: id,
        playlist_id: None,
        title: text(v, "title").unwrap_or_default(),
        kind: match v.get("type").and_then(Value::as_str) {
            Some("SINGLE") => AlbumKind::Single,
            Some("EP") => AlbumKind::Ep,
            _ => AlbumKind::Album,
        },
        artists: artists(v),
        year: year(v),
        thumbnails: thumbs(cover_of(v.get("cover"))),
        explicit: flag(v, "explicit"),
    })
}

pub fn artist(v: &Value) -> Option<ArtistSummary> {
    let id = text(v, "id")?;
    Some(ArtistSummary {
        id: id.strip_prefix("MPLA").unwrap_or(&id).to_owned(),
        name: text(v, "name").unwrap_or_default(),
        thumbnails: thumbs(cover_of(v.get("picture")).or_else(|| cover_of(v.get("image")))),
        audience: None,
    })
}

pub fn playlist(v: &Value) -> Option<PlaylistSummary> {
    let id = text(v, "uuid").or_else(|| text(v, "id"))?;
    Some(PlaylistSummary {
        id: id.strip_prefix("VL").unwrap_or(&id).to_owned(),
        title: text(v, "title")
            .or_else(|| text(v, "name"))
            .unwrap_or_default(),
        author: v
            .get("user")
            .or_else(|| v.get("creator"))
            .and_then(artist_ref),
        thumbnails: thumbs(
            cover_of(v.get("squareImage"))
                .or_else(|| cover_of(v.get("image")))
                .or_else(|| cover_of(v.get("cover"))),
        ),
        track_count: int(v, "numberOfTracks").map(|n| n as u32),
        subtitle: None,
    })
}

/// A liked YouTube Music mix (`RD…`) is a playlist.
pub fn mix(v: &Value) -> Option<PlaylistSummary> {
    let id = text(v, "id")?;
    Some(PlaylistSummary {
        id,
        title: text(v, "title").unwrap_or_default(),
        author: None,
        thumbnails: thumbs(cover_of(v.get("cover")).or_else(|| cover_of(v.get("images")))),
        track_count: None,
        subtitle: text(v, "subTitle").map(|s| unescape_html(&s)),
    })
}

/// `shape::escape_html` undone (mix subtitles).
fn unescape_html(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn pin(v: &Value, now: i64) -> Option<Pin> {
    let kind = match v.get("type").and_then(Value::as_str)? {
        "album" => PinKind::Album,
        "artist" => PinKind::Artist,
        "playlist" => PinKind::Playlist,
        "user-playlist" => PinKind::UserPlaylist,
        _ => return None,
    };
    let id = text(v, "id")?;
    let id = match kind {
        PinKind::Artist => id.strip_prefix("MPLA").unwrap_or(&id).to_owned(),
        PinKind::Playlist => id.strip_prefix("VL").unwrap_or(&id).to_owned(),
        _ => id,
    };
    Some(Pin {
        kind,
        id,
        title: text(v, "name")
            .or_else(|| text(v, "title"))
            .unwrap_or_default(),
        thumbnail: cover_of(v.get("cover")).or_else(|| cover_of(v.get("images"))),
        pinned_at: int(v, "pinnedAt").unwrap_or(now),
    })
}

fn user_playlist(v: &Value, now: i64, report: &mut Report) -> Option<LegacyPlaylist> {
    let id = text(v, "id")?;
    let created_at = int(v, "createdAt").unwrap_or(now);
    let tracks = v
        .get("tracks")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|t| {
                    let got =
                        track(t, false).map(|tr| (tr, int(t, "addedAt").unwrap_or(created_at)));
                    if got.is_none() {
                        report.skipped += 1;
                    }
                    got
                })
                .collect()
        })
        .unwrap_or_default();
    // A custom cover may be any URL (or a data: URL); keep what is usable.
    let cover = text(v, "cover")
        .and_then(|c| cover_source(&c).or_else(|| (c.starts_with("data:image/")).then_some(c)));
    Some(LegacyPlaylist {
        id,
        name: text(v, "name")
            .or_else(|| text(v, "title"))
            .unwrap_or_else(|| "Playlist".into()),
        description: text(v, "description").unwrap_or_default(),
        cover,
        created_at,
        updated_at: int(v, "updatedAt").unwrap_or(created_at),
        tracks,
    })
}

/// Rows of a store: an array, or an object of rows (older exports).
fn rows<'a>(stores: &'a Map<String, Value>, name: &str) -> Vec<&'a Value> {
    match stores.get(name) {
        Some(Value::Array(a)) => a.iter().collect(),
        Some(Value::Object(o)) => o.values().collect(),
        _ => vec![],
    }
}

/// Maps a dump. `now` (unix ms) stands in for missing timestamps.
pub fn map(dump: &Value, now: i64) -> Result<Mapped, String> {
    let root = dump
        .as_object()
        .ok_or("bad_args: the library dump is not a JSON object")?;
    if let Some(f) = root.get("format").and_then(Value::as_str)
        && f != FORMAT
    {
        return Err(format!("bad_args: unknown dump format {f}"));
    }
    let stores = match root.get("stores") {
        Some(Value::Object(s)) => s,
        _ => root,
    };
    let known = [
        "favorites_tracks",
        "favorites_videos",
        "favorites_albums",
        "favorites_artists",
        "favorites_playlists",
        "favorites_mixes",
        "history_tracks",
        "user_playlists",
    ];
    if !known.iter().any(|k| stores.contains_key(*k)) && !root.contains_key("localStorage") {
        return Err("bad_args: this file is not a Volna/Monochrome library export".into());
    }

    let mut m = Mapped::default();
    let added = |v: &Value| int(v, "addedAt").unwrap_or(now);

    macro_rules! likes {
        ($store:expr, $f:expr, $count:ident) => {
            for v in rows(stores, $store) {
                match $f(v) {
                    Some(item) => {
                        m.likes.push((item, added(v)));
                        m.report.$count += 1;
                    }
                    None => m.report.skipped += 1,
                }
            }
        };
    }
    likes!(
        "favorites_tracks",
        |v| track(v, false).map(Item::Track),
        tracks
    );
    likes!(
        "favorites_videos",
        |v| track(v, true).map(Item::Track),
        tracks
    );
    likes!("favorites_albums", |v| album(v).map(Item::Album), albums);
    likes!(
        "favorites_artists",
        |v| artist(v).map(Item::Artist),
        artists
    );
    likes!(
        "favorites_playlists",
        |v| playlist(v).map(Item::Playlist),
        playlists
    );
    likes!("favorites_mixes", |v| mix(v).map(Item::Playlist), playlists);

    for v in rows(stores, "history_tracks") {
        match track(v, false) {
            Some(t) => {
                m.history.push((t, int(v, "timestamp").unwrap_or(now)));
                m.report.history += 1;
            }
            None => m.report.skipped += 1,
        }
    }
    for v in rows(stores, "user_playlists") {
        match user_playlist(v, now, &mut m.report) {
            Some(p) => {
                m.report.user_playlists += 1;
                m.report.playlist_tracks += p.tracks.len();
                m.playlists.push(p);
            }
            None => m.report.skipped += 1,
        }
    }
    for v in rows(stores, "pinned_items") {
        match pin(v, now) {
            Some(p) => {
                m.pins.push(p);
                m.report.pinned += 1;
            }
            None => m.report.skipped += 1,
        }
    }

    // Unported parts stay as they were, for whoever needs them later.
    for (store, key) in [
        ("settings", "idb:settings"),
        ("user_folders", "idb:user_folders"),
    ] {
        if let Some(v) = stores.get(store).filter(|v| !v.is_null()) {
            m.kv.push((key.to_owned(), v.to_string()));
        }
    }
    if let Some(Value::Object(ls)) = root.get("localStorage") {
        for (k, v) in ls {
            if !LOCAL_STORAGE_PREFIXES.iter().any(|p| k.starts_with(p)) {
                continue;
            }
            let raw = match v {
                Value::String(s) => s.clone(),
                Value::Null => continue,
                other => other.to_string(),
            };
            m.kv.push((k.clone(), raw));
            m.report.legacy_keys += 1;
        }
    }
    Ok(m)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// What `ui/js/legacy-export.js` sends, trimmed to the shapes `db.js`
    /// writes (`_minifyItem`, `_minifyPinnedItem`, `createPlaylist`).
    pub fn sample() -> Value {
        json!({
            "format": "volna-legacy-library",
            "version": 1,
            "exportedAt": 1_760_000_000_000_i64,
            "stores": {
                "favorites_tracks": [{
                    "id": "dQw4w9WgXcQ",
                    "addedAt": 1_700_000_000_000_i64,
                    "title": "Never Gonna Give You Up",
                    "duration": 213,
                    "explicit": false,
                    "artist": {"id": "UCuAXFkgsw1L7xaCfnd5JJOw", "name": "Rick Astley", "type": "MAIN", "picture": null},
                    "artists": [{"id": "UCuAXFkgsw1L7xaCfnd5JJOw", "name": "Rick Astley"}],
                    "album": {
                        "id": "MPREb_abc",
                        "title": "Whenever You Need Somebody",
                        "cover": "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fx%3Dw544-h544&s=544",
                        "releaseDate": "1987-01-01"
                    },
                    "isrc": null,
                    "mixes": {"TRACK_MIX": "RDAMVMdQw4w9WgXcQ"},
                    "mediaMetadata": {"tags": []}
                }, {
                    "id": "local-123", "title": "A local file", "addedAt": 1
                }],
                "favorites_videos": [{
                    "id": "9bZkp7q19f0",
                    "type": "video",
                    "title": "Gangnam Style",
                    "duration": 252,
                    "image": "http://volna.localhost/img/?u=https%3A%2F%2Fi.ytimg.com%2Fvi%2F9bZkp7q19f0%2Fhq.jpg&s=544",
                    "artists": [{"id": null, "name": "Unknown artist"}],
                    "album": {"title": "Video", "cover": null},
                    "addedAt": 1_700_000_000_500_i64
                }],
                "favorites_albums": [{
                    "id": "MPREb_abc",
                    "addedAt": 1_700_000_001_000_i64,
                    "title": "Whenever You Need Somebody",
                    "cover": "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fx%3Dw544-h544&s=544",
                    "releaseDate": "1987-01-01",
                    "explicit": false,
                    "artist": {"name": "Rick Astley", "id": "UCuAXFkgsw1L7xaCfnd5JJOw"},
                    "type": "EP",
                    "numberOfTracks": 10
                }],
                "favorites_artists": [{
                    "id": "UCuAXFkgsw1L7xaCfnd5JJOw",
                    "addedAt": 1_700_000_002_000_i64,
                    "name": "Rick Astley",
                    "picture": "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fa&s=226"
                }],
                "favorites_playlists": [{
                    "uuid": "PLx",
                    "addedAt": 1_700_000_003_000_i64,
                    "title": "Eighties",
                    "image": null,
                    "numberOfTracks": 50,
                    "user": {"name": "someone"}
                }],
                "favorites_mixes": [{
                    "id": "RDCLAK5uy_mix",
                    "addedAt": 1_700_000_004_000_i64,
                    "title": "Mix",
                    "subTitle": "A &amp; B",
                    "mixType": "DISCOVERY_MIX",
                    "cover": {"LARGE": {"url": "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fm&s=544"}}
                }],
                "history_tracks": [{
                    "id": "dQw4w9WgXcQ", "title": "Never Gonna Give You Up",
                    "artists": [{"id": "UCuAXFkgsw1L7xaCfnd5JJOw", "name": "Rick Astley"}],
                    "album": {"id": null, "title": "", "cover": null},
                    "duration": 213, "timestamp": 1_700_000_100_000_i64
                }, {
                    "id": "9bZkp7q19f0", "type": "video", "title": "Gangnam Style",
                    "timestamp": 1_700_000_200_000_i64
                }],
                "user_playlists": [{
                    "id": "8c1f8e1e-0000-4000-8000-000000000001",
                    "name": "Road",
                    "description": "for driving",
                    "cover": "",
                    "createdAt": 1_690_000_000_000_i64,
                    "updatedAt": 1_695_000_000_000_i64,
                    "numberOfTracks": 2,
                    "images": [],
                    "tracks": [
                        {"id": "9bZkp7q19f0", "type": "video", "title": "Gangnam Style", "addedAt": 1_690_000_000_001_i64},
                        {"id": "dQw4w9WgXcQ", "title": "Never Gonna Give You Up", "addedAt": 1_690_000_000_002_i64},
                        {"id": "podcast_1", "title": "x"}
                    ]
                }],
                "user_folders": [{"id": "f", "name": "Folder", "playlists": ["8c1f8e1e-0000-4000-8000-000000000001"]}],
                "pinned_items": [
                    {"id": "8c1f8e1e-0000-4000-8000-000000000001", "type": "user-playlist", "name": "Road", "cover": "", "images": [], "href": "/userplaylist/8c1f", "pinnedAt": 5},
                    {"id": "UCuAXFkgsw1L7xaCfnd5JJOw", "type": "artist", "name": "Rick Astley", "cover": "volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fa&s=226", "href": "/artist/UC", "pinnedAt": 6}
                ],
                "settings": {"autoeq-cache": {"v": 1}}
            },
            "localStorage": {
                "lastfm-session": "{\"key\":\"sk\",\"name\":\"me\"}",
                "lastfm-enabled": "true",
                "lastfm-custom-api-secret": "b2xsZWg=",
                "librefm-session": "{\"key\":\"lk\",\"name\":\"me\"}",
                "listenbrainz-token": "tok",
                "maloja-custom-url": "https://maloja.example",
                "monochrome-theme": "dark"
            }
        })
    }

    #[test]
    fn maps_every_store() {
        let m = map(&sample(), 42).unwrap();
        let r = &m.report;
        assert_eq!(
            (r.tracks, r.albums, r.artists, r.playlists),
            (2, 1, 1, 2),
            "{r:?}"
        );
        assert_eq!(
            (r.history, r.user_playlists, r.playlist_tracks, r.pinned),
            (2, 1, 2, 2)
        );
        assert_eq!(r.legacy_keys, 6);
        assert_eq!(r.skipped, 2, "local file + podcast row");

        let Item::Track(t) = &m.likes[0].0 else {
            panic!()
        };
        assert_eq!(m.likes[0].1, 1_700_000_000_000);
        assert_eq!(t.video_id, "dQw4w9WgXcQ");
        assert_eq!(t.artists[0].id.as_deref(), Some("UCuAXFkgsw1L7xaCfnd5JJOw"));
        assert_eq!(t.album.as_ref().unwrap().id.as_deref(), Some("MPREb_abc"));
        assert_eq!(
            t.thumbnails[0].url,
            "https://lh3.googleusercontent.com/x=w544-h544"
        );
        assert_eq!(
            (t.duration_s, t.year, t.kind),
            (Some(213), Some(1987), TrackKind::Song)
        );

        let Item::Track(v) = &m.likes[1].0 else {
            panic!()
        };
        assert_eq!(v.kind, TrackKind::Video);
        assert!(v.artists.is_empty(), "\"Unknown artist\" is a placeholder");
        assert!(v.album.is_none(), "the Video placeholder album is dropped");
        assert_eq!(
            v.thumbnails[0].url,
            "https://i.ytimg.com/vi/9bZkp7q19f0/hq.jpg"
        );

        let Item::Album(a) = &m.likes[2].0 else {
            panic!()
        };
        assert_eq!((a.browse_id.as_str(), a.kind), ("MPREb_abc", AlbumKind::Ep));
        assert_eq!(a.artists[0].name, "Rick Astley");

        let Item::Artist(ar) = &m.likes[3].0 else {
            panic!()
        };
        assert_eq!(ar.thumbnails[0].url, "https://lh3.googleusercontent.com/a");

        let Item::Playlist(p) = &m.likes[4].0 else {
            panic!()
        };
        assert_eq!((p.id.as_str(), p.track_count), ("PLx", Some(50)));
        assert_eq!(p.author.as_ref().unwrap().name, "someone");
        let Item::Playlist(mx) = &m.likes[5].0 else {
            panic!()
        };
        assert_eq!(mx.subtitle.as_deref(), Some("A & B"));
        assert_eq!(mx.thumbnails[0].url, "https://lh3.googleusercontent.com/m");

        let pl = &m.playlists[0];
        assert_eq!(
            (pl.name.as_str(), pl.description.as_str()),
            ("Road", "for driving")
        );
        assert_eq!(pl.cover, None);
        assert_eq!(pl.tracks[0].0.kind, TrackKind::Video);
        assert_eq!(pl.tracks[1].1, 1_690_000_000_002);

        assert_eq!(m.pins[0].kind, PinKind::UserPlaylist);
        assert_eq!(m.history[1].1, 1_700_000_200_000);

        let keys: Vec<&str> = m.kv.iter().map(|(k, _)| k.as_str()).collect();
        assert!(keys.contains(&"idb:settings") && keys.contains(&"idb:user_folders"));
        assert!(keys.contains(&"lastfm-session") && !keys.contains(&"monochrome-theme"));
        let secret =
            m.kv.iter()
                .find(|(k, _)| k == "lastfm-custom-api-secret")
                .unwrap();
        assert_eq!(secret.1, "b2xsZWg=", "kept verbatim");
    }

    #[test]
    fn accepts_monochrome_backup_files() {
        // `db.exportData()`: stores at the top level, no localStorage.
        let dump = json!({
            "favorites_tracks": [{"id": "dQw4w9WgXcQ", "title": "x", "addedAt": 7}],
            "favorites_albums": [],
            "history_tracks": [],
            "user_playlists": [],
            "user_folders": []
        });
        let m = map(&dump, 1).unwrap();
        assert_eq!(m.report.tracks, 1);
        assert_eq!(m.likes[0].1, 7);
    }

    #[test]
    fn rejects_other_json() {
        assert!(map(&json!([1]), 1).is_err());
        assert!(map(&json!({"hello": 1}), 1).is_err());
        assert!(map(&json!({"format": "other", "stores": {}}), 1).is_err());
    }

    #[test]
    fn covers_and_ids() {
        assert_eq!(
            cover_source("volna://img/?u=https%3A%2F%2Fa.b%2Fc&s=80").as_deref(),
            Some("https://a.b/c")
        );
        assert_eq!(
            cover_source("https://a.b/c").as_deref(),
            Some("https://a.b/c")
        );
        assert_eq!(cover_source("blob:xyz"), None);
        assert_eq!(cover_source("volna://img/?u=http%3A%2F%2Fa"), None);
        assert!(is_video_id("dQw4w9WgXcQ"));
        assert!(!is_video_id("tracker-1234"));
    }
}
