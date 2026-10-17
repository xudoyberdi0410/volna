//! The YouTube Music web client's configuration (`ytcfg`), taken from the
//! `music.youtube.com` page the way yt-dlp's `web_music` client does, and the
//! InnerTube request headers built from it.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{Map, Value, json};

use super::cookies::CookieJar;
use crate::error::{Result, StreamError};

pub(crate) const MUSIC_ORIGIN: &str = "https://music.youtube.com";
pub(crate) const MUSIC_HOST: &str = "music.youtube.com";
/// `WEB_REMIX`'s numeric id (`X-YouTube-Client-Name`).
const CLIENT_NAME_ID: u32 = 67;
const CLIENT_NAME: &str = "WEB_REMIX";
/// Used when the page gives none (yt-dlp 2026.08.19's default).
const FALLBACK_CLIENT_VERSION: &str = "1.20260707.12.00";

/// What the resolver needs from `ytcfg`.
#[derive(Debug, Clone)]
pub(crate) struct Session {
    /// `INNERTUBE_CONTEXT`, with `hl`/`timeZone` pinned like yt-dlp does.
    pub context: Value,
    pub client_version: String,
    pub visitor_data: Option<String>,
    /// `DATASYNC_ID`: `delegated||user` or `user||`.
    pub data_sync_id: Option<String>,
    pub session_index: Option<u32>,
    pub logged_in: bool,
    /// Path or URL of the player JS (`PLAYER_JS_URL`).
    pub player_url: Option<String>,
    /// `STS` when the page has it.
    pub sts: Option<u32>,
    /// The web player experiment that binds GVS PO tokens to the video id.
    pub bind_pot_to_video: bool,
    pub fetched_at: Instant,
}

impl Session {
    /// Parses every `ytcfg.set({...})` on the page, later calls overriding
    /// earlier keys (the page sets the config in several chunks).
    pub fn from_page(html: &str, now: Instant) -> Option<Self> {
        let cfg = extract_ytcfg(html);
        if cfg.is_empty() {
            return None;
        }
        Some(Self::from_ytcfg(&cfg, now))
    }

