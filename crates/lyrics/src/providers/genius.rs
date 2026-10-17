//! Genius through the `fetch-genius` worker `AmLyrics` uses: `{lyrics}` as
//! plain text with `[Verse]`-style headers, which are dropped.

use reqwest::Client;
use serde_json::Value;

use super::{get_json, url};
use crate::error::{Error, Result};
use crate::model::{Lyrics, TrackQuery};

const WORKER: &str = "https://fetch-genius.samidy.workers.dev/";

pub fn parse(payload: &Value) -> Option<Lyrics> {
    let text = payload["lyrics"].as_str()?;
    let l = Lyrics::from_plain("Genius", text, Some('['));
    (!l.is_empty()).then_some(l)
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Option<Lyrics>> {
    if !q.has_title_artist() {
        return Ok(None);
    }
    let u = url(
        WORKER,
        &[
            ("title", q.title.trim().to_owned()),
            ("artist", q.artist.trim().to_owned()),
        ],
    )?;
    match get_json(http, u).await {
        Ok(v) => Ok(parse(&v)),
        // "No results found" comes as a 404.
        Err(Error::Status { status: 404, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_headers() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/genius.json")).unwrap();
        let l = parse(&v).unwrap();
        let texts: Vec<&str> = l.lines.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, vec!["First line", "Second line", "Chorus line"]);
        assert!(parse(&serde_json::json!({"error": "No results found"})).is_none());
    }
}
