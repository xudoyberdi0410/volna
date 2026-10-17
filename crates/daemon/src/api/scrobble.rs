//! Scrobbling commands over [`volna_scrobble::Scrobbling`] (phase 3e).
//!
//! The config (enabled services, sessions, tokens, percentage) lives in the
//! scrobble database `<app data dir>/scrobble.sqlite3` next to the offline
//! queue, not in `settings.json`: the crate owns its whole state and moves
//! into `volna-core` for mobile unchanged.
//!
//! Everything is off until the user turns a service on here or the legacy
//! import carries the old UI's keys over. Until then the old webview UI keeps
//! scrobbling from its own localStorage, so a play is never sent twice by
//! default (see `ui/js/multi-scrobbler.js` for the guard on its side).
//!
//! Commands (camelCase args):
//! - `scrobble_status` → [`volna_scrobble::Status`]
//! - `scrobble_set{service?, enabled?, token?, url?, loveOnLike?, percentage?,
//!   useCustomCredentials?, customApiKey?, customApiSecret?}` → status
//! - `scrobble_login_start{service}` → `{url}` to open in the browser
//! - `scrobble_login_finish{service}` → `{username}`
//! - `scrobble_logout{service}` → status
//! - `scrobble_flush` → status (retry the queue now)
//! - `scrobble_import_legacy` → status, from the keys `legacy_import` kept
//!   (`legacy_import` itself already applies them)
//!
//! Love on like: `library_like{kind: track}` (Flutter) and `wave_event{like}`
//! (old UI) call [`love_on_like`].

use std::collections::HashMap;

use serde_json::{Value, json};
use volna_scrobble::config::Patch;
use volna_scrobble::{Scrobbling, Service, Status, Track};

use crate::dispatch::{arg, opt, out};

fn service(a: &Value) -> Result<Service, String> {
    let s: String = arg(a, "service")?;
    Service::parse(&s).ok_or_else(|| format!("bad_args: unknown service `{s}`"))
}

/// Handles `scrobble_*`; `None` for other commands.
pub async fn call(s: &Scrobbling, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    Some(match cmd {
        "scrobble_status" => out(Ok(s.status())),
        "scrobble_set" => (|| {
            let svc = opt::<String>(a, "service")?
                .map(|n| {
                    Service::parse(&n).ok_or_else(|| format!("bad_args: unknown service `{n}`"))
                })
                .transpose()?;
            let patch: Patch =
                serde_json::from_value(a.clone()).map_err(|e| format!("bad_args: {e}"))?;
            out(s.set(svc, patch))
        })(),
        "scrobble_login_start" => match service(a) {
            Ok(svc) => out(s.login_start(svc).await.map(|url| json!({ "url": url }))),
            Err(e) => Err(e),
        },
        "scrobble_login_finish" => match service(a) {
            Ok(svc) => out(s
                .login_finish(svc)
                .await
                .map(|name| json!({ "username": name }))),
            Err(e) => Err(e),
        },
        "scrobble_logout" => service(a).and_then(|svc| out(s.logout(svc))),
        "scrobble_flush" => {
            s.flush_all().await;
            out(Ok(s.status()))
        }
        _ => return None,
    })
}

/// The old UI's localStorage scrobbler keys (raw strings, as
/// `localStorage.getItem` gives them) into the daemon config. For the
/// library's `legacy_import`; the key mapping is in `volna_scrobble::legacy`.
pub fn import_legacy(s: &Scrobbling, keys: &HashMap<String, String>) -> Result<Status, String> {
    s.import_legacy(keys)
}

/// `scrobble_import_legacy`: the scrobbler keys a `legacy_import` kept in
/// the library database (`legacy_kv`), e.g. from before this existed.
pub fn import_legacy_from_library(
    s: &Scrobbling,
    lib: &super::library::Library,
) -> Result<Value, String> {
    let mut keys = HashMap::new();
    for p in super::library::legacy::LOCAL_STORAGE_PREFIXES {
        let rows = lib
            .store()
            .legacy_prefixed(p)
            .map_err(|e| format!("internal: {e}"))?;
        keys.extend(rows);
    }
    if keys.is_empty() {
        return Err("not_found: no imported scrobbler settings".into());
    }
    out(s.import_legacy(&keys))
}

/// As [`import_legacy`], from a JSON object (values may be parsed JSON).
pub fn import_legacy_json(s: &Scrobbling, keys: &Value) -> Result<Status, String> {
    s.import_legacy(&volna_scrobble::legacy::keys_from_json(keys))
}

/// A library like (`on`) or unlike: love/unlove on the services with
/// "love on like" on. Runs in the background; never fails the like.
pub fn love_on_like(s: &Scrobbling, track: &volna_ytm::Track, on: bool) {
    let s = s.clone();
    let t = track_from_ytm(track);
    tokio::spawn(async move { s.love_on_like(t, on).await });
}

/// A YouTube Music track as the scrobblers see it.
pub fn track_from_ytm(t: &volna_ytm::Track) -> Track {
    Track {
        title: t.title.clone(),
        artists: t
            .artists
            .iter()
            .map(|a| a.name.clone())
            .filter(|n| !n.is_empty() && n != "Unknown artist")
            .collect(),
        album: t
            .album
            .as_ref()
            .map(|a| a.name.clone())
            .filter(|n| !n.is_empty()),
        duration: t.duration_s.map(f64::from),
        track_number: None,
        video_id: Some(t.video_id.clone()).filter(|v| !v.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn scrobbling() -> Scrobbling {
        let http = volna_scrobble::ReqwestHttp::new(reqwest::Client::new);
        Scrobbling::with_store(
            volna_scrobble::queue::Store::in_memory().unwrap(),
            Arc::new(http),
        )
    }

    #[tokio::test]
    async fn commands() {
        let s = scrobbling();
        let st = call(&s, "scrobble_status", &json!({}))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st["percentage"], 50);
        assert_eq!(st["services"][0]["service"], "lastfm");
        assert_eq!(st["services"][0]["active"], false);

        let st = call(
            &s,
            "scrobble_set",
            &json!({"service": "maloja", "enabled": true, "token": "k", "url": "https://m", "percentage": 70}),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(st["percentage"], 70);
        assert_eq!(st["services"][3]["active"], true);
        assert_eq!(st["services"][3]["hasToken"], true);
        assert!(st["services"][3].get("token").is_none(), "no secrets out");

        assert!(
            call(&s, "scrobble_set", &json!({"service": "nope"}))
                .await
                .unwrap()
                .is_err()
        );
        assert!(
            call(&s, "scrobble_login_start", &json!({"service": "maloja"}))
                .await
                .unwrap()
                .is_err()
        );
        let st = call(&s, "scrobble_logout", &json!({"service": "maloja"}))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(st["services"][3]["active"], false);
        assert!(call(&s, "other", &json!({})).await.is_none());
    }

    #[test]
    fn legacy_from_json() {
        let s = scrobbling();
        let st = import_legacy_json(
            &s,
            &json!({"listenbrainz-enabled": "true", "listenbrainz-token": "t"}),
        )
        .unwrap();
        assert!(st.services[2].active);
    }
}