    pub fn from_ytcfg(cfg: &Map<String, Value>, now: Instant) -> Self {
        let str_at = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_owned);
        let mut context = cfg
            .get("INNERTUBE_CONTEXT")
            .cloned()
            .filter(Value::is_object)
            .unwrap_or_else(default_context);
        if let Some(client) = context.get_mut("client").and_then(Value::as_object_mut) {
            client.insert("hl".into(), json!("en"));
            client.insert("timeZone".into(), json!("UTC"));
            client.insert("utcOffsetMinutes".into(), json!(0));
        }
        let client_version = str_at(cfg.get("INNERTUBE_CLIENT_VERSION"))
            .or_else(|| str_at(context.pointer("/client/clientVersion")))
            .unwrap_or_else(|| FALLBACK_CLIENT_VERSION.to_owned());
        let visitor_data = str_at(cfg.get("VISITOR_DATA"))
            .or_else(|| str_at(context.pointer("/client/visitorData")));
        let player_url = str_at(cfg.get("PLAYER_JS_URL")).or_else(|| {
            cfg.get("WEB_PLAYER_CONTEXT_CONFIGS")
                .and_then(Value::as_object)
                .and_then(|m| m.values().find_map(|c| str_at(c.get("jsUrl"))))
        });
        let bind_pot_to_video = cfg
            .get("WEB_PLAYER_CONTEXT_CONFIGS")
            .and_then(Value::as_object)
            .is_some_and(|m| {
                m.values().any(|c| {
                    c.get("serializedExperimentFlags")
                        .and_then(Value::as_str)
                        .is_some_and(|flags| {
                            url::form_urlencoded::parse(flags.as_bytes())
                                .any(|(k, v)| k == "html5_generate_content_po_token" && v == "true")
                        })
                })
            });
        Self {
            context,
            client_version,
            visitor_data,
            data_sync_id: str_at(cfg.get("DATASYNC_ID")),
            session_index: cfg
                .get("SESSION_INDEX")
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                .and_then(|v| u32::try_from(v).ok()),
            logged_in: cfg
                .get("LOGGED_IN")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            player_url,
            sts: cfg
                .get("STS")
                .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                .and_then(|v| u32::try_from(v).ok()),
            bind_pot_to_video,
            fetched_at: now,
        }
    }

    /// A session without a page (YTM page unreachable): yt-dlp's built-in
    /// client context; the player comes from `iframe_api`.
    pub fn fallback(now: Instant) -> Self {
        Self::from_ytcfg(&Map::new(), now)
    }

    /// `(delegated_session_id, user_session_id)` from `DATASYNC_ID`.
    pub fn session_ids(&self) -> (Option<&str>, Option<&str>) {
        let Some(id) = self.data_sync_id.as_deref().filter(|s| !s.is_empty()) else {
            return (None, None);
        };
        match id.split_once("||") {
            Some((first, second)) if !second.is_empty() => (Some(first), Some(second)),
            Some((first, _)) => (None, Some(first)),
            None => (None, Some(id)),
        }
    }

    /// Headers for an InnerTube call on `music.youtube.com`.
    pub fn api_headers(&self, user_agent: &str, cookies: Option<&CookieJar>) -> HeaderMap {
        let mut h = HeaderMap::new();
        let mut put = |name: &'static str, value: &str| {
            if let Ok(v) = HeaderValue::from_str(value) {
                h.insert(HeaderName::from_static(name), v);
            }
        };
        put("content-type", "application/json");
        put("user-agent", user_agent);
        put("origin", MUSIC_ORIGIN);
        put("x-youtube-client-name", &CLIENT_NAME_ID.to_string());
        put("x-youtube-client-version", &self.client_version);
        if let Some(v) = &self.visitor_data {
            put("x-goog-visitor-id", v);
        }
        let (delegated, user) = self.session_ids();
        if let Some(cookies) = cookies {
            if let Some(d) = delegated {
                put("x-goog-pageid", d);
            }
            if delegated.is_some() || self.session_index.is_some() {
                put(
                    "x-goog-authuser",
                    &self.session_index.unwrap_or(0).to_string(),
                );
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(auth) = cookies.authorization(MUSIC_ORIGIN, user, now) {
                put("authorization", &auth);
                put("x-origin", MUSIC_ORIGIN);
            }
            if let Some(c) = cookies.header_for(MUSIC_HOST) {
                put("cookie", &c);
            }
        }
        if self.logged_in {
            put("x-youtube-bootstrap-logged-in", "true");
        }
        h
    }
}

/// yt-dlp's `web_music` context, for when the page cannot be read.
fn default_context() -> Value {
    let client = json!({
        "clientName": CLIENT_NAME,
        "clientVersion": FALLBACK_CLIENT_VERSION,
    });
    json!({ "client": client })
}

