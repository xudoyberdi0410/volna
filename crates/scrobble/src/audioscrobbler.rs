//! The Audioscrobbler 2.0 API shared by Last.fm (`ui/js/lastfm.js`) and
//! Libre.fm (`ui/js/librefm.js`): md5 `api_sig`, desktop auth
//! (token → browser → session), now playing, batched scrobbles, love.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Service;
use crate::http::{Body, BoxFuture, Http, Request, Response};
use crate::service::Scrobbler;
use crate::{Error, Play, Track};

/// The key/secret the UI ships (`DEFAULT_API_KEY` in `lastfm.js`).
pub const LASTFM_API_KEY: &str = "85214f5abbc730e78770f27784b9bdf7";
pub const LASTFM_API_SECRET: &str = "2c2c37fd86739191860db810dd063292";
pub const LASTFM_API_URL: &str = "https://ws.audioscrobbler.com/2.0/";
pub const LASTFM_AUTH_URL: &str = "https://www.last.fm/api/auth/";

/// Libre.fm accepts any key; these are the UI's (`librefm.js`).
pub const LIBREFM_API_KEY: &str = "monochrome_music_app";
pub const LIBREFM_API_SECRET: &str = "monochrome_music_secret_2024";
pub const LIBREFM_API_URL: &str = "https://libre.fm/2.0/";
pub const LIBREFM_AUTH_URL: &str = "https://libre.fm/api/auth/";

/// `track.scrobble` takes at most 50 plays per call.
pub const MAX_BATCH: usize = 50;

/// One Audioscrobbler-compatible server with the app's credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub service: Service,
    pub api_url: String,
    pub auth_url: String,
    pub api_key: String,
    pub api_secret: String,
    /// Libre.fm gets the main artist without "feat. X"; Last.fm the full name.
    pub split_artist: bool,
}

impl Endpoint {
    /// Last.fm, with the user's own key/secret when both are given.
    pub fn lastfm(custom: Option<(&str, &str)>) -> Self {
        let (key, secret) = custom
            .filter(|(k, s)| !k.trim().is_empty() && !s.trim().is_empty())
            .map_or((LASTFM_API_KEY, LASTFM_API_SECRET), |(k, s)| {
                (k.trim(), s.trim())
            });
        Self {
            service: Service::LastFm,
            api_url: LASTFM_API_URL.into(),
            auth_url: LASTFM_AUTH_URL.into(),
            api_key: key.into(),
            api_secret: secret.into(),
            split_artist: false,
        }
    }

    pub fn librefm() -> Self {
        Self {
            service: Service::LibreFm,
            api_url: LIBREFM_API_URL.into(),
            auth_url: LIBREFM_AUTH_URL.into(),
            api_key: LIBREFM_API_KEY.into(),
            api_secret: LIBREFM_API_SECRET.into(),
            split_artist: true,
        }
    }

    fn artist(&self, t: &Track) -> String {
        if self.split_artist {
            t.main_artist()
        } else {
            t.first_artist().to_owned()
        }
    }

    /// The page the user opens to allow the app.
    pub fn auth_page(&self, token: &str) -> String {
        let mut u = url::Url::parse(&self.auth_url).expect("static auth url");
        u.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("token", token);
        u.into()
    }

    /// A signed POST: `method`, `api_key`, `params`, `sk`, `api_sig`, `format=json`.
    pub fn request(
        &self,
        method: &str,
        params: Vec<(String, String)>,
        sk: Option<&str>,
    ) -> Request {
        let mut fields = vec![
            ("method".to_owned(), method.to_owned()),
            ("api_key".to_owned(), self.api_key.clone()),
        ];
        fields.extend(params);
        if let Some(sk) = sk {
            fields.push(("sk".into(), sk.into()));
        }
        let sig = sign(&fields, &self.api_secret);
        fields.push(("api_sig".into(), sig));
        fields.push(("format".into(), "json".into()));
        Request::post(self.api_url.clone(), Body::Form(fields))
    }

    async fn call(
        &self,
        http: &dyn Http,
        method: &str,
        params: Vec<(String, String)>,
        sk: Option<&str>,
    ) -> Result<Value, Error> {
        parse(&http.send(self.request(method, params, sk)).await?)
    }

