//! `lyrics_get` / `lyrics_sources` over `volna-lyrics`.
//!
//! - `lyrics_get {videoId, source?, title?, artist?, album?, durationMs?, isrc?}`
//!   → `Lyrics | null` (the best source, or the named one).
//! - `lyrics_sources {videoId, …same}` → `[{source, synced, lines}]`, best first.
//!
//! Title/artist/album/duration are looked up through YouTube Music when the
//! caller does not pass them, and only on a cache miss. Only takes `ytm`, the
//! HTTP client and a data dir, so it moves into `volna-core` as is.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use serde_json::Value;
use volna_lyrics::{Key, LyricsService, TrackQuery};
use volna_ytm::YtmClient;

use crate::core::Core;
use crate::dispatch::out;

/// Arguments shared by both commands.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub video_id: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub isrc: Option<String>,
}

impl Args {
    fn key(&self) -> Key<'_> {
        Key {
            video_id: &self.video_id,
            isrc: self.isrc.as_deref().filter(|s| !s.trim().is_empty()),
        }
    }
}

/// The caller's description if complete, else YouTube Music's.
async fn describe(ytm: &YtmClient, a: &Args) -> volna_lyrics::Result<TrackQuery> {
    let given = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (title, artist) = (given(&a.title), given(&a.artist));
    let mut q = TrackQuery {
        title: title.clone().unwrap_or_default(),
        artist: artist.clone().unwrap_or_default(),
        album: given(&a.album),
        duration_ms: a.duration_ms.filter(|&d| d > 0),
        isrc: given(&a.isrc),
        video_id: Some(a.video_id.clone()),
    };
    if title.is_none() || artist.is_none() {
        let t = ytm.track(&a.video_id).await?.track;
        if title.is_none() {
            q.title = t.title;
        }
        if artist.is_none() {
            q.artist = t
                .artists
                .iter()
                .map(|x| x.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
        }
        if q.album.is_none() {
            q.album = t.album.map(|x| x.name).filter(|s| !s.is_empty());
        }
        if q.duration_ms.is_none() {
            q.duration_ms = t.duration_s.map(|s| u64::from(s) * 1000);
        }
    }
    Ok(q)
}

pub async fn lyrics_get(
    svc: &LyricsService,
    http: &reqwest::Client,
    ytm: &YtmClient,
    a: &Args,
) -> Result<Option<volna_lyrics::Lyrics>, String> {
    let query = || describe(ytm, a);
    match a.source.as_deref().filter(|s| !s.is_empty()) {
        Some(src) => svc.get_source(http, a.key(), src, query).await,
        None => svc.get(http, a.key(), query).await,
    }
    .map_err(|e| e.to_string())
}

pub async fn lyrics_sources(
    svc: &LyricsService,
    http: &reqwest::Client,
    ytm: &YtmClient,
    a: &Args,
) -> Result<Vec<volna_lyrics::SourceInfo>, String> {
    svc.sources(http, a.key(), || describe(ytm, a))
        .await
        .map_err(|e| e.to_string())
}

static SERVICE: OnceLock<Arc<LyricsService>> = OnceLock::new();

/// Opens `<data>/lyrics.sqlite3` on first use.
fn service(data_dir: &Path, ytm: YtmClient) -> Result<Arc<LyricsService>, String> {
    if let Some(s) = SERVICE.get() {
        return Ok(s.clone());
    }
    let svc = LyricsService::open(&data_dir.join("lyrics.sqlite3"), Some(ytm))
        .map_err(|e| format!("lyrics cache: {e}"))?;
    Ok(SERVICE.get_or_init(|| Arc::new(svc)).clone())
}

/// Lyrics commands; `None` for any other command.
pub async fn call(d: &Core, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    if !matches!(cmd, "lyrics_get" | "lyrics_sources") {
        return None;
    }
    Some(run(d, cmd, a).await)
}

async fn run(d: &Core, cmd: &str, a: &Value) -> Result<Value, String> {
    let args: Args = serde_json::from_value(a.clone()).map_err(|e| format!("invalid args: {e}"))?;
    if args.video_id.trim().is_empty() {
        return Err("invalid args `videoId`: empty".into());
    }
    let ytm = d.app.ytm.clone();
    let svc = service(&d.dirs.data, ytm.clone())?;
    let http = d.app.net.client();
    if cmd == "lyrics_sources" {
        out(lyrics_sources(&svc, &http, &ytm, &args).await)
    } else {
        out(lyrics_get(&svc, &http, &ytm, &args).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_from_page_keys() {
        let a: Args = serde_json::from_value(serde_json::json!({
            "videoId": "abc", "durationMs": 200000, "source": "LRCLIB"
        }))
        .unwrap();
        assert_eq!(a.video_id, "abc");
        assert_eq!(a.duration_ms, Some(200_000));
        assert_eq!(a.key().isrc, None);
    }

    #[tokio::test]
    async fn caller_description_skips_lookup() {
        // An unreachable client: any lookup would fail.
        let ytm = YtmClient::new(reqwest::Client::new());
        let a = Args {
            video_id: "abc".into(),
            title: Some(" Song ".into()),
            artist: Some("Artist".into()),
            duration_ms: Some(0),
            ..Default::default()
        };
        let q = describe(&ytm, &a).await.unwrap();
        assert_eq!(q.title, "Song");
        assert_eq!(q.duration_ms, None);
        assert_eq!(q.video_id.as_deref(), Some("abc"));
    }
}

#[cfg(test)]
mod live;
