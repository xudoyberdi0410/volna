//! `search` and `music/get_search_suggestions` responses.

use serde_json::Value;

use super::items::{fill_missing_artist, parse_item, parse_items};
use super::json::{arr_at, at, browse_endpoint, str_at, string_at, text_at};
use super::runs::parse_runs;
use crate::models::{ArtistRef, ArtistSummary, Item, SearchFilter, SearchPage, SearchSuggestions};

/// InnerTube `params` for each filter (from ytmusicapi).
pub fn filter_params(filter: SearchFilter) -> Option<&'static str> {
    match filter {
        SearchFilter::All => None,
        SearchFilter::Songs => Some("EgWKAQIIAWoMEA4QChADEAQQCRAF"),
        SearchFilter::Videos => Some("EgWKAQIQAWoMEA4QChADEAQQCRAF"),
        SearchFilter::Albums => Some("EgWKAQIYAWoMEA4QChADEAQQCRAF"),
        SearchFilter::Artists => Some("EgWKAQIgAWoMEA4QChADEAQQCRAF"),
        SearchFilter::Playlists => Some("Eg-KAQwIABAAGAAgACgBMABqChAEEAMQCRAFEAo%3D"),
    }
}

/// Whether `item` belongs in the results of `filter`. YTM sometimes pads a
/// filtered search with an unrelated shelf; dropping by type keeps results
/// honest without reading localized shelf titles.
fn matches(filter: SearchFilter, item: &Item) -> bool {
    use crate::models::TrackKind;
    match (filter, item) {
        (SearchFilter::All, _) => true,
        (SearchFilter::Songs, Item::Track(t)) => t.kind == TrackKind::Song,
        (SearchFilter::Videos, Item::Track(t)) => t.kind != TrackKind::Song,
        (SearchFilter::Albums, Item::Album(_)) => true,
        (SearchFilter::Artists, Item::Artist(_)) => true,
        (SearchFilter::Playlists, Item::Playlist(_)) => true,
        _ => false,
    }
}

pub fn parse_search(resp: &Value, filter: SearchFilter) -> SearchPage {
    let mut page = SearchPage::default();
    let tabs = arr_at(resp, "/contents/tabbedSearchResultsRenderer/tabs");
    let sections = match tabs.first() {
        Some(tab) => arr_at(tab, "/tabRenderer/content/sectionListRenderer/contents"),
        None => arr_at(resp, "/contents/sectionListRenderer/contents"),
    };

    for section in sections {
        if let Some(card) = section.get("musicCardShelfRenderer") {
            let (top, children) = parse_top_card(card);
            page.top_result = page.top_result.or(top);
            page.items.extend(children);
        } else if let Some(shelf) = section.get("musicShelfRenderer") {
            page.items.extend(parse_items(arr_at(shelf, "/contents")));
            page.continuation = page.continuation.or_else(|| next_continuation(shelf));
        } else if let Some(shelf) = section.get("itemSectionRenderer") {
            page.items.extend(parse_items(arr_at(shelf, "/contents")));
        }
    }
    page.items.retain(|i| matches(filter, i));
    if filter == SearchFilter::All {
        page.continuation = None;
    }
    page
}

/// A `search` continuation response (`continuationContents.musicShelfContinuation`).
pub fn parse_search_continuation(resp: &Value, filter: SearchFilter) -> SearchPage {
    let shelf = at(resp, "/continuationContents/musicShelfContinuation");
    let mut items = shelf
        .map(|s| parse_items(arr_at(s, "/contents")))
        .unwrap_or_default();
    items.retain(|i| matches(filter, i));
    SearchPage {
        top_result: None,
        items,
        continuation: shelf.and_then(next_continuation),
    }
}

pub fn next_continuation(shelf: &Value) -> Option<String> {
    string_at(shelf, "/continuations/0/nextContinuationData/continuation").or_else(|| {
        string_at(
            shelf,
            "/continuations/0/nextRadioContinuationData/continuation",
        )
    })
}

/// The "Top result" card. Its children (e.g. an artist's top songs) often
/// omit the artist, so we fill it in from the card.
fn parse_top_card(card: &Value) -> (Option<Item>, Vec<Item>) {
    let title_run = at(card, "/title/runs/0");
    let info = parse_runs(arr_at(card, "/subtitle/runs"));

    let top = title_run.and_then(|run| {
        let name = str_at(run, "/text")?.to_owned();
        if let Some((id, page_type)) = browse_endpoint(run) {
            // Reuse the list-item logic by wrapping the card like a row.
            let fake = serde_json::json!({
                "musicResponsiveListItemRenderer": {
                    "flexColumns": [
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": name}]}}},
                        {"musicResponsiveListItemFlexColumnRenderer": {"text": card.get("subtitle").cloned().unwrap_or_default()}},
                    ],
                    "navigationEndpoint": {"browseEndpoint": {
                        "browseId": id,
                        "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": page_type}},
                    }},
                    "thumbnail": card.get("thumbnail").cloned().unwrap_or_default(),
                    "overlay": card.pointer("/thumbnailOverlay").cloned().unwrap_or_default(),
                    "badges": card.get("subtitleBadges").cloned().unwrap_or_default(),
                }
            });
            return parse_item(&fake);
        }
        // Song / video card: the watch target lives in onTap.
        let watch = at(card, "/onTap/watchEndpoint")
            .or_else(|| at(run, "/navigationEndpoint/watchEndpoint"))?;
        let fake = serde_json::json!({
            "musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": name}]},
                "subtitle": card.get("subtitle").cloned().unwrap_or_default(),
                "navigationEndpoint": {"watchEndpoint": watch},
                "thumbnailRenderer": card.get("thumbnail").cloned().unwrap_or_default(),
                "subtitleBadges": card.get("subtitleBadges").cloned().unwrap_or_default(),
            }
        });
        parse_item(&fake)
    });

    let mut children = parse_items(arr_at(card, "/contents"));
    let card_artist = match &top {
        Some(Item::Artist(ArtistSummary { id, name, .. })) => Some(ArtistRef {
            id: Some(id.clone()),
            name: name.clone(),
        }),
        _ => info.artists.first().cloned(),
    };
    if let Some(a) = card_artist {
        fill_missing_artist(&mut children, &a);
    }
    (top, children)
}

pub fn parse_suggestions(resp: &Value) -> SearchSuggestions {
    let mut out = SearchSuggestions::default();
    for section in arr_at(resp, "/contents") {
        for entry in arr_at(section, "/searchSuggestionsSectionRenderer/contents") {
            let sugg = entry
                .get("searchSuggestionRenderer")
                .or_else(|| entry.get("historySuggestionRenderer"));
            if let Some(s) = sugg {
                if let Some(q) = string_at(s, "/navigationEndpoint/searchEndpoint/query")
                    .or_else(|| text_at(s, "/suggestion"))
                {
                    out.queries.push(q);
                }
            } else if let Some(item) = parse_item(entry) {
                out.items.push(item);
            }
        }
    }
    out
}
