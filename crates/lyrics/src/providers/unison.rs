//! Unison (`unison.boidu.dev/lyrics`): `{success, data: {lyrics, format:
//! ttml|lrc, syncType: linesync|plain|…}}`.

use reqwest::Client;
use serde_json::Value;

use super::{from_ttml, get_json, track_params, url};
use crate::error::Result;
use crate::model::{Lyrics, TrackQuery};

const BASE: &str = "https://unison.boidu.dev/lyrics";

/// Lyrics in a Unison response.
pub fn parse(payload: &Value) -> Option<Lyrics> {
    if payload["success"].as_bool() != Some(true) {
        return None;
    }
    let data = &payload["data"];
    let text = data["lyrics"].as_str().filter(|s| !s.is_empty())?;
    let format = data["format"].as_str().unwrap_or("lrc");
    let sync = data["syncType"].as_str().unwrap_or("linesync");
    let l = match (format, sync) {
        ("ttml", _) => from_ttml("Unison", text)?,
        ("lrc", "plain") => Lyrics::from_plain("Unison (unsynced)", text, None),
        ("lrc", _) => Lyrics::new("Unison", crate::lrc::parse(text)),
        _ => return None,
    };
    (!l.is_empty()).then_some(l)
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Option<Lyrics>> {
    if !q.has_title_artist() {
        return Ok(None);
    }
    let params = track_params(q, ["song", "artist", "album", "duration"]);
    Ok(parse(&get_json(http, url(BASE, &params)?).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Synced;

    #[test]
    fn lrc_ttml_plain_and_miss() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/unison-lrc.json")).unwrap();
        let l = parse(&v).unwrap();
        assert_eq!(l.source, "Unison");
        assert_eq!(l.synced, Synced::Line);
        assert_eq!(l.lines.len(), 2);

        let ttml = serde_json::json!({"success": true, "data": {
            "format": "ttml", "syncType": "wordsync",
            "lyrics": include_str!("../../tests/fixtures/word.ttml")}});
        assert_eq!(parse(&ttml).unwrap().synced, Synced::Word);

        let plain = serde_json::json!({"success": true, "data": {
            "format": "lrc", "syncType": "plain", "lyrics": "One\n\nTwo"}});
        let p = parse(&plain).unwrap();
        assert_eq!(p.source, "Unison (unsynced)");
        assert_eq!(p.synced, Synced::None);

        let miss =
            serde_json::json!({"success": false, "error": "Lyrics not found", "code": "NOT_FOUND"});
        assert!(parse(&miss).is_none());
    }
}
