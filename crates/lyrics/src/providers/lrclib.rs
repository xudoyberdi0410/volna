//! LRCLIB: the exact `/api/get` lookup the old UI used (`ui/js/lyrics.js`,
//! track/artist/album/duration), then `AmLyrics`'s `/api/search?q=artist title`
//! preferring a result with synced lyrics.

use reqwest::Client;
use serde_json::Value;

use super::{get_json, track_params, url};
use crate::error::{Error, Result};
use crate::model::{Lyrics, Synced, TrackQuery};

const BASE: &str = "https://lrclib.net/api";

/// Lyrics of one LRCLIB record: synced if present, else plain.
pub fn parse_record(r: &Value) -> Option<Lyrics> {
    if let Some(lrc) = r["syncedLyrics"].as_str() {
        let lines = crate::lrc::parse(lrc);
        if !lines.is_empty() {
            return Some(Lyrics::new("LRCLIB", lines));
        }
    }
    let plain = r["plainLyrics"].as_str()?;
    let l = Lyrics::from_plain("LRCLIB (unsynced)", plain, None);
    (!l.is_empty()).then_some(l)
}

/// The best record of a search: the first with synced lyrics, else the first.
pub fn parse_search(results: &Value) -> Option<Lyrics> {
    let arr = results.as_array()?;
    let best = arr
        .iter()
        .find(|r| r["syncedLyrics"].is_string())
        .or_else(|| arr.first())?;
    parse_record(best)
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Option<Lyrics>> {
    if !q.has_title_artist() {
        return Ok(None);
    }
    let exact = url(
        &format!("{BASE}/get"),
        &track_params(q, ["track_name", "artist_name", "album_name", "duration"]),
    )?;
    // `/api/get` gives a 404 on a miss. Only a synced hit ends the lookup:
    // the search may still find a synced version of an unsynced record.
    let unsynced = match get_json(http, exact).await {
        Ok(v) => match parse_record(&v) {
            Some(l) if l.synced != Synced::None => return Ok(Some(l)),
            other => other,
        },
        Err(Error::Status { status: 404, .. }) => None,
        Err(e) => {
            tracing::debug!("LRCLIB get: {e}");
            None
        }
    };
    let search = url(
        &format!("{BASE}/search"),
        &[("q", format!("{} {}", q.artist.trim(), q.title.trim()))],
    )?;
    match get_json(http, search).await {
        Ok(v) => Ok(parse_search(&v).or(unsynced)),
        Err(e) if unsynced.is_some() => {
            tracing::debug!("LRCLIB search: {e}");
            Ok(unsynced)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_search() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/lrclib-search.json")).unwrap();
        let l = parse_search(&v).unwrap();
        assert_eq!(l.source, "LRCLIB");
        assert_eq!(l.synced, Synced::Line);
        assert_eq!(l.lines[0].text, "Synced one");

        let plain = parse_record(&v[0]).unwrap();
        assert_eq!(plain.source, "LRCLIB (unsynced)");
        assert_eq!(plain.lines.len(), 2);

        assert!(parse_search(&serde_json::json!([])).is_none());
        assert!(
            parse_record(&serde_json::json!({"instrumental": true, "plainLyrics": null})).is_none()
        );
    }
}
