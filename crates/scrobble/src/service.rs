//! The fan-out (`multi-scrobbler.js`): one [`Scrobbler`] per configured
//! service, each fed from its own queue, none waiting on another.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::audioscrobbler::{Audioscrobbler, Endpoint, Session};
use crate::config::{Config, Patch, Service};
use crate::http::{BoxFuture, Http};
use crate::listenbrainz::ListenBrainz;
use crate::maloja::Maloja;
use crate::queue::{Item, QueueInfo, Queued, Store};
use crate::{Error, MAX_AGE_S, Play, Track, now_s};

/// One scrobbling service.
pub trait Scrobbler: Send + Sync {
    fn service(&self) -> Service;

    /// Plays per [`scrobble_batch`](Self::scrobble_batch) call.
    fn max_batch(&self) -> usize;

    fn now_playing<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
    ) -> BoxFuture<'a, Result<(), Error>>;

    /// Up to [`max_batch`](Self::max_batch) finished plays.
    fn scrobble_batch<'a>(
        &'a self,
        http: &'a dyn Http,
        plays: &'a [Play],
    ) -> BoxFuture<'a, Result<(), Error>>;

    /// Love (`on`) or unlove a track.
    fn love<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
        on: bool,
    ) -> BoxFuture<'a, Result<(), Error>>;

    fn can_love(&self) -> bool {
        true
    }

    /// One finished play that started at `started_at` (unix seconds).
    fn scrobble<'a>(
        &'a self,
        http: &'a dyn Http,
        track: &'a Track,
        started_at: i64,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let plays = [Play {
                track: track.clone(),
                started_at,
            }];
            self.scrobble_batch(http, &plays).await
        })
    }
}

/// The Last.fm rule the UI used (`shouldScrobble` in `player-journal.js`):
/// tracks over 30 s, played `min(fraction of the length, 4 min)`, not older
/// than the services accept. Times in seconds.
pub fn should_scrobble(
    played: f64,
    duration: Option<f64>,
    fraction: f64,
    ended_at: i64,
    now: i64,
) -> bool {
    let Some(duration) = duration.filter(|d| d.is_finite() && *d > 30.0) else {
        return false;
    };
    played >= (duration * fraction).min(240.0) && now - ended_at <= MAX_AGE_S
}

/// The scrobbler for `s` when it is enabled and has credentials.
fn scrobbler(c: &Config, s: Service) -> Option<Box<dyn Scrobbler>> {
    if !c.active(s) {
        return None;
    }
    Some(match s {
        Service::LastFm => Box::new(Audioscrobbler {
            endpoint: lastfm_endpoint(c),
            session_key: c.lastfm.session.as_ref()?.key.clone(),
        }),
        Service::LibreFm => Box::new(Audioscrobbler {
            endpoint: Endpoint::librefm(),
            session_key: c.librefm.session.as_ref()?.key.clone(),
        }),
        Service::ListenBrainz => Box::new(ListenBrainz::new(
            Some(&c.listenbrainz.url),
            &c.listenbrainz.token,
        )),
        Service::Maloja => Box::new(Maloja::new(&c.maloja.url, &c.maloja.token)),
    })
}

fn lastfm_endpoint(c: &Config) -> Endpoint {
    let l = &c.lastfm;
    Endpoint::lastfm(
        l.use_custom_credentials
            .then_some((l.custom_api_key.as_str(), l.custom_api_secret.as_str())),
    )
}