    /// `auth.getToken`.
    pub async fn get_token(&self, http: &dyn Http) -> Result<String, Error> {
        let v = self.call(http, "auth.getToken", vec![], None).await?;
        v.get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| Error::Server("no token in the answer".into()))
    }

    /// `auth.getSession` once the user allowed the token in the browser.
    pub async fn get_session(&self, http: &dyn Http, token: &str) -> Result<Session, Error> {
        let v = self
            .call(
                http,
                "auth.getSession",
                vec![("token".into(), token.into())],
                None,
            )
            .await?;
        serde_json::from_value(v.get("session").cloned().unwrap_or(Value::Null))
            .map_err(|_| Error::Auth("no session in the answer".into()))
    }

    pub fn now_playing_params(&self, t: &Track) -> Vec<(String, String)> {
        let mut p = vec![
            ("artist".into(), self.artist(t)),
            ("track".into(), t.title.clone()),
        ];
        push_extra(&mut p, t, "");
        p
    }

    /// `track.scrobble` fields for up to [`MAX_BATCH`] plays (`artist[i]`, …).
    pub fn scrobble_params(&self, plays: &[Play]) -> Vec<(String, String)> {
        let mut p = vec![];
        for (i, play) in plays.iter().enumerate() {
            let t = &play.track;
            p.push((format!("artist[{i}]"), self.artist(t)));
            p.push((format!("track[{i}]"), t.title.clone()));
            p.push((format!("timestamp[{i}]"), play.started_at.to_string()));
            push_extra(&mut p, t, &format!("[{i}]"));
        }
        p
    }

    pub fn love_params(&self, t: &Track) -> Vec<(String, String)> {
        vec![
            ("artist".into(), self.artist(t)),
            ("track".into(), t.title.clone()),
        ]
    }
}

fn push_extra(p: &mut Vec<(String, String)>, t: &Track, suffix: &str) {
    if let Some(album) = t.album.as_deref().filter(|a| !a.is_empty()) {
        p.push((format!("album{suffix}"), album.to_owned()));
    }
    if let Some(d) = t.duration_s() {
        p.push((format!("duration{suffix}"), d.to_string()));
    }
    if let Some(n) = t.track_number.filter(|n| *n > 0) {
        p.push((format!("trackNumber{suffix}"), n.to_string()));
    }
}

/// `api_sig`: md5 of the sorted `keyvalue` pairs (without `format` and
/// `callback`) followed by the secret, as `generateSignature` in the UI.
pub fn sign(fields: &[(String, String)], secret: &str) -> String {
    let sorted: BTreeMap<&str, &str> = fields
        .iter()
        .filter(|(k, _)| k != "format" && k != "callback")
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    format!("{:x}", md5::compute(s.as_bytes()))
}

/// The JSON answer, or the API error mapped to what the queue should do.
pub fn parse(resp: &Response) -> Result<Value, Error> {
    let json: Option<Value> = serde_json::from_str(&resp.body).ok();
    if let Some(code) = json
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_i64)
    {
        let msg = json
            .as_ref()
            .and_then(|v| v.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("API error");
        return Err(classify(code, &format!("{code}: {msg}")));
    }
    match json {
        Some(v) if resp.ok() => Ok(v),
        _ if resp.status >= 500 || resp.status == 429 || resp.ok() => Err(Error::Server(format!(
            "HTTP {}: {}",
            resp.status,
            resp.body.chars().take(200).collect::<String>()
        ))),
        _ => Err(crate::error::from_status(resp.status, &resp.body)),
    }
}

/// Last.fm error codes (<https://www.last.fm/api/errorcodes>).
fn classify(code: i64, msg: &str) -> Error {
    match code {
        // Auth failed, invalid key, invalid session, bad signature, token
        // not authorized / expired, suspended key.
        4 | 9 | 10 | 13 | 14 | 15 | 26 => Error::Auth(msg.into()),
        // Operation failed, service offline, temporary error, rate limit.
        8 | 11 | 16 | 29 => Error::Server(msg.into()),
        _ => Error::Rejected(msg.into()),
    }
}

/// A Last.fm / Libre.fm session (`lastfm-session` in the UI's localStorage).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub key: String,
    pub name: String,
}

/// Last.fm or Libre.fm with a session.
pub struct Audioscrobbler {
    pub endpoint: Endpoint,
    pub session_key: String,
}

impl Scrobbler for Audioscrobbler {
    fn service(&self) -> Service {
        self.endpoint.service
    }

    fn max_batch(&self) -> usize {
        MAX_BATCH
    }

