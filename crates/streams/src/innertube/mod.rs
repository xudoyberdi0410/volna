//! In-process resolver (feature `innertube`): the YouTube Music web client
//! (`WEB_REMIX`) the way yt-dlp's `web_music` client does it, without yt-dlp.
//!
//! 1. `ytcfg` from the `music.youtube.com` page (client version, visitor
//!    data, player URL, session ids), cached per exit for an hour.
//! 2. The player's `base.js` (main variant), cached per player id; its
//!    `signatureTimestamp` goes into the `player` request.
//! 3. `POST music.youtube.com/youtubei/v1/player` with the browser session's
//!    cookies and SAPISIDHASH when there is one.
//! 4. The audio format [`Quality`] asks for (Opus 251/250/249 or AAC 140/139).
//! 5. Its `s` signature and `n` parameter solved by the [`JsRuntime`]
//!    (yt-dlp's EJS solver), and a GVS PO token (`pot`) minted by BotGuard in
//!    the same runtime, bound to the video id or the session.
//!
//! All HTTP goes through Rust, on the exit the stream will be fetched from.

mod cookies;
#[cfg(feature = "innertube-deno")]
mod deno;
mod formats;

mod player;
mod pot;
mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

pub use crate::js::{JsCallback, JsError, JsFuture, JsRequest, JsRuntime, PRELUDE};
pub use cookies::{Cookie, CookieJar};
#[cfg(feature = "innertube-deno")]
pub use deno::DenoRuntime;

use crate::error::{Result, StreamError};
use crate::model::{Delivery, Quality, ResolvedStream, VideoId, url_expiry};
use formats::{PlayerResponse, final_url};
use player::{Player, PlayerCache, player_js_url};
use pot::PotProvider;
use session::{
    MUSIC_ORIGIN, Session, check_page, extract_ytcfg, player_id, player_id_from_iframe_api,
};

/// What the web clients send as User-Agent (yt-dlp's default for them).
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";
/// How long a fetched `ytcfg` is used.
const SESSION_TTL: Duration = Duration::from_secs(60 * 60);
const API_TIMEOUT: Duration = Duration::from_secs(30);
const PLAYER_JS_TIMEOUT: Duration = Duration::from_secs(90);
/// Pauses before re-trying a refused `player` request.
const RETRY_BACKOFF: [Duration; 2] = [Duration::from_millis(500), Duration::from_secs(3)];

/// Settings for [`InnerTube`].
#[derive(Debug, Clone)]
pub struct InnerTubeOptions {
    pub user_agent: String,
    /// Keeps `base.js` across runs (~3 MB per player version). `None`: memory only.
    pub cache_dir: Option<PathBuf>,
    /// Ask for a GVS PO token (`pot=`). Without one googlevideo answers 403
    /// for the web clients unless the account has Premium.
    pub po_token: bool,
    /// Wait out pre-roll ads before returning the URL, as yt-dlp does. Off by
    /// default: playback starts with the bytes that are already allowed.
    pub wait_for_ads: bool,
}

impl Default for InnerTubeOptions {
    fn default() -> Self {
        Self {
            user_agent: DEFAULT_USER_AGENT.into(),
            cache_dir: None,
            po_token: true,
            wait_for_ads: false,
        }
    }
}

/// The InnerTube [`Backend`](crate::Backend). Cheap to clone.
#[derive(Clone)]
pub struct InnerTube {
    inner: Arc<Inner>,
}

/// Cached ytcfg per exit (proxy URL), with the cookie generation it was fetched with.
type Sessions = HashMap<Option<String>, (u64, Arc<Session>)>;

struct Inner {
    js: Arc<dyn JsRuntime>,
    opts: InnerTubeOptions,
    cookies: RwLock<Option<Arc<CookieJar>>>,
    clients: Mutex<HashMap<Option<String>, reqwest::Client>>,
    /// Cached ytcfg per exit, tagged with the cookie generation it was fetched with.
    sessions: tokio::sync::Mutex<Sessions>,
    /// Bumped by `set_cookies`.
    cookie_generation: AtomicU64,
    players: PlayerCache,
    /// One `base.js` download at a time.
    player_fetch: tokio::sync::Mutex<()>,
    pot: PotProvider,
}

