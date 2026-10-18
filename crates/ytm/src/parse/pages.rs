//! Album, artist and playlist `browse` pages.

use std::collections::HashMap;

use serde_json::Value;

use super::items::{parse_section, parse_tracks, strip_vl};
use super::json::{
    arr_at, at, browse_endpoint, has_explicit_badge, str_at, string_at, text_at, thumbnails,
};
use super::runs::{Label, leading_count, parse_runs};
use crate::models::{
    Album, AlbumKind, AlbumRef, Artist, ArtistRef, Item, Playlist, Track, TrackKind, TrackPage,
};

const TWO_COL: &str = "/contents/twoColumnBrowseResultsRenderer";

fn responsive_header(resp: &Value) -> Option<&Value> {
    at(
        resp,
        &format!("{TWO_COL}/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer"),
    )
    .or_else(|| at(resp, "/header/musicDetailHeaderRenderer"))
    .or_else(|| at(resp, "/header/musicResponsiveHeaderRenderer"))
}

fn secondary_sections(resp: &Value) -> &[Value] {
    arr_at(
        resp,
        &format!("{TWO_COL}/secondaryContents/sectionListRenderer/contents"),
    )
}

fn description(header: &Value) -> Option<String> {
    text_at(
        header,
        "/description/musicDescriptionShelfRenderer/description",
    )
    .or_else(|| text_at(header, "/description"))
}

fn header_play_playlist_id(header: &Value) -> Option<String> {
    arr_at(header, "/buttons").iter().find_map(|b| {
        let ep = at(b, "/musicPlayButtonRenderer/playNavigationEndpoint")?;
        string_at(ep, "/watchPlaylistEndpoint/playlistId")
            .or_else(|| string_at(ep, "/watchEndpoint/playlistId"))
    })
}