    fn now_playing<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let p = self.endpoint.now_playing_params(track);
            self.endpoint
                .call(http, "track.updateNowPlaying", p, Some(&self.session_key))
                .await
                .map(drop)
        })
    }

    fn scrobble_batch<'a>(
        &'a self,
        http: &'a dyn Http,
        plays: &'a [Play],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let p = self.endpoint.scrobble_params(plays);
            let v = self
                .endpoint
                .call(http, "track.scrobble", p, Some(&self.session_key))
                .await?;
            let ignored = v
                .pointer("/scrobbles/@attr/ignored")
                .and_then(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok()))
                .unwrap_or(0);
            if ignored > 0 {
                tracing::info!(service = ?self.endpoint.service, ignored, "plays ignored by the service");
            }
            Ok(())
        })
    }

    fn love<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
        on: bool,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let method = if on { "track.love" } else { "track.unlove" };
            let p = self.endpoint.love_params(track);
            self.endpoint
                .call(http, method, p, Some(&self.session_key))
                .await
                .map(drop)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect()
    }

    #[test]
    fn signature_is_exact() {
        // md5("api_keykmethodauth.getTokens")
        assert_eq!(
            sign(
                &pairs(&[
                    ("method", "auth.getToken"),
                    ("api_key", "k"),
                    ("format", "json")
                ]),
                "s"
            ),
            "fcb68e4c03131d77e8851e889687a441"
        );
    }

    #[test]
    fn lastfm_scrobble_request() {
        let e = Endpoint::lastfm(None);
        let plays = [Play {
            track: Track {
                title: "Title".into(),
                artists: vec!["Artist".into(), "Other".into()],
                album: Some("Album".into()),
                duration: Some(200.7),
                ..Default::default()
            },
            started_at: 1_700_000_000,
        }];
        let r = e.request("track.scrobble", e.scrobble_params(&plays), Some("SK"));
        assert_eq!(r.url, "https://ws.audioscrobbler.com/2.0/");
        let f = r.form();
        assert_eq!(f["method"], "track.scrobble");
        assert_eq!(f["api_key"], LASTFM_API_KEY);
        assert_eq!(f["artist[0]"], "Artist");
        assert_eq!(f["timestamp[0]"], "1700000000");
        assert_eq!(f["duration[0]"], "200");
        assert_eq!(f["sk"], "SK");
        assert_eq!(f["format"], "json");
        // md5 of album[0]Album api_key… artist[0]Artist duration[0]200
        // methodtrack.scrobble skSK timestamp[0]1700000000 track[0]Title + secret.
        assert_eq!(f["api_sig"], "be8bcaff080227eaec50305289fa0d83");
    }

    #[test]
    fn librefm_session_request_and_split_artist() {
        let e = Endpoint::librefm();
        let r = e.request("auth.getSession", pairs(&[("token", "TOK")]), None);
        assert_eq!(r.url, "https://libre.fm/2.0/");
        assert_eq!(r.form()["api_sig"], "94debcfda5308dc0af79bca8971b16d5");
        assert_eq!(
            e.auth_page("TOK"),
            "https://libre.fm/api/auth/?api_key=monochrome_music_app&token=TOK"
        );
        let t = Track {
            title: "T".into(),
            artists: vec!["A feat. B".into()],
            ..Default::default()
        };
        assert_eq!(e.love_params(&t)[0].1, "A");
        assert_eq!(Endpoint::lastfm(None).love_params(&t)[0].1, "A feat. B");
    }

    #[test]
    fn custom_credentials_need_both() {
        assert_eq!(Endpoint::lastfm(Some(("key", ""))).api_key, LASTFM_API_KEY);
        let e = Endpoint::lastfm(Some(("key", "secret")));
        assert_eq!(
            (e.api_key.as_str(), e.api_secret.as_str()),
            ("key", "secret")
        );
        assert_eq!(
            e.auth_page("t"),
            "https://www.last.fm/api/auth/?api_key=key&token=t"
        );
    }

    #[test]
    fn batch_indexes_every_play() {
        let e = Endpoint::lastfm(None);
        let plays: Vec<Play> = (0..3)
            .map(|i| Play {
                track: Track {
                    title: format!("t{i}"),
                    artists: vec!["a".into()],
                    ..Default::default()
                },
                started_at: i,
            })
            .collect();
        let p = e.scrobble_params(&plays);
        assert!(p.contains(&("track[2]".into(), "t2".into())));
        assert!(p.contains(&("timestamp[1]".into(), "1".into())));
        assert!(!p.iter().any(|(k, _)| k.starts_with("album")));
    }

    #[test]
    fn errors_map_to_queue_actions() {
        let r = |status, body: &str| {
            parse(&Response {
                status,
                body: body.into(),
            })
        };
        assert!(r(200, r#"{"token":"x"}"#).is_ok());
        assert!(matches!(
            r(403, r#"{"error":9,"message":"Invalid session key"}"#),
            Err(Error::Auth(_))
        ));
        assert!(matches!(
            r(200, r#"{"error":29,"message":"Rate"}"#),
            Err(Error::Server(_))
        ));
        assert!(matches!(
            r(400, r#"{"error":6,"message":"bad"}"#),
            Err(Error::Rejected(_))
        ));
        assert!(matches!(r(502, "<html>"), Err(Error::Server(_))));
        assert!(matches!(r(200, "<html>"), Err(Error::Server(_))));
    }
}