impl InnerTube {
    pub fn new(js: Arc<dyn JsRuntime>, opts: InnerTubeOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                js,
                players: PlayerCache::new(opts.cache_dir.clone()),
                opts,
                cookies: RwLock::new(None),
                clients: Mutex::default(),
                sessions: tokio::sync::Mutex::default(),
                cookie_generation: AtomicU64::new(0),
                player_fetch: tokio::sync::Mutex::default(),
                pot: PotProvider::default(),
            }),
        }
    }

    /// Uses a logged-in browser session (or none), from the next resolve on.
    /// The cached `ytcfg`, which depends on it, is fetched again.
    pub fn set_cookies(&self, jar: Option<CookieJar>) {
        let jar = jar.filter(|j| !j.is_empty()).map(Arc::new);
        *self
            .inner
            .cookies
            .write()
            .unwrap_or_else(|e| e.into_inner()) = jar;
        self.inner.cookie_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn cookies(&self) -> Option<Arc<CookieJar>> {
        self.inner
            .cookies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Resolves `id` through `proxy` (the exit the stream is fetched from).
    pub async fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> Result<ResolvedStream> {
        let started = Instant::now();
        let http = self.client(proxy)?;
        let cookies = self.cookies();
        let mut session = self
            .session(&http, proxy, cookies.as_deref(), false)
            .await?;
        let player = self.player(&http, &session).await?;

        // 403/429 is either a stale ytcfg (rotated visitor data, new client
        // version) or Google's "Sorry..." abuse page, which datacenter exits
        // get now and then: refresh the session, then back off once.
        let mut attempt = 0;
        let pr = loop {
            match self
                .player_request(&http, &session, &player, id, cookies.as_deref())
                .await
            {
                Err(StreamError::UpstreamStatus(status @ (400 | 403 | 429)))
                    if attempt < RETRY_BACKOFF.len() =>
                {
                    tracing::debug!(%id, status, attempt, "player request refused, retrying");
                    tokio::time::sleep(RETRY_BACKOFF[attempt]).await;
                    session = self.session(&http, proxy, cookies.as_deref(), true).await?;
                    attempt += 1;
                }
                Err(StreamError::UpstreamStatus(status @ (403 | 429))) => {
                    return Err(StreamError::BotCheck(format!(
                        "Google refused the player request (HTTP {status}); this exit is rate limited"
                    )));
                }
                other => break other?,
            }
        };
        pr.check(id.as_str())?;
        let format = pr.pick(quality)?;
        let (base_url, s, sp) = format
            .locate()
            .ok_or_else(|| StreamError::NoAudioStream("format without a URL".into()))?;
        let base =
            url::Url::parse(&base_url).map_err(|e| StreamError::InvalidUrl(e.to_string()))?;
        let n = crate::model::url_param(&base, "n");

        let rt: &dyn JsRuntime = &*self.inner.js;
        let solve = player.solve(rt, n.as_deref(), s.as_deref());
        let pot = async {
            if !self.inner.opts.po_token {
                return None;
            }
            let binding = pot_binding(&session, cookies.as_deref(), id);
            match self
                .inner
                .pot
                .mint(&http, &self.inner.opts.user_agent, proxy, rt, &binding)
                .await
            {
                Ok(t) => Some(t),
                Err(e) => {
                    // As yt-dlp: try without; googlevideo may still serve it.
                    tracing::warn!(%id, "no PO token: {e}");
                    None
                }
            }
        };
        let ((n_solved, sig_solved), pot) = {
            let (solved, pot) = tokio::join!(solve, pot);
            (solved?, pot)
        };
        if n.is_some() && n_solved.is_none() {
            return Err(StreamError::JsRuntime("n challenge not solved".into()));
        }
        let sig = match (&s, &sig_solved) {
            (Some(_), Some(solved)) => Some((sp.as_str(), solved.as_str())),
            (Some(_), None) => {
                return Err(StreamError::JsRuntime("signature not solved".into()));
            }
            _ => None,
        };
        let url = final_url(&base_url, sig, n_solved.as_deref(), pot.as_deref())?;

        let ads = pr.ad_seconds();
        if ads > 0 && self.inner.opts.wait_for_ads {
            tracing::debug!(%id, ads, "waiting out pre-roll ads");
            tokio::time::sleep(Duration::from_secs(ads)).await;
        }
        let now = SystemTime::now();
        let stream = ResolvedStream {
            delivery: Delivery::Url,
            format_id: format.itag.to_string(),
            mime: format.mime().to_owned(),
            codec: format.codecs().map(str::to_owned),
            bitrate_kbps: format.kbps(),
            filesize: format
                .content_length
                .as_deref()
                .and_then(|v| v.parse().ok())
                .or_else(|| crate::model::url_param(&url, "clen").and_then(|v| v.parse().ok())),
            expires_at: url_expiry(&url, now),
            user_agent: Some(self.inner.opts.user_agent.clone()),
            url: url.into(),
        };
        tracing::debug!(
            %id,
            itag = %stream.format_id,
            pot = pot.is_some(),
            ads,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved via InnerTube"
        );
        Ok(stream)
    }

    /// The signed-in account's display name (YouTube Music's
    /// `account/account_menu`); `None` without a signed-in session.
    pub async fn account_name(&self, proxy: Option<&str>) -> Result<Option<String>> {
        let Some(cookies) = self.cookies().filter(|c| c.is_logged_in()) else {
            return Ok(None);
        };
        let http = self.client(proxy)?;
        let session = self.session(&http, proxy, Some(&cookies), false).await?;
        let body = serde_json::json!({ "context": session.context });
        let resp = http
            .post(format!(
                "{MUSIC_ORIGIN}/youtubei/v1/account/account_menu?prettyPrint=false"
            ))
            .timeout(API_TIMEOUT)
            .headers(session.api_headers(&self.inner.opts.user_agent, Some(&cookies)))
            .body(body.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(StreamError::UpstreamStatus(resp.status().as_u16()));
        }
        let menu: serde_json::Value = resp.json().await?;
        Ok(account_name(&menu))
    }

    fn client(&self, proxy: Option<&str>) -> Result<reqwest::Client> {
        let key = proxy.map(str::to_owned);
        let mut clients = self.inner.clients.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = clients.get(&key) {
            return Ok(c.clone());
        }
        let builder = reqwest::Client::builder()
            .gzip(true)
            .brotli(true)
            .connect_timeout(Duration::from_secs(10));
        let builder = match proxy {
            Some(p) => builder
                .proxy(reqwest::Proxy::all(p).map_err(|e| StreamError::InvalidUrl(e.to_string()))?),
            None => builder.no_proxy(),
        };
        let client = builder.build()?;
        clients.insert(key, client.clone());
        Ok(client)
    }

    async fn session(
        &self,
        http: &reqwest::Client,
        proxy: Option<&str>,
        cookies: Option<&CookieJar>,
        refresh: bool,
    ) -> Result<Arc<Session>> {
        let key = proxy.map(str::to_owned);
        let generation = self.inner.cookie_generation.load(Ordering::Acquire);
        let mut sessions = self.inner.sessions.lock().await;
        if !refresh
            && let Some((g, s)) = sessions.get(&key)
            && *g == generation
            && s.fetched_at.elapsed() < SESSION_TTL
        {
            return Ok(s.clone());
        }
        let session = Arc::new(self.fetch_session(http, cookies).await?);
        sessions.insert(key, (generation, session.clone()));
        Ok(session)
    }

    async fn fetch_session(
        &self,
        http: &reqwest::Client,
        cookies: Option<&CookieJar>,
    ) -> Result<Session> {
        let mut req = http
            .get(format!("{MUSIC_ORIGIN}/"))
            .timeout(API_TIMEOUT)
            .header(reqwest::header::USER_AGENT, &self.inner.opts.user_agent)
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9");
        if let Some(c) = cookies.and_then(|c| c.header_for(session::MUSIC_HOST)) {
            req = req.header(reqwest::header::COOKIE, c);
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            return Err(StreamError::UpstreamStatus(resp.status().as_u16()));
        }
        let html = resp.text().await?;
        check_page(&html)?;
        let now = Instant::now();
        if !extract_ytcfg(&html).is_empty()
            && let Some(s) = Session::from_page(&html, now)
        {
            tracing::debug!(
                logged_in = s.logged_in,
                version = %s.client_version,
                player = ?s.player_url,
                bind_pot_to_video = s.bind_pot_to_video,
                "YouTube Music ytcfg"
            );
            return Ok(s);
        }
        tracing::warn!("no ytcfg on the YouTube Music page; using the built-in client context");
        Ok(Session::fallback(now))
    }

    async fn player(&self, http: &reqwest::Client, session: &Session) -> Result<Arc<Player>> {
        let id = match session.player_url.as_deref().and_then(player_id) {
            Some(id) => id.to_owned(),
            None => self.player_id_from_iframe(http).await?,
        };
        if let Some(p) = self.inner.players.get(&id) {
            return Ok(p);
        }
        let _one = self.inner.player_fetch.lock().await;
        if let Some(p) = self
            .inner
            .players
            .get(&id)
            .or_else(|| self.inner.players.load(&id))
        {
            return Ok(p);
        }
        let started = Instant::now();
        let code = http
            .get(player_js_url(&id))
            .timeout(PLAYER_JS_TIMEOUT)
            .header(reqwest::header::USER_AGENT, &self.inner.opts.user_agent)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        tracing::debug!(
            player = %id,
            bytes = code.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "downloaded player JS"
        );
        Ok(self.inner.players.insert(&id, code, true))
    }

    async fn player_id_from_iframe(&self, http: &reqwest::Client) -> Result<String> {
        let js = http
            .get("https://www.youtube.com/iframe_api")
            .timeout(API_TIMEOUT)
            .header(reqwest::header::USER_AGENT, &self.inner.opts.user_agent)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        player_id_from_iframe_api(&js)
            .ok_or_else(|| StreamError::ResolveFailed("cannot find the player version".into()))
    }

    async fn player_request(
        &self,
        http: &reqwest::Client,
        session: &Session,
        player: &Player,
        id: &VideoId,
        cookies: Option<&CookieJar>,
    ) -> Result<PlayerResponse> {
        let mut content = serde_json::json!({ "html5Preference": "HTML5_PREF_WANTS" });
        if let Some(sts) = session.sts.or(player.sts) {
            content["signatureTimestamp"] = sts.into();
        }
        let body = serde_json::json!({
            "context": session.context,
            "videoId": id.as_str(),
            "playbackContext": { "contentPlaybackContext": content },
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        let resp = http
            .post(format!(
                "{MUSIC_ORIGIN}/youtubei/v1/player?prettyPrint=false"
            ))
            .timeout(API_TIMEOUT)
            .headers(session.api_headers(&self.inner.opts.user_agent, cookies))
            .body(body.to_string())
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::debug!(%id, %status, "player request failed: {}", text.chars().take(300).collect::<String>());
            return Err(StreamError::UpstreamStatus(status.as_u16()));
        }
        Ok(resp.json().await?)
    }
}

/// What the GVS PO token is bound to: the video id under the web player's
/// content-binding experiment, else the account (data sync id) when logged
/// in, else the visitor.
fn pot_binding(session: &Session, cookies: Option<&CookieJar>, id: &VideoId) -> String {
    let logged_in = cookies.is_some_and(CookieJar::is_logged_in);
    if session.bind_pot_to_video {
        return id.as_str().to_owned();
    }
    if logged_in && let Some(d) = session.data_sync_id.as_deref() {
        return d.to_owned();
    }
    session
        .visitor_data
        .clone()
        .unwrap_or_else(|| id.as_str().to_owned())
}

/// `activeAccountHeaderRenderer.accountName` anywhere in an account menu.
fn account_name(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(h) = m.get("activeAccountHeaderRenderer") {
                let name = h
                    .pointer("/accountName/runs/0/text")
                    .or_else(|| h.pointer("/accountName/simpleText"));
                if let Some(name) = name.and_then(|n| n.as_str()) {
                    return Some(name.to_owned());
                }
            }
            m.values().find_map(account_name)
        }
        serde_json::Value::Array(a) => a.iter().find_map(account_name),
        _ => None,
    }
}

