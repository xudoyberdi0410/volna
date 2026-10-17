//! The `player` response: playability, the audio formats in it, the one
//! [`Quality`] asks for, and its final URL once the challenges are solved.

use serde::Deserialize;

use crate::error::{Result, StreamError};
use crate::model::{Codec, Quality, Tier};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlayerResponse {
    #[serde(default)]
    pub playability_status: Playability,
    #[serde(default)]
    pub streaming_data: Option<StreamingData>,
    #[serde(default)]
    pub video_details: Option<VideoDetails>,
    #[serde(default)]
    pub ad_placements: Vec<serde_json::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Playability {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VideoDetails {
    pub video_id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreamingData {
    #[serde(default)]
    pub adaptive_formats: Vec<Format>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Format {
    pub itag: u32,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub bitrate: Option<u64>,
    #[serde(default)]
    pub average_bitrate: Option<u64>,
    #[serde(default)]
    pub content_length: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub signature_cipher: Option<String>,
    #[serde(default)]
    pub cipher: Option<String>,
    #[serde(default)]
    pub audio_track: Option<AudioTrack>,
    #[serde(default)]
    pub is_drc: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AudioTrack {
    #[serde(default)]
    pub audio_is_default: bool,
}

/// Audio codec family of a format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Opus,
    Aac,
    Other,
}

impl Format {
    /// `audio/webm` from `audio/webm; codecs="opus"`.
    pub fn mime(&self) -> &str {
        self.mime_type.split(';').next().unwrap_or("").trim()
    }

    /// `opus` from `audio/webm; codecs="opus"`.
    pub fn codecs(&self) -> Option<&str> {
        let (_, rest) = self.mime_type.split_once("codecs=")?;
        Some(rest.trim().trim_matches('"'))
    }

    pub fn family(&self) -> Family {
        match (self.mime(), self.codecs().unwrap_or("")) {
            ("audio/webm", c) if c.contains("opus") => Family::Opus,
            ("audio/mp4", c) if c.starts_with("mp4a") => Family::Aac,
            _ => Family::Other,
        }
    }

    pub fn kbps(&self) -> Option<f64> {
        self.average_bitrate
            .or(self.bitrate)
            .map(|b| b as f64 / 1000.0)
    }

    fn rank(&self) -> u64 {
        self.average_bitrate.or(self.bitrate).unwrap_or(0)
    }

    /// Has a URL or a signature cipher (SABR-only formats have neither).
    pub fn has_url(&self) -> bool {
        self.url.is_some() || self.signature_cipher.is_some() || self.cipher.is_some()
    }

    /// `(url, encrypted signature, signature parameter name)`.
    pub fn locate(&self) -> Option<(String, Option<String>, String)> {
        if let Some(url) = &self.url {
            return Some((url.clone(), None, String::new()));
        }
        let cipher = self
            .signature_cipher
            .as_deref()
            .or(self.cipher.as_deref())?;
        let (mut url, mut s, mut sp) = (None, None, None);
        for (k, v) in url::form_urlencoded::parse(cipher.as_bytes()) {
            match &*k {
                "url" => url = Some(v.into_owned()),
                "s" => s = Some(v.into_owned()),
                "sp" => sp = Some(v.into_owned()),
                _ => {}
            }
        }
        Some((url?, s, sp.unwrap_or_else(|| "signature".into())))
    }
}

impl PlayerResponse {
    /// Turns a non-`OK` playability status into the error the UI explains.
    pub fn check(&self, id: &str) -> Result<()> {
        let status = self.playability_status.status.as_str();
        let reason = self
            .playability_status
            .reason
            .clone()
            .unwrap_or_else(|| status.to_owned());
        match status {
            "OK" => {}
            "LOGIN_REQUIRED" if reason.to_ascii_lowercase().contains("not a bot") => {
                return Err(StreamError::BotCheck(reason));
            }
            "" => return Err(StreamError::ResolveFailed("no playability status".into())),
            _ => return Err(StreamError::Unavailable(reason)),
        }
        if let Some(d) = &self.video_details
            && d.video_id != id
        {
            return Err(StreamError::ResolveFailed(format!(
                "got the player for {} instead of {id}",
                d.video_id
            )));
        }
        Ok(())
    }

    /// Seconds of pre-roll ads the web player would show before the content
    /// (yt-dlp waits that long before downloading).
    pub fn ad_seconds(&self) -> u64 {
        self.ad_placements
            .iter()
            .filter(|p| {
                p.pointer("/adPlacementRenderer/config/adPlacementConfig/kind")
                    .and_then(|k| k.as_str())
                    == Some("AD_PLACEMENT_KIND_START")
            })
            .filter_map(|p| p.pointer("/adPlacementRenderer/renderer/instreamVideoAdRenderer"))
            .filter_map(|r| {
                let skip = r
                    .get("skipOffsetMilliseconds")
                    .and_then(|v| v.as_u64())
                    .map(|ms| ms.div_ceil(1000));
                let length = r
                    .get("playerVars")
                    .and_then(|v| v.as_str())
                    .and_then(|vars| {
                        url::form_urlencoded::parse(vars.as_bytes())
                            .find(|(k, _)| k == "length_seconds")
                            .and_then(|(_, v)| v.parse().ok())
                    });
                skip.or(length)
            })
            .sum()
    }

    /// The audio format `quality` asks for.
    pub fn pick(&self, quality: Quality) -> Result<Format> {
        let formats = self
            .streaming_data
            .as_ref()
            .map(|s| s.adaptive_formats.as_slice())
            .unwrap_or_default();
        pick(formats, quality)
    }
}

pub(crate) fn pick(formats: &[Format], quality: Quality) -> Result<Format> {
    let audio: Vec<&Format> = formats
        .iter()
        .filter(|f| f.mime().starts_with("audio/"))
        .filter(|f| !f.is_drc)
        .filter(|f| f.audio_track.as_ref().is_none_or(|t| t.audio_is_default))
        .collect();
    if audio.is_empty() {
        return Err(StreamError::NoAudioStream("no audio formats".into()));
    }
    let playable: Vec<&Format> = audio.iter().copied().filter(|f| f.has_url()).collect();
    if playable.is_empty() {
        return Err(StreamError::NoAudioStream(
            "only SABR streams (no format has a URL)".into(),
        ));
    }
    let of = |family: Family| -> Vec<&Format> {
        playable
            .iter()
            .copied()
            .filter(|f| f.family() == family)
            .collect()
    };
    let candidates = match quality.codec {
        Codec::PreferOpus => {
            let opus = of(Family::Opus);
            if opus.is_empty() {
                of(Family::Aac)
            } else {
                opus
            }
        }
        Codec::AacOnly => of(Family::Aac),
    };
    let chosen = match quality.tier {
        Tier::High => candidates.into_iter().max_by_key(|f| f.rank()),
        Tier::Low => candidates.into_iter().min_by_key(|f| f.rank()),
    };
    chosen
        .cloned()
        .ok_or_else(|| StreamError::NoAudioStream(format!("no {:?} audio stream", quality.codec)))
}

/// The playable URL: decrypted signature under `sp`, transformed `n`, `pot`.
pub(crate) fn final_url(
    base: &str,
    sig: Option<(&str, &str)>,
    n: Option<&str>,
    pot: Option<&str>,
) -> Result<url::Url> {
    let mut url = url::Url::parse(base).map_err(|e| StreamError::InvalidUrl(e.to_string()))?;
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut set = |name: &str, value: &str| match pairs.iter_mut().find(|(k, _)| k == name) {
        Some(p) => p.1 = value.to_owned(),
        None => pairs.push((name.to_owned(), value.to_owned())),
    };
    if let Some((sp, value)) = sig {
        set(sp, value);
    }
    if let Some(n) = n {
        set("n", n);
    }
    if let Some(pot) = pot {
        set("pot", pot);
    }
    url.query_pairs_mut().clear().extend_pairs(&pairs);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(itag: u32, mime: &str, bitrate: u64) -> Format {
        Format {
            itag,
            mime_type: mime.into(),
            bitrate: Some(bitrate),
            average_bitrate: Some(bitrate - 1000),
            content_length: Some("1000".into()),
            url: Some(format!(
                "https://x.googlevideo.com/videoplayback?itag={itag}&n=abc"
            )),
            signature_cipher: None,
            cipher: None,
            audio_track: None,
            is_drc: false,
        }
    }

    fn formats() -> Vec<Format> {
        let opus = r#"audio/webm; codecs="opus""#;
        let aac = r#"audio/mp4; codecs="mp4a.40.2""#;
        let mut drc = fmt(251, opus, 200_000);
        drc.is_drc = true;
        vec![
            fmt(137, r#"video/mp4; codecs="avc1""#, 4_000_000),
            fmt(249, opus, 50_000),
            fmt(251, opus, 140_000),
            drc,
            fmt(139, aac, 49_000),
            fmt(140, aac, 130_000),
        ]
    }

    #[test]
    fn picks_by_codec_and_tier() {
        let f = formats();
        assert_eq!(pick(&f, Quality::High).unwrap().itag, 251);
        assert_eq!(pick(&f, Quality::Low).unwrap().itag, 249);
        let aac = Quality::High.with_codec(Codec::AacOnly);
        assert_eq!(pick(&f, aac).unwrap().itag, 140);
        assert_eq!(
            pick(&f, Quality::Low.with_codec(Codec::AacOnly))
                .unwrap()
                .itag,
            139
        );
        let only_aac: Vec<Format> = f.iter().filter(|x| x.itag == 140).cloned().collect();
        assert_eq!(
            pick(&only_aac, Quality::High).unwrap().itag,
            140,
            "Opus falls back to AAC"
        );
        let only_opus: Vec<Format> = f.iter().filter(|x| x.itag == 251).cloned().collect();
        assert!(matches!(
            pick(&only_opus, aac),
            Err(StreamError::NoAudioStream(_))
        ));
    }

    #[test]
    fn sabr_only_is_reported() {
        let mut f = formats();
        for x in &mut f {
            x.url = None;
        }
        let err = pick(&f, Quality::High).unwrap_err().to_string();
        assert!(err.contains("SABR"), "{err}");
    }

    #[test]
    fn reads_signature_cipher_and_builds_the_url() {
        let mut f = fmt(251, r#"audio/webm; codecs="opus""#, 140_000);
        f.url = None;
        f.signature_cipher = Some(
            "s=ABC%3D&sp=sig&url=https%3A%2F%2Fx.googlevideo.com%2Fvideoplayback%3Fitag%3D251%26n%3Dqwe".into(),
        );
        let (url, s, sp) = f.locate().unwrap();
        assert_eq!(s.as_deref(), Some("ABC="));
        assert_eq!(sp, "sig");
        let u = final_url(&url, Some((&sp, "CBA=")), Some("ewq"), Some("tok-_")).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        assert_eq!(q["sig"], "CBA=");
        assert_eq!(q["n"], "ewq");
        assert_eq!(q["pot"], "tok-_");
        assert_eq!(q["itag"], "251");
        assert_eq!(f.family(), Family::Opus);
        assert_eq!(f.codecs(), Some("opus"));
    }

    #[test]
    fn playability_errors() {
        let pr = |status: &str, reason: &str| -> PlayerResponse {
            serde_json::from_value(serde_json::json!({
                "playabilityStatus": {"status": status, "reason": reason}
            }))
            .unwrap()
        };
        assert!(matches!(
            pr("LOGIN_REQUIRED", "Sign in to confirm you’re not a bot").check("x"),
            Err(StreamError::BotCheck(_))
        ));
        assert!(matches!(
            pr("UNPLAYABLE", "Video unavailable").check("x"),
            Err(StreamError::Unavailable(_))
        ));
        assert!(pr("OK", "").check("x").is_ok());
    }

    #[test]
    fn counts_preroll_ads() {
        let pr: PlayerResponse = serde_json::from_value(serde_json::json!({
            "playabilityStatus": {"status": "OK"},
            "adPlacements": [{"adPlacementRenderer": {
                "config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_START"}},
                "renderer": {"instreamVideoAdRenderer": {
                    "playerVars": "length_seconds=6", "skipOffsetMilliseconds": 5000}}
            }}]
        }))
        .unwrap();
        assert_eq!(pr.ad_seconds(), 5);
    }
}
