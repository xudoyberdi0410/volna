//! lrc.red: word-synced TTML by ISRC (`/s/<ISRC>.ttml`); without an ISRC,
//! `match.json` finds one by title/artist/album/duration.

use reqwest::Client;
use serde_json::Value;

use super::{from_ttml, get_json, get_text, track_params, url};
use crate::error::Result;
use crate::model::{Lyrics, TrackQuery};

const BASE: &str = "https://lrc.red";

async fn ttml(http: &Client, isrc: &str) -> Result<Option<Lyrics>> {
    // Path segment, so the ISRC gets percent-encoded like encodeURIComponent.
    let mut u = url(BASE, &[])?;
    u.path_segments_mut()
        .map_err(|_| crate::Error::Parse(BASE.into()))?
        .clear()
        .push("s")
        .push(&format!("{isrc}.ttml"));
    let text = get_text(http, u).await?;
    Ok(from_ttml("lrc.red", &text))
}

/// ISRC of the first `match.json` / `search.json` hit.
pub fn first_hit_isrc(payload: &Value) -> Option<String> {
    payload["hits"][0]["isrc"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Option<Lyrics>> {
    if let Some(isrc) = q.isrc() {
        match ttml(http, isrc).await {
            Ok(Some(l)) => return Ok(Some(l)),
            Ok(None) => {}
            Err(e) => tracing::debug!("lrc.red by ISRC {isrc}: {e}"),
        }
    }
    let u = if q.has_title_artist() {
        url(
            &format!("{BASE}/match.json"),
            &track_params(q, ["track", "artist", "album", "duration"]),
        )?
    } else {
        return Ok(None);
    };
    match first_hit_isrc(&get_json(http, u).await?) {
        Some(isrc) => ttml(http, &isrc).await,
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_hit() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/lrcred-match.json")).unwrap();
        assert_eq!(first_hit_isrc(&v).as_deref(), Some("USUG11904206"));
        assert_eq!(first_hit_isrc(&serde_json::json!({"hits": []})), None);
    }
}
