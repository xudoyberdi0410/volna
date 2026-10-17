//! YouLyPlus / lyricsplus ("KPoe") mirrors: `/v2/lyrics/get` answers from
//! Apple Music, Musixmatch, Spotify, QQ or Deezer, labelled in
//! `metadata.source`. Ported from `AmLyrics.fetchLyricsFromYouLyPlus`
//! (called there with `skipBiniCache`, since the chain asks Bini itself).

use reqwest::Client;
use serde_json::Value;

use super::{get_json, url};
use crate::error::Result;
use crate::kpoe;
use crate::model::{Lyrics, TrackQuery};
use crate::rank::rank;

pub const SERVERS: [&str; 4] = [
    "https://lyricsplus.binimum.org",
    "https://lyricsplus-seven.vercel.app",
    "https://lyricsplus.prjktla.workers.dev",
    "https://lyrics-plus-backend.vercel.app",
];

/// The server asked again for word-synced lyrics when no mirror gave
/// Apple or Bini word sync.
const EXPLICIT: &str = "https://lyricsplus.binimum.org";

fn params(q: &TrackQuery) -> Vec<(&'static str, String)> {
    let mut p = Vec::new();
    if !q.title.trim().is_empty() {
        p.push(("title", q.title.trim().to_owned()));
    }
    if !q.artist.trim().is_empty() {
        p.push(("artist", q.artist.trim().to_owned()));
    }
    if let Some(isrc) = q.isrc() {
        p.push(("isrc", isrc.to_owned()));
    }
    if let Some(album) = q.album() {
        p.push(("album", album.to_owned()));
    }
    if let Some(d) = q.duration_s() {
        p.push(("duration", d.to_string()));
    }
    p
}

/// Lyrics in one `/v2/lyrics/get` payload.
pub fn parse(payload: &Value) -> Option<Lyrics> {
    let lines = kpoe::convert(payload)?;
    Some(Lyrics::new(kpoe::source_label(payload), lines))
}

fn word_synced(l: &Lyrics) -> bool {
    l.lines
        .iter()
        .any(|x| x.words.as_ref().is_some_and(|w| w.len() > 1))
}

async fn ask(http: &Client, base: &str, q: &TrackQuery) -> Result<Option<Lyrics>> {
    let u = url(&format!("{base}/v2/lyrics/get"), &params(q))?;
    Ok(parse(&get_json(http, u).await?))
}

pub async fn fetch(http: &Client, q: &TrackQuery) -> Result<Vec<Lyrics>> {
    if !q.has_title_artist() && q.isrc().is_none() {
        return Ok(Vec::new());
    }
    // A random three of the mirrors, tried in turn.
    let mut servers = SERVERS.to_vec();
    fastrand::shuffle(&mut servers);
    servers.truncate(3);

    let mut out: Vec<Lyrics> = Vec::new();
    for base in servers {
        match ask(http, base, q).await {
            Ok(Some(l)) => {
                let best = rank(&l) == 1;
                out.push(l);
                if best {
                    break;
                }
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("lyricsplus {base}: {e}"),
        }
    }

    if !out.iter().any(|l| rank(l) <= 2) {
        match ask(http, EXPLICIT, q).await {
            Ok(Some(l)) if word_synced(&l) => out.push(l),
            Ok(_) => {}
            Err(e) => tracing::debug!("lyricsplus explicit: {e}"),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Synced;

    #[test]
    fn parses_payload_and_params() {
        let v: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/kpoe-word.json")).unwrap();
        let l = parse(&v).unwrap();
        assert_eq!(l.source, "qApple");
        assert_eq!(l.synced, Synced::Word);
        assert_eq!(rank(&l), 1);

        let q = TrackQuery {
            title: "T".into(),
            artist: "A".into(),
            album: Some(" ".into()),
            duration_ms: Some(200_400),
            isrc: Some("X".into()),
            video_id: None,
        };
        let keys: Vec<_> = params(&q)
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        assert_eq!(keys, vec!["title=T", "artist=A", "isrc=X", "duration=200"]);
    }
}