impl crate::Backend for InnerTube {
    fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> impl std::future::Future<Output = Result<ResolvedStream>> + Send {
        InnerTube::resolve(self, id, quality, proxy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_account_name() {
        let menu = serde_json::json!({"actions": [{"openPopupAction": {"popup": {
            "multiPageMenuRenderer": {"header": {"activeAccountHeaderRenderer": {
                "accountName": {"runs": [{"text": "Khudoberdi"}]}}}}}}}]});
        assert_eq!(account_name(&menu).as_deref(), Some("Khudoberdi"));
        assert_eq!(account_name(&serde_json::json!({"actions": []})), None);
    }

    #[test]
    fn pot_binding_follows_yt_dlp() {
        let id = VideoId::parse("dQw4w9WgXcQ").unwrap();
        let mut s = Session::fallback(Instant::now());
        s.visitor_data = Some("VD".into());
        s.data_sync_id = Some("123||".into());
        assert_eq!(pot_binding(&s, None, &id), "VD");
        let jar = CookieJar::from_netscape(
            ".youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\ts\n.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tx\n",
        );
        assert_eq!(pot_binding(&s, Some(&jar), &id), "123||");
        s.bind_pot_to_video = true;
        assert_eq!(pot_binding(&s, Some(&jar), &id), "dQw4w9WgXcQ");
    }
}
