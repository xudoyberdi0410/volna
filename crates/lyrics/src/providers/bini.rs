//! BiniLyrics (`lyrics-api.binimum.org`): a lookup by ISRC or by
//! title/artist/album/duration returns `results[0].lyricsUrl`, a TTML file.

use reqwest::Client;
use serde_json::Value;

use super::{from_ttml, get_json, get_text, track_params, url};
use crate::error::Result;
use crate::model::{Lyrics, TrackQuery};

const BASE: &str = "https://lyrics-api.binimum.org/";

/// `lyricsUrl` of the first result.
pub fn lyrics_url(payload: &Value) -> Option<String> {
    payload["results"][0]["lyricsUrl"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn has_results(payload: &Value) -> bool {
    payload["results"].as_array().is_some_and(|r| !r.is_empty())
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Option<Lyrics>> {
    if !q.has_title_artist() && q.isrc().is_none() {
        return Ok(None);
    }
    let mut found = None;
    if let Some(isrc) = q.isrc() {
        match get_json(http, url(BASE, &[("isrc", isrc.to_owned())])?).await {
            Ok(v) if has_results(&v) => found = Some(v),
            Ok(_) => {}
            Err(e) => tracing::debug!("BiniLyrics by ISRC: {e}"),
        }
    }
    if found.is_none() && q.has_title_artist() {
        let params = track_params(q, ["track", "artist", "album", "duration"]);
        found = Some(get_json(http, url(BASE, &params)?).await?);
    }
    let Some(target) = found.as_ref().and_then(lyrics_url) else {
        return Ok(None);
    };
    let target = url::Url::parse(&target).map_err(|_| crate::Error::Parse("BiniLyrics".into()))?;
    let ttml = get_text(http, target).await?;
    Ok(from_ttml("BiniLyrics", &ttml))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_first_lyrics_url() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/bini.json")).unwrap();
        assert!(has_results(&v));
        assert_eq!(
            lyrics_url(&v).as_deref(),
            Some("https://lyrics-api.binimum.org/ttml/abc123.ttml")
        );
        assert!(!has_results(&serde_json::json!({"results": []})));
    }
}
