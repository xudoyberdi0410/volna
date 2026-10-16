//! Feed-like `browse` pages: home, moods & genres, related tab, lyrics.

use serde_json::Value;

use super::items::parse_sections;
use super::json::{arr_at, at, string_at, text_at};
use super::search::next_continuation;
use crate::models::{Item, Lyrics, Mood, MoodCategory, Related, Section};

const SINGLE_COL_SECTIONS: &str =
    "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer";

/// Sections and the continuation token of a `FEmusic_*` page.
pub fn parse_feed(resp: &Value) -> (Vec<Section>, Option<String>) {
    let list = at(resp, SINGLE_COL_SECTIONS)
        .or_else(|| at(resp, "/continuationContents/sectionListContinuation"))
        .or_else(|| at(resp, "/contents/sectionListRenderer"));
    match list {
        Some(l) => (parse_sections(arr_at(l, "/contents")), next_continuation(l)),
        None => (Vec::new(), None),
    }
}

pub fn parse_moods(resp: &Value) -> Vec<MoodCategory> {
    let Some(list) = at(resp, SINGLE_COL_SECTIONS) else {
        return Vec::new();
    };
    arr_at(list, "/contents")
        .iter()
        .filter_map(|s| s.get("gridRenderer"))
        .map(|grid| MoodCategory {
            title: text_at(grid, "/header/gridHeaderRenderer/title").unwrap_or_default(),
            moods: arr_at(grid, "/items")
                .iter()
                .filter_map(|i| i.get("musicNavigationButtonRenderer"))
                .filter_map(|b| {
                    Some(Mood {
                        title: text_at(b, "/buttonText")?,
                        params: string_at(b, "/clickCommand/browseEndpoint/params")?,
                        color: b
                            .pointer("/solid/leftStripeColor")
                            .and_then(Value::as_u64)
                            .and_then(|c| u32::try_from(c).ok()),
                    })
                })
                .collect(),
        })
        .filter(|c| !c.moods.is_empty())
        .collect()
}

pub fn parse_related(resp: &Value) -> Related {
    let contents = arr_at(resp, "/contents/sectionListRenderer/contents");
    let mut out = Related::default();
    for section in parse_sections(contents) {
        for item in section.items {
            match item {
                Item::Track(t) => out.tracks.push(t),
                Item::Artist(a) => out.artists.push(a),
                Item::Album(a) => out.albums.push(a),
                Item::Playlist(p) => out.playlists.push(p),
            }
        }
    }
    out.artist_description = contents
        .iter()
        .find_map(|s| text_at(s, "/musicDescriptionShelfRenderer/description"));
    out
}

pub fn parse_lyrics(resp: &Value) -> Option<Lyrics> {
    let shelf = arr_at(resp, "/contents/sectionListRenderer/contents")
        .iter()
        .find_map(|s| s.get("musicDescriptionShelfRenderer"))?;
    let text = text_at(shelf, "/description")?;
    let source = text_at(shelf, "/footer").map(|s| {
        s.strip_prefix("Source:")
            .map(str::trim)
            .unwrap_or(&s)
            .to_owned()
    });
    Some(Lyrics { text, source })
}