/// Every `ytcfg.set({...})` object on a page, merged.
pub(crate) fn extract_ytcfg(html: &str) -> Map<String, Value> {
    let mut merged = Map::new();
    let mut rest = html;
    while let Some(pos) = rest.find("ytcfg.set(") {
        rest = &rest[pos + "ytcfg.set(".len()..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with('{') {
            continue;
        }
        let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
        if let Some(Ok(Value::Object(obj))) = stream.next() {
            merged.extend(obj);
        }
    }
    merged
}

/// Player id from a player URL: `/s/player/<id>/...`.
pub(crate) fn player_id(player_url: &str) -> Option<&str> {
    let rest = &player_url[player_url.find("/s/player/")? + "/s/player/".len()..];
    let id = &rest[..rest.find('/')?];
    (id.len() >= 8 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// The player id from `https://www.youtube.com/iframe_api` (`player\/<id>\/`).
pub(crate) fn player_id_from_iframe_api(js: &str) -> Option<String> {
    let mut rest = js;
    while let Some(pos) = rest.find("player") {
        rest = &rest[pos + "player".len()..];
        let r = rest.strip_prefix('\\').unwrap_or(rest);
        let Some(r) = r.strip_prefix('/') else {
            continue;
        };
        let id: String = r.chars().take_while(char::is_ascii_hexdigit).collect();
        let after = &r[id.len()..];
        if id.len() == 8 && (after.starts_with('/') || after.starts_with("\\/")) {
            return Some(id);
        }
    }
    None
}

/// Rejects the "YouTube Music is not available in your area" page.
pub(crate) fn check_page(html: &str) -> Result<()> {
    if html.contains("not available in your area") || html.contains("unavailable in your country") {
        return Err(StreamError::Unavailable(
            "YouTube Music is not available from this network".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<script>ytcfg.set({"INNERTUBE_CONTEXT":{"client":{"clientName":"WEB_REMIX","clientVersion":"1.2","visitorData":"VD%3D","hl":"ru"}},"INNERTUBE_CLIENT_VERSION":"1.2","LOGGED_IN":true,"DATASYNC_ID":"111||","SESSION_INDEX":0});</script>
<script>ytcfg.set({"PLAYER_JS_URL":"/s/player/0229cd64/player_es6.vflset/en_US/base.js","WEB_PLAYER_CONTEXT_CONFIGS":{"WEB_PLAYER_CONTEXT_CONFIG_ID_MUSIC_WATCH":{"jsUrl":"/s/player/0229cd64/x.js","serializedExperimentFlags":"a=1&html5_generate_content_po_token=true&b=2"}},"STS":20739});</script>"#;

    #[test]
    fn reads_ytcfg_from_the_page() {
        let s = Session::from_page(PAGE, Instant::now()).unwrap();
        assert_eq!(s.client_version, "1.2");
        assert_eq!(s.visitor_data.as_deref(), Some("VD%3D"));
        assert_eq!(s.context["client"]["hl"], "en");
        assert_eq!(s.context["client"]["timeZone"], "UTC");
        assert!(s.logged_in);
        assert_eq!(s.session_index, Some(0));
        assert_eq!(s.session_ids(), (None, Some("111")));
        assert_eq!(s.sts, Some(20739));
        assert!(s.bind_pot_to_video);
        assert_eq!(
            player_id(s.player_url.as_deref().unwrap()),
            Some("0229cd64")
        );
        assert!(Session::from_page("<html></html>", Instant::now()).is_none());
    }

    #[test]
    fn session_ids_of_a_secondary_channel() {
        let mut s = Session::fallback(Instant::now());
        s.data_sync_id = Some("del||usr".into());
        assert_eq!(s.session_ids(), (Some("del"), Some("usr")));
        assert_eq!(s.context["client"]["clientName"], "WEB_REMIX");
    }

    #[test]
    fn headers_carry_auth_only_with_cookies() {
        let s = Session::from_page(PAGE, Instant::now()).unwrap();
        let anon = s.api_headers("UA", None);
        assert_eq!(anon["x-youtube-client-name"], "67");
        assert_eq!(anon["x-goog-visitor-id"], "VD%3D");
        assert!(anon.get("authorization").is_none());
        let jar = CookieJar::from_netscape(
            ".youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsid\n.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tx\n",
        );
        let authed = s.api_headers("UA", Some(&jar));
        let auth = authed["authorization"].to_str().unwrap();
        assert!(
            auth.starts_with("SAPISIDHASH ") && auth.ends_with("_u"),
            "{auth}"
        );
        assert_eq!(authed["x-goog-authuser"], "0");
        assert!(authed["cookie"].to_str().unwrap().contains("SAPISID=sid"));
    }

    #[test]
    fn player_ids() {
        assert_eq!(
            player_id("https://www.youtube.com/s/player/0229cd64/player_ias.vflset/en_US/base.js"),
            Some("0229cd64")
        );
        assert_eq!(player_id("/s/player/zz/base.js"), None);
        let iframe = r#"var scriptUrl = 'https:\/\/www.youtube.com\/s\/player\/0229cd64\/www-widgetapi.vflset\/www-widgetapi.js';"#;
        assert_eq!(
            player_id_from_iframe_api(iframe).as_deref(),
            Some("0229cd64")
        );
    }
}
