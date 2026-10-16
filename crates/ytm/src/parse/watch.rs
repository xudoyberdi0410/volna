//! `next` responses: watch queues (radio, up next) and per-track tab ids.

use serde_json::Value;

use super::items::parse_panel_video;
use super::json::{arr_at, at, str_at, string_at};
use super::search::next_continuation;
use crate::models::{TrackPage, WatchQueue};

const TABS: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs";

/// Browse id of the watch-page tab with the given page type. Tabs marked
/// `unselectable` (no lyrics for this song) are skipped.
fn tab_browse_id(resp: &Value, page_type: &str) -> Option<String> {
    arr_at(resp, TABS).iter().find_map(|tab| {
        let t = tab.get("tabRenderer")?;
        if t.get("unselectable").and_then(Value::as_bool) == Some(true) {
            return None;
        }
        let be = at(t, "/endpoint/browseEndpoint")?;
        (str_at(
            be,
            "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
        ) == Some(page_type))
        .then(|| string_at(be, "/browseId"))
        .flatten()
    })
}

pub fn lyrics_browse_id(resp: &Value) -> Option<String> {
    tab_browse_id(resp, "MUSIC_PAGE_TYPE_TRACK_LYRICS")
}

pub fn related_browse_id(resp: &Value) -> Option<String> {
    tab_browse_id(resp, "MUSIC_PAGE_TYPE_TRACK_RELATED")
}

fn panel(resp: &Value) -> Option<&Value> {
    arr_at(resp, TABS).iter().find_map(|tab| {
        at(
            tab,
            "/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer",
        )
    })
}

pub fn parse_watch(resp: &Value) -> WatchQueue {
    let panel = panel(resp);
    let tracks = panel
        .map(|p| {
            arr_at(p, "/contents")
                .iter()
                .filter_map(parse_panel_video)
                .collect()
        })
        .unwrap_or_default();
    WatchQueue {
        playlist_id: panel.and_then(|p| string_at(p, "/playlistId")),
        tracks,
        continuation: panel.and_then(next_continuation),
        lyrics_browse_id: lyrics_browse_id(resp),
        related_browse_id: related_browse_id(resp),
    }
}

pub fn parse_watch_continuation(resp: &Value) -> TrackPage {
    let panel = at(resp, "/continuationContents/playlistPanelContinuation");
    TrackPage {
        tracks: panel
            .map(|p| {
                arr_at(p, "/contents")
                    .iter()
                    .filter_map(parse_panel_video)
                    .collect()
            })
            .unwrap_or_default(),
        continuation: panel.and_then(next_continuation),
    }
}
