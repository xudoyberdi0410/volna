//! Renderer → [`Item`] conversion shared by every page.
//!
//! The item type is decided from navigation endpoints (page type of the
//! browse target, music video type of the watch target), never from
//! localized labels.

use serde_json::Value;

use super::json::{
    arr_at, at, browse_endpoint, has_explicit_badge, str_at, string_at, text, text_at, thumbnails,
};
use super::runs::{Label, RunInfo, leading_count, parse_duration, parse_runs};
use crate::models::{
    AlbumKind, AlbumSummary, ArtistRef, ArtistSummary, Item, PlaylistSummary, Section, Track,
    TrackKind,
};

const MRLIR: &str = "musicResponsiveListItemRenderer";
const MTRIR: &str = "musicTwoRowItemRenderer";
const PPVR: &str = "playlistPanelVideoRenderer";
const PPVWR: &str = "playlistPanelVideoWrapperRenderer";

/// What a browse id points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Artist,
    Album,
    Playlist,
    Other,
}

pub fn classify_browse(id: &str, page_type: Option<&str>) -> Target {
    match page_type {
        Some("MUSIC_PAGE_TYPE_ARTIST") => Target::Artist,
        Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => Target::Album,
        Some("MUSIC_PAGE_TYPE_PLAYLIST") => Target::Playlist,
        Some(_) => Target::Other,
        None if id.starts_with("MPRE") => Target::Album,
        None if id.starts_with("VL") => Target::Playlist,
        None if id.starts_with("UC") => Target::Artist,
        None => Target::Other,
    }
}

/// Maps `musicVideoType`; `None` for things we never play (podcast episodes).
pub fn track_kind(video_type: Option<&str>) -> Option<TrackKind> {
    match video_type {
        Some("MUSIC_VIDEO_TYPE_ATV") | None => Some(TrackKind::Song),
        Some("MUSIC_VIDEO_TYPE_OMV") => Some(TrackKind::OfficialVideo),
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => None,
        Some(_) => Some(TrackKind::Video),
    }
}

