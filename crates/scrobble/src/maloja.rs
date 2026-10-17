//! Maloja (`ui/js/maloja.js`): the user's server and API key,
//! `apis/mlj_1/newscrobble` with a fallback to `apis/native/newscrobble`.
//! No "now playing", no love.

use crate::config::Service;
use crate::error::from_status;
use crate::http::{Body, BoxFuture, Http, Request};
use crate::service::Scrobbler;
use crate::{Error, Play, Track};

pub struct Maloja {
    /// Without a trailing `/`.
    pub base: String,
    pub key: String,
}

impl Maloja {
    pub fn new(url: &str, key: &str) -> Self {
        Self {
            base: url.trim().trim_end_matches('/').to_owned(),
            key: key.trim().to_owned(),
        }
    }

    pub fn form(&self, t: &Track, time: Option<i64>) -> Vec<(String, String)> {
        let mut f = vec![
            ("artist".to_owned(), t.main_artist()),
            ("title".to_owned(), t.title.clone()),
            ("key".to_owned(), self.key.clone()),
        ];
        if let Some(a) = t.album.as_deref().filter(|a| !a.is_empty()) {
            f.push(("album".into(), a.into()));
        }
        if let Some(d) = t.duration_s() {
            f.push(("duration".into(), d.to_string()));
        }
        if let Some(n) = t.track_number.filter(|n| *n > 0) {
            f.push(("track_number".into(), n.to_string()));
        }
        if let Some(time) = time {
            f.push(("time".into(), time.to_string()));
        }
        f
    }

    /// `apis/mlj_1/newscrobble`, or the old `apis/native/newscrobble`.
    pub fn request(&self, t: &Track, time: Option<i64>, native: bool) -> Request {
        let api = if native { "native" } else { "mlj_1" };
        Request::post(
            format!("{}/apis/{api}/newscrobble", self.base),
            Body::Form(self.form(t, time)),
        )
    }

    pub async fn submit(&self, http: &dyn Http, t: &Track, time: i64) -> Result<(), Error> {
        let resp = http.send(self.request(t, Some(time), false)).await?;
        if resp.ok() {
            return Ok(());
        }
        tracing::debug!(status = resp.status, "Maloja mlj_1 failed, trying native");
        let resp = http.send(self.request(t, Some(time), true)).await?;
        if resp.ok() {
            Ok(())
        } else {
            Err(from_status(resp.status, &resp.body))
        }
    }
}

impl Scrobbler for Maloja {
    fn service(&self) -> Service {
        Service::Maloja
    }

    fn max_batch(&self) -> usize {
        1
    }

    fn now_playing<'a>(
        &'a self,
        _: &'a dyn Http,
        _: &'a Track,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }

    fn scrobble_batch<'a>(
        &'a self,
        http: &'a dyn Http,
        plays: &'a [Play],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            for p in plays {
                self.submit(http, &p.track, p.started_at).await?;
            }
            Ok(())
        })
    }

    fn love<'a>(
        &'a self,
        _: &'a dyn Http,
        _: &'a Track,
        _: bool,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Err(Error::Rejected("Maloja has no love".into())) })
    }

    fn can_love(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newscrobble_request() {
        let m = Maloja::new("https://maloja.example/ ", "KEY");
        let t = Track {
            title: "Song".into(),
            artists: vec!["A & B".into()],
            album: Some("Al".into()),
            duration: Some(240.0),
            ..Default::default()
        };
        let r = m.request(&t, Some(1_700_000_000), false);
        assert_eq!(r.url, "https://maloja.example/apis/mlj_1/newscrobble");
        assert_eq!(
            r.body,
            Body::Form(vec![
                ("artist".into(), "A".into()),
                ("title".into(), "Song".into()),
                ("key".into(), "KEY".into()),
                ("album".into(), "Al".into()),
                ("duration".into(), "240".into()),
                ("time".into(), "1700000000".into()),
            ])
        );
        assert_eq!(
            m.request(&t, None, true).url,
            "https://maloja.example/apis/native/newscrobble"
        );
    }
}