fn canonical_list_id(resp: &Value) -> Option<String> {
    let url = str_at(resp, "/microformat/microformatDataRenderer/urlCanonical")?;
    let (_, q) = url.split_once("list=")?;
    Some(q.split('&').next()?.to_owned()).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------

pub fn parse_album(resp: &Value, browse_id: &str) -> Option<Album> {
    let header = responsive_header(resp)?;
    let title = text_at(header, "/title")?;
    let sub = parse_runs(arr_at(header, "/subtitle/runs"));
    let mut artists = parse_runs(arr_at(header, "/straplineTextOne/runs")).artists;
    if artists.is_empty() {
        artists = sub.artists.clone();
    }
    let second = parse_runs(arr_at(header, "/secondSubtitle/runs"));
    let thumbs = thumbnails(header);

    let mut tracks = Vec::new();
    let mut related = Vec::new();
    for section in secondary_sections(resp) {
        if let Some(shelf) = section.get("musicShelfRenderer") {
            tracks.extend(parse_tracks(arr_at(shelf, "/contents")));
        } else if let Some(s) = parse_section(section) {
            related.extend(s.items.into_iter().filter_map(|i| match i {
                Item::Album(a) => Some(a),
                _ => None,
            }));
        }
    }

    let playlist_id = header_play_playlist_id(header)
        .or_else(|| canonical_list_id(resp))
        .or_else(|| {
            secondary_sections(resp).iter().find_map(|s| {
                string_at(
                    s,
                    "/musicShelfRenderer/contents/0/musicResponsiveListItemRenderer/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint/playlistId",
                )
            })
        });

    let album_ref = AlbumRef {
        id: Some(browse_id.to_owned()),
        name: title.clone(),
    };
    for t in &mut tracks {
        // Some albums (Daft Punk's "Random Access Memories") link every row
        // to the official video instead of the Art Track. The row is still
        // a song of the album; only user uploads stay videos.
        if t.kind == TrackKind::OfficialVideo {
            t.kind = TrackKind::Song;
        }
        t.album.get_or_insert_with(|| album_ref.clone());
        if t.thumbnails.is_empty() {
            t.thumbnails = thumbs.clone();
        }
        if t.artists.is_empty() {
            t.artists = artists.clone();
        }
        t.year = t.year.or(sub.year);
    }

    Some(Album {
        browse_id: browse_id.to_owned(),
        playlist_id,
        title,
        kind: sub
            .label
            .and_then(Label::album_kind)
            .unwrap_or(AlbumKind::Album),
        artists,
        year: sub.year,
        description: description(header),
        explicit: has_explicit_badge(header),
        thumbnails: thumbs,
        track_count: second.stat_with(&["song", "track"]).and_then(leading_count),
        duration_s: tracks.iter().filter_map(|t| t.duration_s).sum(),
        tracks,
        related,
    })
}

/// Album browse id behind an `OLAK5uy_…` playlist, from its `VL…` page:
/// every row links the album, the most common link wins.
pub fn album_id_from_playlist_page(resp: &Value) -> Option<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for t in playlist_tracks(resp).0 {
        if let Some(AlbumRef { id: Some(id), .. }) = t.album {
            *counts.entry(id).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(id, _)| id)
}

// ---------------------------------------------------------------------------

pub fn parse_artist(resp: &Value, channel_id: &str) -> Option<Artist> {
    let header = at(resp, "/header/musicImmersiveHeaderRenderer")
        .or_else(|| at(resp, "/header/musicVisualHeaderRenderer"))
        .or_else(|| at(resp, "/header/musicResponsiveHeaderRenderer"))?;
    let name = text_at(header, "/title")?;
    let mut artist = Artist {
        id: channel_id.to_owned(),
        name,
        description: text_at(header, "/description"),
        thumbnails: thumbnails(header),
        subscribers: text_at(
            header,
            "/subscriptionButton/subscribeButtonRenderer/longSubscriberCountText",
        ),
        shuffle_playlist_id: string_at(header, "/playButton/buttonRenderer/navigationEndpoint/watchEndpoint/playlistId"),
        radio_playlist_id: string_at(
            header,
            "/startRadioButton/buttonRenderer/navigationEndpoint/watchEndpoint/playlistId",
        )
        .or_else(|| {
            string_at(
                header,
                "/startRadioButton/buttonRenderer/navigationEndpoint/watchPlaylistEndpoint/playlistId",
            )
        }),
        top_songs: Vec::new(),
        top_songs_playlist_id: None,
        albums: Vec::new(),
        singles: Vec::new(),
        videos: Vec::new(),
        playlists: Vec::new(),
        related: Vec::new(),
    };

    let sections = arr_at(
        resp,
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents",
    );
    for section in sections {
        if let Some(shelf) = section.get("musicShelfRenderer") {
            artist
                .top_songs
                .extend(parse_tracks(arr_at(shelf, "/contents")));
            if let Some((id, _)) = at(shelf, "/title/runs/0").and_then(browse_endpoint) {
                artist.top_songs_playlist_id = Some(strip_vl(id).to_owned());
            }
            continue;
        }
        if let Some(d) = section.get("musicDescriptionShelfRenderer") {
            artist.description = artist
                .description
                .take()
                .or_else(|| text_at(d, "/description"));
            continue;
        }
        let Some(s) = parse_section(section) else {
            continue;
        };
        // Shelf titles are English because the client pins hl=en; item
        // types decide everything else.
        let is_singles = s.title.to_lowercase().contains("single");
        for item in s.items {
            match item {
                Item::Album(a) if is_singles => artist.singles.push(a),
                Item::Album(a) => artist.albums.push(a),
                Item::Track(t) => artist.videos.push(t),
                Item::Playlist(p) => artist.playlists.push(p),
                Item::Artist(a) => artist.related.push(a),
            }
        }
    }

    let me = ArtistRef {
        id: Some(artist.id.clone()),
        name: artist.name.clone(),
    };
    for a in artist.albums.iter_mut().chain(artist.singles.iter_mut()) {
        if a.artists.is_empty() {
            a.artists.push(me.clone());
        }
    }
    Some(artist)
}

// ---------------------------------------------------------------------------

fn playlist_shelf(resp: &Value) -> Option<&Value> {
    secondary_sections(resp)
        .iter()
        .find_map(|s| s.get("musicPlaylistShelfRenderer").or_else(|| s.get("musicShelfRenderer")))
        .or_else(|| {
            at(
                resp,
                "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicPlaylistShelfRenderer",
            )
        })
}

/// Continuation token at the end of an item list.
pub fn trailing_continuation(items: &[Value]) -> Option<String> {
    let last = items.last()?.get("continuationItemRenderer")?;
    string_at(last, "/continuationEndpoint/continuationCommand/token").or_else(|| {
        arr_at(
            last,
            "/continuationEndpoint/commandExecutorCommand/commands",
        )
        .iter()
        .find_map(|c| string_at(c, "/continuationCommand/token"))
    })
}

fn playlist_tracks(resp: &Value) -> (Vec<Track>, Option<String>) {
    let Some(shelf) = playlist_shelf(resp) else {
        return (Vec::new(), None);
    };
    let contents = arr_at(shelf, "/contents");
    let cont = trailing_continuation(contents)
        .or_else(|| string_at(shelf, "/continuations/0/nextContinuationData/continuation"));
    (parse_tracks(contents), cont)
}

pub fn parse_playlist(resp: &Value, playlist_id: &str) -> Option<Playlist> {
    let header = responsive_header(resp).or_else(|| {
        at(
            resp,
            "/header/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer",
        )
    });
    let (tracks, continuation) = playlist_tracks(resp);
    // Album playlists (`OLAK5uy_…`) come without a header; name them after
    // the album their rows link to.
    let title = header
        .and_then(|h| text_at(h, "/title"))
        .or_else(|| tracks.first()?.album.as_ref().map(|a| a.name.clone()))?;
    let empty = Value::Null;
    let header = header.unwrap_or(&empty);
    let sub = parse_runs(arr_at(header, "/subtitle/runs"));
    let second = parse_runs(arr_at(header, "/secondSubtitle/runs"));

    let facepile = at(header, "/facepile/avatarStackViewModel");
    let author = facepile
        .and_then(|f| str_at(f, "/text/content"))
        .map(|name| ArtistRef {
            id: facepile.and_then(|f| {
                string_at(
                    f,
                    "/rendererContext/commandContext/onTap/innertubeCommand/browseEndpoint/browseId",
                )
            }),
            name: name.to_owned(),
        })
        .or_else(|| {
            parse_runs(arr_at(header, "/straplineTextOne/runs"))
                .artists
                .into_iter()
                .next()
        });

    Some(Playlist {
        id: strip_vl(playlist_id).to_owned(),
        title,
        author,
        description: description(header),
        thumbnails: thumbnails(header),
        year: sub.year,
        track_count: second
            .stat_with(&["song", "track", "video", "episode"])
            .and_then(leading_count),
        tracks,
        continuation,
    })
}

/// A browse continuation of a playlist (new `appendContinuationItemsAction`
/// shape, with the legacy `musicPlaylistShelfContinuation` as fallback).
pub fn parse_playlist_continuation(resp: &Value) -> TrackPage {
    let items = arr_at(
        resp,
        "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems",
    );
    if !items.is_empty() {
        return TrackPage {
            tracks: parse_tracks(items),
            continuation: trailing_continuation(items),
        };
    }
    let shelf = at(resp, "/continuationContents/musicPlaylistShelfContinuation");
    TrackPage {
        tracks: shelf
            .map(|s| parse_tracks(arr_at(s, "/contents")))
            .unwrap_or_default(),
        continuation: shelf.and_then(|s| {
            trailing_continuation(arr_at(s, "/contents"))
                .or_else(|| string_at(s, "/continuations/0/nextContinuationData/continuation"))
        }),
    }
}