/// `VLPL…` → `PL…`.
pub fn strip_vl(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

/// Any supported renderer wrapper → item. Unknown renderers yield `None`.
pub fn parse_item(v: &Value) -> Option<Item> {
    if let Some(r) = v.get(MRLIR) {
        parse_list_item(r)
    } else if let Some(r) = v.get(MTRIR) {
        parse_two_row_item(r)
    } else {
        parse_panel_video(v).map(Item::Track)
    }
}

pub fn parse_items(list: &[Value]) -> Vec<Item> {
    list.iter().filter_map(parse_item).collect()
}

pub fn parse_tracks(list: &[Value]) -> Vec<Track> {
    list.iter()
        .filter_map(parse_item)
        .filter_map(|i| match i {
            Item::Track(t) => Some(t),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// musicResponsiveListItemRenderer (rows: search, album, playlist, top songs)

fn flex_column(r: &Value, i: usize) -> Option<&Value> {
    at(
        r,
        &format!("/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text"),
    )
}

/// Runs of all flex columns after the first, joined with a dummy separator.
fn subtitle_runs(r: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    for col in arr_at(r, "/flexColumns").iter().skip(1) {
        let runs = arr_at(col, "/musicResponsiveListItemFlexColumnRenderer/text/runs");
        if runs.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(serde_json::json!({"text": " • "}));
        }
        out.extend(runs.iter().cloned());
    }
    out
}

fn list_item_video(r: &Value) -> (Option<&str>, Option<&str>) {
    const PLAY: &str = "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint";
    const VT: &str = "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
    let title_watch = at(
        r,
        "/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint",
    );
    let play_watch = at(r, PLAY);
    let video_id = str_at(r, "/playlistItemData/videoId")
        .or_else(|| title_watch.and_then(|w| str_at(w, "/videoId")))
        .or_else(|| play_watch.and_then(|w| str_at(w, "/videoId")));
    let video_type = play_watch
        .and_then(|w| str_at(w, VT))
        .or_else(|| title_watch.and_then(|w| str_at(w, VT)));
    (video_id, video_type)
}

fn play_playlist_id(r: &Value) -> Option<String> {
    let ep = at(
        r,
        "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint",
    )
    .or_else(|| {
        at(
            r,
            "/thumbnailOverlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint",
        )
    })?;
    string_at(ep, "/watchPlaylistEndpoint/playlistId")
        .or_else(|| string_at(ep, "/watchEndpoint/playlistId"))
}

fn parse_list_item(r: &Value) -> Option<Item> {
    let title = flex_column(r, 0).and_then(text).filter(|t| !t.is_empty())?;
    let runs = subtitle_runs(r);
    let info = parse_runs(&runs);

    if let Some((id, page_type)) = browse_endpoint(r) {
        return match classify_browse(id, page_type) {
            Target::Artist => Some(Item::Artist(ArtistSummary {
                id: id.to_owned(),
                name: title,
                thumbnails: thumbnails(r),
                audience: info.stats.first().cloned(),
            })),
            Target::Album => Some(Item::Album(album_summary(
                id,
                title,
                &info,
                thumbnails(r),
                play_playlist_id(r),
                has_explicit_badge(r),
            ))),
            Target::Playlist => Some(Item::Playlist(playlist_summary(
                id,
                title,
                &info,
                &runs,
                thumbnails(r),
            ))),
            Target::Other => None,
        };
    }

    let (video_id, video_type) = list_item_video(r);
    let kind = track_kind(video_type)?;
    let duration_s = info.duration_s.or_else(|| {
        at(
            r,
            "/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text",
        )
        .and_then(text)
        .and_then(|t| parse_duration(&t))
    });
    let views = info.views();
    Some(Item::Track(Track {
        video_id: video_id?.to_owned(),
        title,
        artists: info.artists,
        album: info.album,
        duration_s,
        thumbnails: thumbnails(r),
        explicit: has_explicit_badge(r),
        kind,
        year: info.year,
        views,
        playable: str_at(r, "/musicItemRendererDisplayPolicy")
            != Some("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT"),
    }))
}

fn album_summary(
    id: &str,
    title: String,
    info: &RunInfo,
    thumbnails: Vec<crate::models::Thumbnail>,
    playlist_id: Option<String>,
    explicit: bool,
) -> AlbumSummary {
    AlbumSummary {
        browse_id: id.to_owned(),
        playlist_id,
        title,
        kind: info
            .label
            .and_then(Label::album_kind)
            .unwrap_or(AlbumKind::Album),
        artists: info.artists.clone(),
        year: info.year,
        thumbnails,
        explicit,
    }
}

fn playlist_summary(
    id: &str,
    title: String,
    info: &RunInfo,
    runs: &[Value],
    thumbnails: Vec<crate::models::Thumbnail>,
) -> PlaylistSummary {
    // With a "Playlist •" label the first name is the author; without one the
    // subtitle is usually a list of featured artists, so only trust links.
    let author = if info.label.is_some() {
        info.artists.first().cloned()
    } else {
        info.artists.iter().find(|a| a.id.is_some()).cloned()
    };
    let track_count = info
        .stat_with(&["song", "track", "video", "episode"])
        .and_then(leading_count);
    let subtitle = if info.label.is_none() && author.is_none() {
        let s: String = runs.iter().filter_map(|r| str_at(r, "/text")).collect();
        Some(s).filter(|s| !s.trim().is_empty())
    } else {
        None
    };
    PlaylistSummary {
        id: strip_vl(id).to_owned(),
        title,
        author,
        thumbnails,
        track_count,
        subtitle,
    }
}

// ---------------------------------------------------------------------------
// musicTwoRowItemRenderer (cards in carousels and grids)

fn parse_two_row_item(r: &Value) -> Option<Item> {
    let title = text_at(r, "/title")?;
    let runs = arr_at(r, "/subtitle/runs");
    let info = parse_runs(runs);
    let thumbs = thumbnails(r);

    if let Some((id, page_type)) = browse_endpoint(r) {
        return match classify_browse(id, page_type) {
            Target::Artist => Some(Item::Artist(ArtistSummary {
                id: id.to_owned(),
                name: title,
                thumbnails: thumbs,
                audience: info.stats.first().cloned(),
            })),
            Target::Album => Some(Item::Album(album_summary(
                id,
                title,
                &info,
                thumbs,
                play_playlist_id(r),
                has_explicit_badge(r),
            ))),
            Target::Playlist => Some(Item::Playlist(playlist_summary(
                id, title, &info, runs, thumbs,
            ))),
            Target::Other => None,
        };
    }

    let watch = at(r, "/navigationEndpoint/watchEndpoint")?;
    let kind = track_kind(str_at(
        watch,
        "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
    ))?;
    let views = info.views();
    Some(Item::Track(Track {
        video_id: string_at(watch, "/videoId")?,
        title,
        artists: info.artists,
        album: info.album,
        duration_s: info.duration_s,
        thumbnails: thumbs,
        explicit: has_explicit_badge(r),
        kind,
        year: info.year,
        views,
        playable: true,
    }))
}

// ---------------------------------------------------------------------------
// playlistPanelVideoRenderer (watch queues)

pub fn parse_panel_video(v: &Value) -> Option<Track> {
    let r = v
        .get(PPVR)
        .or_else(|| at(v, &format!("/{PPVWR}/primaryRenderer/{PPVR}")))?;
    if r.get("unplayableText").is_some() {
        return None;
    }
    let info = parse_runs(arr_at(r, "/longBylineText/runs"));
    let kind = track_kind(str_at(
        r,
        "/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType",
    ))?;
    let views = info.views();
    Some(Track {
        video_id: string_at(r, "/videoId")?,
        title: text_at(r, "/title")?,
        artists: info.artists,
        album: info.album,
        duration_s: text_at(r, "/lengthText")
            .and_then(|t| parse_duration(&t))
            .or(info.duration_s),
        thumbnails: thumbnails(r),
        explicit: has_explicit_badge(r),
        kind,
        year: info.year,
        views,
        playable: true,
    })
}

// ---------------------------------------------------------------------------
// Shelves

/// Parses one entry of a `sectionListRenderer.contents` array into a titled
/// section. Unknown shelves yield `None`.
pub fn parse_section(v: &Value) -> Option<Section> {
    const SHELVES: [(&str, &str); 4] = [
        ("musicCarouselShelfRenderer", "/contents"),
        ("musicImmersiveCarouselShelfRenderer", "/contents"),
        ("gridRenderer", "/items"),
        ("musicShelfRenderer", "/contents"),
    ];
    let (shelf, items_ptr) = SHELVES
        .iter()
        .find_map(|(key, items)| Some((v.get(*key)?, *items)))?;
    let header = at(shelf, "/header/musicCarouselShelfBasicHeaderRenderer");
    let title = header
        .and_then(|h| text_at(h, "/title"))
        .or_else(|| text_at(shelf, "/header/gridHeaderRenderer/title"))
        .or_else(|| text_at(shelf, "/title"))
        .unwrap_or_default();
    Some(Section {
        title,
        strapline: header.and_then(|h| text_at(h, "/strapline")),
        items: parse_items(arr_at(shelf, items_ptr)),
    })
}

pub fn parse_sections(contents: &[Value]) -> Vec<Section> {
    contents
        .iter()
        .filter_map(parse_section)
        .filter(|s| !s.items.is_empty())
        .collect()
}

/// Author for a track whose row omits it (top-result card children).
pub fn fill_missing_artist(tracks: &mut [Item], artist: &ArtistRef) {
    for item in tracks {
        if let Item::Track(t) = item
            && t.artists.is_empty()
        {
            t.artists.push(artist.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_renderers_are_skipped() {
        assert!(parse_item(&json!({"messageRenderer": {}})).is_none());
        assert!(parse_item(&json!({MRLIR: {}})).is_none());
        assert!(parse_item(&json!({MTRIR: {"title": {"runs": []}}})).is_none());
        assert!(parse_section(&json!({"musicTastebuilderShelfRenderer": {}})).is_none());
    }

    #[test]
    fn podcast_episodes_are_not_tracks() {
        assert_eq!(track_kind(Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE")), None);
        assert_eq!(
            track_kind(Some("MUSIC_VIDEO_TYPE_UGC")),
            Some(TrackKind::Video)
        );
    }

    #[test]
    fn browse_classification_falls_back_to_prefix() {
        assert_eq!(classify_browse("MPREb_x", None), Target::Album);
        assert_eq!(classify_browse("VLPLx", None), Target::Playlist);
        assert_eq!(
            classify_browse("UCx", Some("MUSIC_PAGE_TYPE_USER_CHANNEL")),
            Target::Other
        );
    }
}
