//! One module per provider. Each takes the shared HTTP client and returns
//! zero or more [`Lyrics`]; any error stays inside the provider (the chain
//! logs it and moves on).

use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::error::{Error, Result};
use crate::model::{Lyrics, TrackQuery};

pub mod bini;
pub mod genius;
pub mod lrclib;
pub mod lrcred;
pub mod lyricsplus;
pub mod unison;
pub mod ytm;

/// `FETCH_TIMEOUT_MS` of `AmLyrics`: one request.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

/// Some hosts (LRCLIB) ask clients to identify themselves.
const USER_AGENT: &str = concat!(
    "Volna/",
    env!("CARGO_PKG_VERSION"),
    " (lyrics; apple-music-web-components compatible)"
);

/// The providers in chain order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Provider {
    LrcRed,
    Bini,
    Unison,
    LyricsPlus,
    Lrclib,
    Genius,
    Ytm,
}

impl Provider {
    pub const ALL: [Provider; 7] = [
        Provider::LrcRed,
        Provider::Bini,
        Provider::Unison,
        Provider::LyricsPlus,
        Provider::Lrclib,
        Provider::Genius,
        Provider::Ytm,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Provider::LrcRed => "lrc.red",
            Provider::Bini => "BiniLyrics",
            Provider::Unison => "Unison",
            Provider::LyricsPlus => "lyricsplus",
            Provider::Lrclib => "LRCLIB",
            Provider::Genius => "Genius",
            Provider::Ytm => "YouTube Music",
        }
    }

    /// The whole provider's budget (it may make several requests).
    pub fn timeout(self) -> Duration {
        match self {
            // AmLyrics gives lrc.red 3 s; leave room for a proxy.
            Provider::LrcRed => Duration::from_secs(5),
            // Up to three mirrors plus the explicit binimum query.
            Provider::LyricsPlus => Duration::from_secs(25),
            _ => Duration::from_secs(15),
        }
    }
}

pub(crate) fn url(base: &str, params: &[(&str, String)]) -> Result<Url> {
    Url::parse_with_params(base, params).map_err(|_| Error::Parse(base.to_owned()))
}

/// `title`/`artist`/`album`/`duration` with the given key names, skipping
/// what is unknown.
pub(crate) fn track_params(q: &TrackQuery, keys: [&'static str; 4]) -> Vec<(&'static str, String)> {
    let mut p = vec![
        (keys[0], q.title.trim().to_owned()),
        (keys[1], q.artist.trim().to_owned()),
    ];
    if let Some(album) = q.album() {
        p.push((keys[2], album.to_owned()));
    }
    if let Some(d) = q.duration_s() {
        p.push((keys[3], d.to_string()));
    }
    p
}

async fn send(http: &Client, url: Url) -> Result<reqwest::Response> {
    let host = url.host_str().unwrap_or_default().to_owned();
    let res = http
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    if !res.status().is_success() {
        return Err(Error::Status {
            status: res.status().as_u16(),
            host,
        });
    }
    Ok(res)
}

pub(crate) async fn get_text(http: &Client, url: Url) -> Result<String> {
    Ok(send(http, url).await?.text().await?)
}

pub(crate) async fn get_json(http: &Client, url: Url) -> Result<Value> {
    let host = url.host_str().unwrap_or_default().to_owned();
    let text = get_text(http, url).await?;
    serde_json::from_str(&text).map_err(|_| Error::Parse(host))
}

/// TTML text → lyrics labelled `source`.
pub(crate) fn from_ttml(source: &str, ttml: &str) -> Option<Lyrics> {
    let t = crate::ttml::parse(ttml)?;
    Some(Lyrics::new(source, t.lines).with_songwriters(t.songwriters))
}
