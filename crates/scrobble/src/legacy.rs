//! The old UI's localStorage keys (`ui/js/storage.js`, `lastfm.js`,
//! `librefm.js`) → [`Config`]. Values are the raw localStorage strings.
//!
//! | key | field |
//! |---|---|
//! | `lastfm-enabled` | `lastfm.enabled` |
//! | `lastfm-session` (`{"key","name"}`) | `lastfm.session` |
//! | `lastfm-love-on-like` | `lastfm.loveOnLike` |
//! | `lastfm-scrobble-percentage` | `percentage` (absent: the UI's 75) |
//! | `lastfm-use-custom-credentials` | `lastfm.useCustomCredentials` |
//! | `lastfm-custom-api-key` / `-secret` (base64 of the reversed string) | `lastfm.customApiKey` / `customApiSecret` |
//! | `librefm-enabled` / `-session` / `-love-on-like` | `librefm.*` |
//! | `listenbrainz-enabled` / `-token` / `-custom-url` / `-love-on-like` | `listenbrainz.enabled` / `token` / `url` / `loveOnLike` |
//! | `maloja-enabled` / `-token` / `-custom-url` | `maloja.enabled` / `token` / `url` |
//!
//! Only keys present in the dump change the config.

use std::collections::HashMap;

use base64::Engine;
use serde_json::Value;

use crate::audioscrobbler::Session;
use crate::config::Config;

/// The percentage the old UI used when the user never set one.
pub const UI_DEFAULT_PERCENTAGE: u8 = 75;

const PREFIXES: [&str; 4] = ["lastfm-", "librefm-", "listenbrainz-", "maloja-"];

/// A JSON object of localStorage keys as strings (numbers and booleans are
/// stringified, `null` skipped), for dumps that were parsed on the way.
pub fn keys_from_json(v: &Value) -> HashMap<String, String> {
    v.as_object()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => return None,
                Value::Bool(_) | Value::Number(_) => v.to_string(),
                // `lastfm-session` parsed into an object.
                other => other.to_string(),
            };
            Some((k.clone(), s))
        })
        .collect()
}

/// `decodeSensitiveData`: base64, then the characters reversed.
fn decode_sensitive(s: &str) -> String {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .map(|b| b.iter().rev().map(|&c| char::from(c)).collect())
        .unwrap_or_default()
}

fn session(s: &str) -> Option<Session> {
    let s: Session = serde_json::from_str(s).ok()?;
    (!s.key.is_empty()).then_some(s)
}

pub fn apply(c: &mut Config, keys: &HashMap<String, String>) {
    let get = |k: &str| keys.get(k).map(String::as_str);
    let flag = |k: &str, slot: &mut bool| {
        if let Some(v) = get(k) {
            *slot = v == "true";
        }
    };
    let text = |k: &str, slot: &mut String| {
        if let Some(v) = get(k) {
            *slot = v.trim().to_owned();
        }
    };

    if let Some(p) = get("lastfm-scrobble-percentage") {
        if let Ok(p) = p.trim().parse::<i64>() {
            c.percentage = p.clamp(1, 100) as u8;
        }
    } else if keys
        .keys()
        .any(|k| PREFIXES.iter().any(|p| k.starts_with(p)))
    {
        c.percentage = UI_DEFAULT_PERCENTAGE;
    }

    flag("lastfm-enabled", &mut c.lastfm.enabled);
    flag("lastfm-love-on-like", &mut c.lastfm.love_on_like);
    flag(
        "lastfm-use-custom-credentials",
        &mut c.lastfm.use_custom_credentials,
    );
    if let Some(v) = get("lastfm-custom-api-key") {
        c.lastfm.custom_api_key = decode_sensitive(v);
    }
    if let Some(v) = get("lastfm-custom-api-secret") {
        c.lastfm.custom_api_secret = decode_sensitive(v);
    }
    if let Some(s) = get("lastfm-session").and_then(session) {
        c.lastfm.session = Some(s);
    }

    flag("librefm-enabled", &mut c.librefm.enabled);
    flag("librefm-love-on-like", &mut c.librefm.love_on_like);
    if let Some(s) = get("librefm-session").and_then(session) {
        c.librefm.session = Some(s);
    }

    flag("listenbrainz-enabled", &mut c.listenbrainz.enabled);
    flag(
        "listenbrainz-love-on-like",
        &mut c.listenbrainz.love_on_like,
    );
    text("listenbrainz-token", &mut c.listenbrainz.token);
    text("listenbrainz-custom-url", &mut c.listenbrainz.url);

    flag("maloja-enabled", &mut c.maloja.enabled);
    text("maloja-token", &mut c.maloja.token);
    text("maloja-custom-url", &mut c.maloja.url);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Service;
    use serde_json::json;

    #[test]
    fn imports_every_key() {
        // btoa("terces") = "dGVyY2Vz" → reversed "secret".
        let keys = keys_from_json(&json!({
            "lastfm-enabled": "true",
            "lastfm-session": {"key": "SK", "name": "me"},
            "lastfm-love-on-like": "true",
            "lastfm-scrobble-percentage": "60",
            "lastfm-use-custom-credentials": "true",
            "lastfm-custom-api-key": "eWVr",
            "lastfm-custom-api-secret": "dGVyY2Vz",
            "librefm-enabled": "true",
            "librefm-session": "{\"key\":\"LK\",\"name\":\"lib\"}",
            "librefm-love-on-like": "false",
            "listenbrainz-enabled": true,
            "listenbrainz-token": " tok ",
            "listenbrainz-custom-url": "https://lb.example",
            "listenbrainz-love-on-like": "true",
            "maloja-enabled": "true",
            "maloja-token": "mk",
            "maloja-custom-url": "https://m.example",
            "theme": "dark",
        }));
        let mut c = Config::default();
        apply(&mut c, &keys);
        assert_eq!(c.percentage, 60);
        assert!(c.lastfm.enabled && c.lastfm.love_on_like && c.lastfm.use_custom_credentials);
        assert_eq!(c.lastfm.custom_api_key, "key");
        assert_eq!(c.lastfm.custom_api_secret, "secret");
        assert_eq!(c.lastfm.session.as_ref().unwrap().key, "SK");
        assert_eq!(c.librefm.session.as_ref().unwrap().name, "lib");
        assert!(!c.librefm.love_on_like);
        assert_eq!(c.listenbrainz.token, "tok");
        assert_eq!(c.listenbrainz.url, "https://lb.example");
        assert!(c.listenbrainz.love_on_like);
        assert_eq!(c.maloja.url, "https://m.example");
        assert!(Service::ALL.iter().all(|s| c.active(*s)));
    }

    #[test]
    fn missing_keys_keep_the_config() {
        let mut c = Config::default();
        apply(&mut c, &HashMap::new());
        assert_eq!(c, Config::default());
        let mut c = Config::default();
        apply(
            &mut c,
            &keys_from_json(&json!({"maloja-enabled": "false", "lastfm-session": "garbage"})),
        );
        assert_eq!(c.percentage, UI_DEFAULT_PERCENTAGE, "the old UI's default");
        assert!(c.lastfm.session.is_none());
    }
}