fn auth_endpoint(c: &Config, s: Service) -> Result<Endpoint, String> {
    match s {
        Service::LastFm => Ok(lastfm_endpoint(c)),
        Service::LibreFm => Ok(Endpoint::librefm()),
        other => Err(format!(
            "bad_args: {} uses a token, not a login",
            other.as_str()
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Share of the track to play before it is scrobbled (or 4 minutes).
    pub percentage: u8,
    pub services: Vec<ServiceStatus>,
}

/// One service for the settings screen; tokens and secrets are not echoed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub service: Service,
    pub enabled: bool,
    /// Enabled and has credentials: plays are sent.
    pub active: bool,
    pub can_love: bool,
    pub love_on_like: bool,
    /// Last.fm and Libre.fm log in through the browser.
    pub uses_login: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub login_pending: bool,
    pub has_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_custom_credentials: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_custom_api_secret: Option<bool>,
    pub queue: QueueInfo,
}

struct Inner {
    store: Mutex<Store>,
    http: Arc<dyn Http>,
    /// One flush at a time per service.
    flushing: [tokio::sync::Mutex<()>; 4],
}

/// Scrobbling for the whole app. Cheap to clone.
#[derive(Clone)]
pub struct Scrobbling {
    inner: Arc<Inner>,
}

fn db_err(e: rusqlite::Error) -> String {
    format!("internal: scrobble database: {e}")
}

fn idx(s: Service) -> usize {
    Service::ALL
        .iter()
        .position(|x| *x == s)
        .expect("known service")
}

impl Scrobbling {
    /// Opens (or creates) the database at `path`.
    pub fn open(path: &Path, http: Arc<dyn Http>) -> Result<Self, String> {
        Ok(Self::with_store(Store::open(path).map_err(db_err)?, http))
    }

    pub fn with_store(store: Store, http: Arc<dyn Http>) -> Self {
        Self {
            inner: Arc::new(Inner {
                store: Mutex::new(store),
                http,
                flushing: Default::default(),
            }),
        }
    }

    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.inner.store.lock().expect("scrobble store")
    }

    pub fn config(&self) -> Config {
        self.store().config()
    }

    /// Changes the config under the store lock.
    fn update<T>(&self, f: impl FnOnce(&mut Config) -> Result<T, String>) -> Result<T, String> {
        let store = self.store();
        let mut c = store.config();
        let out = f(&mut c)?;
        store.save_config(&c).map_err(db_err)?;
        Ok(out)
    }

    /// Whether any service would send something (the daemon logs it).
    pub fn any_active(&self) -> bool {
        let c = self.config();
        Service::ALL.iter().any(|s| c.active(*s))
    }

    pub fn status(&self) -> Status {
        let store = self.store();
        let c = store.config();
        let services = Service::ALL
            .iter()
            .map(|&s| {
                let queue = store.info(s).unwrap_or_default();
                let mut st = ServiceStatus {
                    service: s,
                    enabled: c.enabled(s),
                    active: c.active(s),
                    can_love: s != Service::Maloja,
                    love_on_like: c.love_on_like(s),
                    uses_login: matches!(s, Service::LastFm | Service::LibreFm),
                    username: None,
                    login_pending: false,
                    has_token: false,
                    url: None,
                    use_custom_credentials: None,
                    custom_api_key: None,
                    has_custom_api_secret: None,
                    queue,
                };
                match s {
                    Service::LastFm => {
                        let l = &c.lastfm;
                        st.username = l.session.as_ref().map(|x| x.name.clone());
                        st.login_pending = l.pending_token.is_some();
                        st.use_custom_credentials = Some(l.use_custom_credentials);
                        st.custom_api_key = Some(l.custom_api_key.clone());
                        st.has_custom_api_secret = Some(!l.custom_api_secret.is_empty());
                    }
                    Service::LibreFm => {
                        st.username = c.librefm.session.as_ref().map(|x| x.name.clone());
                        st.login_pending = c.librefm.pending_token.is_some();
                    }
                    Service::ListenBrainz => {
                        st.has_token = !c.listenbrainz.token.is_empty();
                        st.url = Some(c.listenbrainz.url.clone());
                    }
                    Service::Maloja => {
                        st.has_token = !c.maloja.token.is_empty();
                        st.url = Some(c.maloja.url.clone());
                    }
                }
                st
            })
            .collect();
        Status {
            percentage: c.percentage,
            services,
        }
    }

    /// `scrobble_set`: `service` `None` changes only the shared percentage.
    pub fn set(&self, service: Option<Service>, patch: Patch) -> Result<Status, String> {
        self.update(|c| {
            c.apply(service, patch)
                .map_err(|e| format!("bad_args: {e}"))
        })?;
        if let Some(s) = service {
            // New credentials: what waits for them can go now.
            let _ = self.store().retry_now(s);
            self.kick(Some(s));
        }
        Ok(self.status())
    }

    /// Applies the old UI's localStorage keys (see [`crate::legacy`]).
    pub fn import_legacy(&self, keys: &HashMap<String, String>) -> Result<Status, String> {
        self.update(|c| {
            crate::legacy::apply(c, keys);
            Ok(())
        })?;
        self.kick(None);
        Ok(self.status())
    }

    /// Step 1 of the desktop auth: the page to open in the browser.
    pub async fn login_start(&self, s: Service) -> Result<String, String> {
        let endpoint = auth_endpoint(&self.config(), s)?;
        let token = endpoint
            .get_token(self.inner.http.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        let url = endpoint.auth_page(&token);
        self.update(|c| {
            match s {
                Service::LastFm => c.lastfm.pending_token = Some(token),
                _ => c.librefm.pending_token = Some(token),
            }
            Ok(())
        })?;
        Ok(url)
    }

    /// Step 2, after the user allowed the app: stores the session and turns
    /// the service on. Returns the user name.
    pub async fn login_finish(&self, s: Service) -> Result<String, String> {
        let c = self.config();
        let endpoint = auth_endpoint(&c, s)?;
        let token = match s {
            Service::LastFm => c.lastfm.pending_token.clone(),
            _ => c.librefm.pending_token.clone(),
        }
        .ok_or("bad_args: call scrobble_login_start first")?;
        let session: Session = endpoint
            .get_session(self.inner.http.as_ref(), &token)
            .await
            .map_err(|e| e.to_string())?;
        let name = session.name.clone();
        self.update(|c| {
            match s {
                Service::LastFm => {
                    c.lastfm.session = Some(session);
                    c.lastfm.pending_token = None;
                    c.lastfm.enabled = true;
                }
                _ => {
                    c.librefm.session = Some(session);
                    c.librefm.pending_token = None;
                    c.librefm.enabled = true;
                }
            }
            Ok(())
        })?;
        let _ = self.store().retry_now(s);
        self.kick(Some(s));
        Ok(name)
    }

    /// Forgets the session or token and what was queued for the service.
    pub fn logout(&self, s: Service) -> Result<Status, String> {
        self.update(|c| {
            match s {
                Service::LastFm => {
                    c.lastfm.session = None;
                    c.lastfm.pending_token = None;
                }
                Service::LibreFm => {
                    c.librefm.session = None;
                    c.librefm.pending_token = None;
                }
                Service::ListenBrainz => c.listenbrainz.token.clear(),
                Service::Maloja => c.maloja.token.clear(),
            }
            Ok(())
        })?;
        self.store().clear(s).map_err(db_err)?;
        Ok(self.status())
    }

    /// "Now playing" on every active service at once; best effort.
    pub async fn now_playing(&self, track: &Track) {
        let c = self.config();
        let http = self.inner.http.as_ref();
        let one = |s: Service| {
            let sc = scrobbler(&c, s);
            async move {
                let Some(sc) = sc else { return };
                if let Err(e) = sc.now_playing(http, track).await {
                    tracing::debug!(service = s.as_str(), "now playing failed: {e}");
                }
            }
        };
        tokio::join!(
            one(Service::LastFm),
            one(Service::LibreFm),
            one(Service::ListenBrainz),
            one(Service::Maloja)
        );
    }

    /// A play the player finished (times in unix ms, as the journal has
    /// them; `played` and `duration` in seconds). Queues and sends it when
    /// it passes [`should_scrobble`]; returns whether it did.
    pub async fn play_ended(
        &self,
        track: Track,
        started_at_ms: f64,
        played: f64,
        duration: Option<f64>,
        ended_at_ms: f64,
    ) -> bool {
        let c = self.config();
        let duration = duration.or(track.duration);
        let ok = should_scrobble(
            played,
            duration,
            c.fraction(),
            (ended_at_ms / 1000.0) as i64,
            now_s(),
        );
        if ok {
            self.record_play(Play {
                track,
                started_at: (started_at_ms / 1000.0).floor() as i64,
            })
            .await;
        }
        ok
    }

    /// Queues a play for every active service and sends what is due.
    pub async fn record_play(&self, play: Play) {
        let item = Item::Scrobble {
            track: play.track,
            started_at: play.started_at,
        };
        self.enqueue(&item, |c, s| c.active(s));
        self.flush_all().await;
    }

    /// A like or unlike in the library: love on the services that have
    /// "love on like" on.
    pub async fn love_on_like(&self, track: Track, on: bool) {
        let item = Item::Love { track, on };
        self.enqueue(&item, |c, s| c.active(s) && c.love_on_like(s));
        self.flush_all().await;
    }

    /// Love or unlove on every active service that can.
    pub async fn love(&self, track: Track, on: bool) {
        let item = Item::Love { track, on };
        self.enqueue(&item, |c, s| c.active(s) && s != Service::Maloja);
        self.flush_all().await;
    }

    fn enqueue(&self, item: &Item, want: impl Fn(&Config, Service) -> bool) {
        let store = self.store();
        let c = store.config();
        let now = now_s();
        for s in Service::ALL {
            if want(&c, s)
                && let Err(e) = store.push(s, item, now)
            {
                tracing::warn!(service = s.as_str(), "cannot queue: {e}");
            }
        }
    }

    /// Flushes `only` (or every service) in the background.
    fn kick(&self, only: Option<Service>) {
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let this = self.clone();
        rt.spawn(async move {
            match only {
                Some(s) => drop(this.flush(s).await),
                None => this.flush_all().await,
            }
        });
    }

    /// Sends what is due on every service, side by side.
    pub async fn flush_all(&self) {
        // Errors stay in each service's queue (`status`).
        let _ = tokio::join!(
            self.flush(Service::LastFm),
            self.flush(Service::LibreFm),
            self.flush(Service::ListenBrainz),
            self.flush(Service::Maloja)
        );
    }

    /// Sends what is due for one service, batching scrobbles. Stops at the
    /// first error that should be retried; returns how many rows left the queue.
    pub async fn flush(&self, s: Service) -> Result<usize, Error> {
        let _one = self.inner.flushing[idx(s)].lock().await;
        let http = self.inner.http.as_ref();
        let mut gone = 0;
        // After a refused batch, find the bad play by sending one at a time.
        let mut single = false;
        loop {
            let Some(sc) = scrobbler(&self.config(), s) else {
                return Ok(gone);
            };
            let now = now_s();
            let due = self
                .store()
                .due(s, now, sc.max_batch().max(1))
                .map_err(|e| Error::Server(db_err(e)))?;
            let Some(first) = due.first() else {
                return Ok(gone);
            };
            let (ids, result) = match &first.item {
                Item::Love { track, on } => {
                    let r = if sc.can_love() {
                        sc.love(http, track, *on).await
                    } else {
                        Err(Error::Rejected("no love on this service".into()))
                    };
                    (vec![first.id], r)
                }
                Item::Scrobble { .. } => {
                    let limit = if single { 1 } else { sc.max_batch() };
                    let batch: Vec<&Queued> = due
                        .iter()
                        .take_while(|q| matches!(q.item, Item::Scrobble { .. }))
                        .take(limit)
                        .collect();
                    let plays: Vec<Play> = batch
                        .iter()
                        .filter_map(|q| match &q.item {
                            Item::Scrobble { track, started_at } => Some(Play {
                                track: track.clone(),
                                started_at: *started_at,
                            }),
                            Item::Love { .. } => None,
                        })
                        .collect();
                    let r = sc.scrobble_batch(http, &plays).await;
                    (batch.iter().map(|q| q.id).collect(), r)
                }
            };
            match result {
                Ok(()) => {
                    self.store()
                        .done(&ids)
                        .map_err(|e| Error::Server(db_err(e)))?;
                    gone += ids.len();
                }
                Err(Error::Rejected(m)) if ids.len() > 1 && !single => {
                    tracing::info!(
                        service = s.as_str(),
                        "batch refused ({m}); retrying one by one"
                    );
                    single = true;
                }
                Err(e) if !e.keep() => {
                    tracing::warn!(service = s.as_str(), "dropped from the queue: {e}");
                    self.store()
                        .done(&ids)
                        .map_err(|e| Error::Server(db_err(e)))?;
                    gone += ids.len();
                }
                Err(e) => {
                    tracing::info!(service = s.as_str(), "will retry: {e}");
                    let _ = self.store().failed(&ids, now, &e.to_string());
                    return Err(e);
                }
            }
        }
    }

    /// Retries the queue every `every` (and once right away).
    pub fn spawn_retry(&self, every: Duration) -> tokio::task::JoinHandle<()> {
        let this = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                tick.tick().await;
                this.flush_all().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};

    #[test]
    fn scrobble_threshold() {
        let now = 2_000_000_000;
        // Over 30 s only.
        assert!(!should_scrobble(30.0, Some(30.0), 0.5, now, now));
        assert!(!should_scrobble(100.0, None, 0.5, now, now));
        // Half of 200 s.
        assert!(!should_scrobble(99.0, Some(200.0), 0.5, now, now));
        assert!(should_scrobble(100.0, Some(200.0), 0.5, now, now));
        // 4 minutes is always enough.
        assert!(should_scrobble(240.0, Some(3600.0), 0.9, now, now));
        assert!(!should_scrobble(239.0, Some(3600.0), 0.9, now, now));
        // Too old for the services.
        assert!(!should_scrobble(
            200.0,
            Some(200.0),
            0.5,
            now - MAX_AGE_S - 1,
            now
        ));
    }

    /// Answers by URL prefix; records every request.
    #[derive(Default)]
    struct Fake {
        routes: Mutex<Vec<(String, u16, String)>>,
        seen: Mutex<Vec<Request>>,
    }

    impl Fake {
        fn route(&self, prefix: &str, status: u16, body: &str) {
            self.routes
                .lock()
                .unwrap()
                .insert(0, (prefix.into(), status, body.into()));
        }
        fn seen(&self, prefix: &str) -> Vec<Request> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.url.starts_with(prefix))
                .cloned()
                .collect()
        }
    }

    impl Http for Fake {
        fn send(&self, req: Request) -> BoxFuture<'_, Result<Response, Error>> {
            let hit = self
                .routes
                .lock()
                .unwrap()
                .iter()
                .find(|(p, _, _)| req.url.starts_with(p.as_str()))
                .cloned();
            self.seen.lock().unwrap().push(req);
            Box::pin(async move {
                match hit {
                    Some((_, 0, _)) | None => Err(Error::Network("down".into())),
                    Some((_, status, body)) => Ok(Response { status, body }),
                }
            })
        }
    }

    fn setup() -> (Scrobbling, Arc<Fake>) {
        let fake = Arc::new(Fake::default());
        let s = Scrobbling::with_store(Store::in_memory().unwrap(), fake.clone());
        s.update(|c| {
            c.lastfm.enabled = true;
            c.lastfm.session = Some(Session {
                key: "SK".into(),
                name: "me".into(),
            });
            c.lastfm.love_on_like = true;
            c.listenbrainz.enabled = true;
            c.listenbrainz.token = "tok".into();
            c.maloja.enabled = true;
            c.maloja.token = "key".into();
            c.maloja.url = "https://maloja.test".into();
            Ok(())
        })
        .unwrap();
        (s, fake)
    }

    fn play(i: i64) -> Play {
        Play {
            track: Track {
                title: format!("t{i}"),
                artists: vec!["a".into()],
                duration: Some(200.0),
                ..Default::default()
            },
            started_at: now_s() - 1000 + i,
        }
    }

    const LASTFM: &str = "https://ws.audioscrobbler.com/";
    const LB: &str = "https://api.listenbrainz.org/";
    const MALOJA: &str = "https://maloja.test/";

    #[tokio::test]
    async fn one_service_down_does_not_block_the_others() {
        let (s, fake) = setup();
        fake.route(LASTFM, 0, ""); // offline
        fake.route(LB, 200, r#"{"status":"ok"}"#);
        fake.route(&format!("{LB}1/metadata/lookup/"), 200, "{}");
        fake.route(MALOJA, 200, "{}");
        for i in 0..3 {
            s.record_play(play(i)).await;
        }
        let st = s.status();
        let q = |svc: Service| {
            st.services
                .iter()
                .find(|x| x.service == svc)
                .unwrap()
                .queue
                .clone()
        };
        assert_eq!(
            q(Service::LastFm).pending,
            3,
            "Last.fm keeps them for later"
        );
        assert!(q(Service::LastFm).last_error.unwrap().contains("network"));
        assert_eq!(q(Service::ListenBrainz).pending, 0);
        assert_eq!(q(Service::Maloja).pending, 0);
        assert_eq!(fake.seen(&format!("{LB}1/submit-listens")).len(), 3);
        assert_eq!(fake.seen(MALOJA).len(), 3);

        // Back online: everything waiting goes in one batch once due.
        fake.route(
            LASTFM,
            200,
            r#"{"scrobbles":{"@attr":{"accepted":3,"ignored":0}}}"#,
        );
        s.store().retry_now(Service::LastFm).unwrap();
        let before = fake.seen(LASTFM).len();
        assert_eq!(s.flush(Service::LastFm).await, Ok(3));
        let sent = fake.seen(LASTFM);
        assert_eq!(sent.len(), before + 1, "one batched call");
        let f = sent.last().unwrap().form();
        assert_eq!(f["method"], "track.scrobble");
        assert_eq!(f["track[2]"], "t2");
    }

    #[tokio::test]
    async fn refused_batch_is_split_and_bad_play_dropped() {
        let (s, fake) = setup();
        s.update(|c| {
            c.listenbrainz.enabled = false;
            c.maloja.enabled = false;
            Ok(())
        })
        .unwrap();
        fake.route(LASTFM, 0, "");
        for i in 0..2 {
            s.record_play(play(i)).await;
        }
        fake.route(LASTFM, 400, r#"{"error":6,"message":"Invalid parameters"}"#);
        s.store().retry_now(Service::LastFm).unwrap();
        assert_eq!(s.flush(Service::LastFm).await, Ok(2));
        let calls = fake.seen(LASTFM);
        // 2 offline singles, then the batch, then one by one.
        assert_eq!(calls.len(), 2 + 1 + 2);
        assert!(calls[3].form().contains_key("track[0]"));
        assert!(!calls[3].form().contains_key("track[1]"));
    }

    #[tokio::test]
    async fn love_on_like_only_where_enabled() {
        let (s, fake) = setup();
        fake.route(LASTFM, 200, "{}");
        s.love_on_like(play(0).track, true).await;
        let calls = fake.seen(LASTFM);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].form()["method"], "track.love");
        assert!(
            fake.seen(LB).is_empty(),
            "ListenBrainz has love on like off"
        );
        assert!(fake.seen(MALOJA).is_empty());
        s.love_on_like(play(0).track, false).await;
        assert_eq!(fake.seen(LASTFM)[1].form()["method"], "track.unlove");
    }

    #[tokio::test]
    async fn auth_error_keeps_the_play() {
        let (s, fake) = setup();
        fake.route(
            LASTFM,
            403,
            r#"{"error":9,"message":"Invalid session key"}"#,
        );
        fake.route(LB, 401, "bad token");
        fake.route(MALOJA, 200, "{}");
        s.record_play(play(0)).await;
        let st = s.status();
        let pending = |svc: Service| {
            st.services
                .iter()
                .find(|x| x.service == svc)
                .unwrap()
                .queue
                .pending
        };
        assert_eq!(pending(Service::LastFm), 1);
        assert_eq!(pending(Service::ListenBrainz), 1);
        assert_eq!(pending(Service::Maloja), 0);
        // Logging out forgets the queue.
        s.logout(Service::LastFm).unwrap();
        assert_eq!(s.store().info(Service::LastFm).unwrap().pending, 0);
    }

    #[tokio::test]
    async fn disabled_by_default_sends_nothing() {
        let fake = Arc::new(Fake::default());
        let s = Scrobbling::with_store(Store::in_memory().unwrap(), fake.clone());
        s.record_play(play(0)).await;
        s.now_playing(&play(0).track).await;
        assert!(
            !s.play_ended(play(0).track, 0.0, 10.0, Some(200.0), 0.0)
                .await
        );
        assert!(fake.seen.lock().unwrap().is_empty());
        assert!(!s.any_active());
    }

    #[tokio::test]
    async fn desktop_login() {
        let fake = Arc::new(Fake::default());
        let s = Scrobbling::with_store(Store::in_memory().unwrap(), fake.clone());
        fake.route(LASTFM, 200, r#"{"token":"TOK"}"#);
        let url = s.login_start(Service::LastFm).await.unwrap();
        assert_eq!(
            url,
            format!(
                "https://www.last.fm/api/auth/?api_key={}&token=TOK",
                crate::audioscrobbler::LASTFM_API_KEY
            )
        );
        fake.route(
            LASTFM,
            200,
            r#"{"session":{"name":"me","key":"SK","subscriber":0}}"#,
        );
        assert_eq!(s.login_finish(Service::LastFm).await.unwrap(), "me");
        let c = s.config();
        assert!(c.active(Service::LastFm));
        assert_eq!(c.lastfm.pending_token, None);
        assert!(s.login_start(Service::Maloja).await.is_err());
    }
}
