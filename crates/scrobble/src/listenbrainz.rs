//! ListenBrainz (`ui/js/listenbrainz.js`): user token, optional own server,
//! `1/submit-listens` (`playing_now`, `single`, `import`), MBID lookup via
//! `1/metadata/lookup/` and love via `1/feedback/recording-feedback`.

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::config::Service;
use crate::error::from_status;
use crate::http::{Body, BoxFuture, Http, Request};
use crate::service::Scrobbler;
use crate::{Error, Play, Track};

pub const DEFAULT_URL: &str = "https://api.listenbrainz.org";
/// Listens per `import` call (the server allows 1000).
pub const MAX_BATCH: usize = 100;

/// MusicBrainz ids from a lookup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Mbids {
    pub recording_mbid: Option<String>,
    pub release_mbid: Option<String>,
    #[serde(default)]
    pub artist_mbids: Vec<String>,
}

pub struct ListenBrainz {
    /// Without a trailing `/1`.
    pub base: String,
    pub token: String,
}

/// The server root: `DEFAULT_URL` or the user's, with `/1` and `/` cut off
/// (`getApiUrl` in the UI).
pub fn base_url(custom: Option<&str>) -> String {
    let s = custom
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_URL);
    let s = s.trim_end_matches('/');
    let s = s.strip_suffix("/1").unwrap_or(s);
    s.trim_end_matches('/').to_owned()
}

impl ListenBrainz {
    pub fn new(custom_url: Option<&str>, token: &str) -> Self {
        Self {
            base: base_url(custom_url),
            token: token.trim().to_owned(),
        }
    }

    fn auth(&self, r: Request) -> Request {
        r.header("Authorization", format!("Token {}", self.token))
    }

    /// `track_metadata` of a listen (`_getMetadata`).
    pub fn metadata(t: &Track, mbids: Option<&Mbids>) -> Value {
        let mut info = Map::new();
        info.insert("submission_client".into(), "Volna".into());
        info.insert(
            "submission_client_version".into(),
            env!("CARGO_PKG_VERSION").into(),
        );
        if let Some(d) = t.duration_s() {
            info.insert("duration".into(), d.into());
        }
        if let Some(n) = t.track_number.filter(|n| *n > 0) {
            info.insert("track_number".into(), n.into());
        }
        if let Some(v) = &t.video_id {
            info.insert("music_service".into(), "music.youtube.com".into());
            info.insert(
                "origin_url".into(),
                format!("https://music.youtube.com/watch?v={v}").into(),
            );
        }
        if let Some(m) = mbids {
            if let Some(r) = &m.recording_mbid {
                info.insert("recording_mbid".into(), r.clone().into());
            }
            if let Some(r) = &m.release_mbid {
                info.insert("release_mbid".into(), r.clone().into());
            }
            if !m.artist_mbids.is_empty() {
                info.insert("artist_mbids".into(), m.artist_mbids.clone().into());
            }
        }
        let mut md = Map::new();
        md.insert("artist_name".into(), t.main_artist_lb().into());
        md.insert("track_name".into(), t.title.clone().into());
        if let Some(a) = t.album.as_deref().filter(|a| !a.is_empty()) {
            md.insert("release_name".into(), a.into());
        }
        md.insert("additional_info".into(), info.into());
        md.into()
    }

    /// `POST 1/submit-listens`; `listened_at` is `None` only for `playing_now`.
    pub fn submit_request(&self, listen_type: &str, listens: &[(Value, Option<i64>)]) -> Request {
        let payload: Vec<Value> = listens
            .iter()
            .map(|(md, at)| {
                let mut l = json!({ "track_metadata": md });
                if let Some(at) = at {
                    l["listened_at"] = (*at).into();
                }
                l
            })
            .collect();
        self.auth(Request::post(
            format!("{}/1/submit-listens", self.base),
            Body::Json(json!({ "listen_type": listen_type, "payload": payload })),
        ))
    }

    /// `GET 1/metadata/lookup/?recording_name=…&artist_name=…[&release_name=…]`.
    pub fn lookup_request(&self, t: &Track, with_album: bool) -> Request {
        let mut u = url::Url::parse(&format!("{}/1/metadata/lookup/", self.base))
            .unwrap_or_else(|_| url::Url::parse(DEFAULT_URL).expect("static url"));
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("recording_name", &t.title);
            q.append_pair("artist_name", &t.main_artist_lb());
            if with_album && let Some(a) = t.album.as_deref().filter(|a| !a.is_empty()) {
                q.append_pair("release_name", a);
            }
        }
        self.auth(Request::get(String::from(u)))
    }

    pub fn feedback_request(&self, mbid: &str, on: bool) -> Request {
        self.auth(Request::post(
            format!("{}/1/feedback/recording-feedback", self.base),
            Body::Json(json!({ "recording_mbid": mbid, "score": i32::from(on) })),
        ))
    }

    /// MBIDs for a track: with the album first, then without (as the UI).
    /// Failures are `None`: listens go out without ids.
    pub async fn lookup(&self, http: &dyn Http, t: &Track) -> Option<Mbids> {
        let has_album = t.album.as_deref().is_some_and(|a| !a.is_empty());
        for with_album in [true, false] {
            if with_album && !has_album {
                continue;
            }
            match http.send(self.lookup_request(t, with_album)).await {
                Ok(r) if r.ok() => {
                    let mut m: Mbids = serde_json::from_str(&r.body).ok()?;
                    m.recording_mbid.as_ref()?;
                    if !with_album {
                        m.release_mbid = None;
                    }
                    return Some(m);
                }
                Ok(r) => {
                    tracing::debug!(status = r.status, with_album, "ListenBrainz lookup failed")
                }
                Err(e) => {
                    tracing::debug!("ListenBrainz lookup: {e}");
                    return None;
                }
            }
        }
        None
    }

    async fn send(&self, http: &dyn Http, r: Request) -> Result<(), Error> {
        let resp = http.send(r).await?;
        if resp.ok() {
            Ok(())
        } else {
            Err(from_status(resp.status, &resp.body))
        }
    }
}

