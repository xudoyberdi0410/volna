//! Backend-neutral types: what is asked for ([`VideoId`], [`Quality`]) and
//! what a [`Backend`](crate::Backend) answers with ([`ResolvedStream`]).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Result, StreamError};

/// Assumed lifetime of a URL that carries no `expire` parameter.
#[allow(dead_code)] // used by the backends, none of which may be enabled
pub(crate) const FALLBACK_URL_LIFETIME: Duration = Duration::from_secs(3600);

/// A YouTube video id: 11 characters of `[A-Za-z0-9_-]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoId(String);

impl VideoId {
    pub fn parse(raw: &str) -> Result<Self> {
        let ok = raw.len() == 11
            && raw
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if ok {
            Ok(Self(raw.to_owned()))
        } else {
            Err(StreamError::InvalidVideoId(raw.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn watch_url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.0)
    }
}

impl std::fmt::Display for VideoId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Bitrate tier. `Low` saves bandwidth on slow proxies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    #[default]
    High,
    Low,
}

/// Which audio codecs the player can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// Opus (WebM) when there is one, AAC (m4a) otherwise: GStreamer,
    /// WebKit, WebView2 and Android's Media3 play both.
    #[default]
    PreferOpus,
    /// AAC in m4a only (itag 140/139): what AVPlayer on iOS plays.
    AacOnly,
}

impl Codec {
    /// Parses the `codec` query parameter of the stream URLs (`aac` or anything else).
    pub fn from_param(value: Option<&str>) -> Self {
        match value {
            Some(v) if v.eq_ignore_ascii_case("aac") || v.eq_ignore_ascii_case("m4a") => {
                Self::AacOnly
            }
            _ => Self::PreferOpus,
        }
    }
}

/// What to fetch for a track: a bitrate tier and a codec preference. Part
/// of the resolve cache key.
///
/// Serialized as `"high"`/`"low"` while the codec is the default (the format
/// the daemon commands already take), as `{"tier", "codec"}` otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(from = "QualityRepr", into = "QualityRepr")]
pub struct Quality {
    pub tier: Tier,
    pub codec: Codec,
}

#[allow(non_upper_case_globals)]
impl Quality {
    /// Best bitrate, Opus preferred (spelled like the former enum variant).
    pub const High: Self = Self::new(Tier::High, Codec::PreferOpus);
    /// Lowest bitrate, Opus preferred.
    pub const Low: Self = Self::new(Tier::Low, Codec::PreferOpus);
}

impl Quality {
    pub const fn new(tier: Tier, codec: Codec) -> Self {
        Self { tier, codec }
    }

    pub const fn with_codec(self, codec: Codec) -> Self {
        Self { codec, ..self }
    }

    /// Parses the `q` query parameter of the stream URLs (`low` or anything else).
    pub fn from_param(value: Option<&str>) -> Self {
        Self::from_params(value, None)
    }

    /// Parses the `q` and `codec` query parameters of the stream URLs.
    pub fn from_params(q: Option<&str>, codec: Option<&str>) -> Self {
        let tier = match q {
            Some(v) if v.eq_ignore_ascii_case("low") => Tier::Low,
            _ => Tier::High,
        };
        Self::new(tier, Codec::from_param(codec))
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum QualityRepr {
    Tier(Tier),
    Full { tier: Tier, codec: Codec },
}

impl From<QualityRepr> for Quality {
    fn from(r: QualityRepr) -> Self {
        match r {
            QualityRepr::Tier(tier) => Self::new(tier, Codec::default()),
            QualityRepr::Full { tier, codec } => Self::new(tier, codec),
        }
    }
}

impl From<Quality> for QualityRepr {
    fn from(q: Quality) -> Self {
        if q.codec == Codec::default() {
            Self::Tier(q.tier)
        } else {
            Self::Full {
                tier: q.tier,
                codec: q.codec,
            }
        }
    }
}

/// How the audio bytes of a [`ResolvedStream`] are fetched.
///
/// Only plain URLs exist today. YouTube is moving clients to SABR (a
/// server-driven protocol without a byte-addressable URL); that will be a
/// new variant here, opened by [`StreamService`](crate::StreamService)
/// behind the same Range-serving front door, so callers of
/// [`StreamService::open`](crate::StreamService::open) and the loopback
/// server do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Delivery {
    /// A googlevideo URL that answers HTTP Range requests.
    #[default]
    Url,
}

/// A playable stream plus what we know about it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedStream {
    pub url: String,
    pub delivery: Delivery,
    pub format_id: String,
    /// `audio/webm` or `audio/mp4`.
    pub mime: String,
    /// `opus`, `mp4a.40.2`, ...
    pub codec: Option<String>,
    pub bitrate_kbps: Option<f64>,
    /// Exact byte length when the backend knows it (or the URL carries `clen`).
    pub filesize: Option<u64>,
    /// When googlevideo stops accepting the URL (`expire` URL parameter).
    pub expires_at: SystemTime,
    /// User-Agent the backend resolved with; googlevideo may check it.
    pub user_agent: Option<String>,
}

/// A googlevideo URL query parameter.
#[allow(dead_code)]
pub(crate) fn url_param(url: &url::Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

/// `expire` of a googlevideo URL, or `now` + [`FALLBACK_URL_LIFETIME`].
#[allow(dead_code)]
pub(crate) fn url_expiry(url: &url::Url, now: SystemTime) -> SystemTime {
    url_param(url, "expire")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap_or(now + FALLBACK_URL_LIFETIME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_id_validation() {
        assert!(VideoId::parse("dQw4w9WgXcQ").is_ok());
        assert!(VideoId::parse("a-b_c-d_e-f").is_ok());
        for bad in ["", "short", "dQw4w9WgXcQx", "dQw4w9WgX/Q", "../../etc/p"] {
            assert!(VideoId::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn quality_param() {
        assert_eq!(Quality::from_param(Some("low")), Quality::Low);
        assert_eq!(Quality::from_param(Some("LOW")), Quality::Low);
        assert_eq!(Quality::from_param(Some("hi")), Quality::High);
        assert_eq!(Quality::from_param(None), Quality::High);
        assert_eq!(
            Quality::from_params(Some("low"), Some("aac")),
            Quality::new(Tier::Low, Codec::AacOnly)
        );
        assert_eq!(Quality::from_params(None, Some("opus")), Quality::High);
    }

    #[test]
    fn quality_serde_keeps_the_old_strings() {
        assert_eq!(serde_json::to_string(&Quality::Low).unwrap(), r#""low""#);
        assert_eq!(
            serde_json::from_str::<Quality>(r#""high""#).unwrap(),
            Quality::High
        );
        let aac = Quality::High.with_codec(Codec::AacOnly);
        let json = serde_json::to_string(&aac).unwrap();
        assert_eq!(json, r#"{"tier":"high","codec":"aac_only"}"#);
        assert_eq!(serde_json::from_str::<Quality>(&json).unwrap(), aac);
    }

    #[test]
    fn expiry_from_url() {
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let u = url::Url::parse("https://x.googlevideo.com/videoplayback?expire=5000").unwrap();
        assert_eq!(url_expiry(&u, now), UNIX_EPOCH + Duration::from_secs(5000));
        let u = url::Url::parse("https://x.googlevideo.com/videoplayback").unwrap();
        assert_eq!(url_expiry(&u, now), now + FALLBACK_URL_LIFETIME);
    }
}