impl Scrobbler for ListenBrainz {
    fn service(&self) -> Service {
        Service::ListenBrainz
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
            let mbids = self.lookup(http, track).await;
            let md = Self::metadata(track, mbids.as_ref());
            self.send(http, self.submit_request("playing_now", &[(md, None)]))
                .await
        })
    }

    fn scrobble_batch<'a>(
        &'a self,
        http: &'a dyn Http,
        plays: &'a [Play],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let mut listens = Vec::with_capacity(plays.len());
            for p in plays {
                let mbids = self.lookup(http, &p.track).await;
                listens.push((Self::metadata(&p.track, mbids.as_ref()), Some(p.started_at)));
            }
            let kind = if listens.len() == 1 {
                "single"
            } else {
                "import"
            };
            self.send(http, self.submit_request(kind, &listens)).await
        })
    }

    fn love<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
        on: bool,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let Some(mbid) = self
                .lookup(http, track)
                .await
                .and_then(|m| m.recording_mbid)
            else {
                return Err(Error::Rejected("no recording MBID for this track".into()));
            };
            self.send(http, self.feedback_request(&mbid, on)).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Method;

    fn track() -> Track {
        Track {
            title: "Song".into(),
            artists: vec!["Artist feat.Guest".into()],
            album: Some("Album".into()),
            duration: Some(181.9),
            track_number: Some(3),
            video_id: Some("dQw4w9WgXcQ".into()),
        }
    }

    #[test]
    fn base_url_rules() {
        assert_eq!(base_url(None), DEFAULT_URL);
        assert_eq!(base_url(Some("  ")), DEFAULT_URL);
        assert_eq!(
            base_url(Some("https://lb.example/1/")),
            "https://lb.example"
        );
        assert_eq!(base_url(Some("https://lb.example/1")), "https://lb.example");
        assert_eq!(base_url(Some("https://lb.example/")), "https://lb.example");
    }

    #[test]
    fn submit_single_and_import() {
        let lb = ListenBrainz::new(None, " tok ");
        let md = ListenBrainz::metadata(
            &track(),
            Some(&Mbids {
                recording_mbid: Some("rec".into()),
                release_mbid: None,
                artist_mbids: vec!["art".into()],
            }),
        );
        let r = lb.submit_request("single", &[(md.clone(), Some(1_700_000_000))]);
        assert_eq!(r.method, Method::Post);
        assert_eq!(r.url, "https://api.listenbrainz.org/1/submit-listens");
        assert_eq!(r.headers, [("Authorization".into(), "Token tok".into())]);
        let Body::Json(b) = &r.body else { panic!() };
        assert_eq!(b["listen_type"], "single");
        let l = &b["payload"][0];
        assert_eq!(l["listened_at"], 1_700_000_000);
        let m = &l["track_metadata"];
        assert_eq!(m["artist_name"], "Artist");
        assert_eq!(m["track_name"], "Song");
        assert_eq!(m["release_name"], "Album");
        assert_eq!(m["additional_info"]["duration"], 181);
        assert_eq!(m["additional_info"]["track_number"], 3);
        assert_eq!(m["additional_info"]["recording_mbid"], "rec");
        assert_eq!(m["additional_info"]["artist_mbids"][0], "art");
        assert!(m["additional_info"].get("release_mbid").is_none());

        let now = lb.submit_request("playing_now", &[(md, None)]);
        let Body::Json(b) = &now.body else { panic!() };
        assert!(b["payload"][0].get("listened_at").is_none());
    }

    #[test]
    fn lookup_and_feedback() {
        let lb = ListenBrainz::new(Some("https://lb.example/1"), "t");
        assert_eq!(
            lb.lookup_request(&track(), true).url,
            "https://lb.example/1/metadata/lookup/?recording_name=Song&artist_name=Artist&release_name=Album"
        );
        assert_eq!(
            lb.lookup_request(&track(), false).url,
            "https://lb.example/1/metadata/lookup/?recording_name=Song&artist_name=Artist"
        );
        let f = lb.feedback_request("rec", false);
        assert_eq!(f.url, "https://lb.example/1/feedback/recording-feedback");
        assert_eq!(
            f.body,
            Body::Json(json!({"recording_mbid": "rec", "score": 0}))
        );
    }
}
