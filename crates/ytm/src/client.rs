//! HTTP layer: the `WEB_REMIX` session (client version + visitor data read
//! from the music.youtube.com shell) and the InnerTube POST helper.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use reqwest::header::{
    ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT,
};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::ytcfg::{self, YtCfg};

pub const ORIGIN_URL: &str = "https://music.youtube.com";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_NAME_ID: &str = "67";
/// Accept the EU cookie banner up front; without it EU exits (e.g. a VPS in
/// Finland) get redirected to consent.youtube.com.
const CONSENT_COOKIE: &str = "SOCS=CAI";
const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";

/// Tunables for [`YtmClient`].
#[derive(Debug, Clone)]
pub struct ClientOptions {
    /// UI language. Parsers rely on English labels for a few things (type
    /// labels such as "Single", artist shelf titles), so keep `en` unless
    /// you know what you are doing. Titles and names are never translated.
    pub hl: String,
    /// Content region (`UZ`, `RU`, …). `None` uses what YouTube detected for
    /// the exit IP.
    pub gl: Option<String>,
    pub user_agent: String,
    /// How long a bootstrapped session is reused before re-reading ytcfg.
    pub session_ttl: Duration,
}

impl Default for ClientOptions {
    fn default() -> Self {
        Self {
            hl: "en".into(),
            gl: None,
            user_agent: DEFAULT_UA.into(),
            session_ttl: Duration::from_secs(6 * 3600),
        }
    }
}

#[derive(Debug, Clone)]
struct Session {
    cfg: YtCfg,
    created: Instant,
    /// Built from the date because the shell could not be read.
    fallback: bool,
}

struct Inner {
    http: RwLock<reqwest::Client>,
    opts: ClientOptions,
    session: Mutex<Option<Arc<Session>>>,
}

/// Async YouTube Music InnerTube client (`WEB_REMIX`, logged out).
///
/// Cheap to clone; clones share the HTTP client and the cached session.
/// The `reqwest::Client` comes from the caller so the app-wide proxy applies.
#[derive(Clone)]
pub struct YtmClient {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for YtmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("YtmClient")
            .field("opts", &self.inner.opts)
            .finish_non_exhaustive()
    }
}

impl YtmClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_options(http, ClientOptions::default())
    }

    pub fn with_options(http: reqwest::Client, opts: ClientOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                http: RwLock::new(http),
                opts,
                session: Mutex::new(None),
            }),
        }
    }

    /// Swaps the HTTP client (e.g. after the proxy changed) and drops the
    /// cached session, since visitor data and region follow the exit IP.
    pub async fn set_http_client(&self, http: reqwest::Client) {
        *self.inner.http.write().unwrap_or_else(|e| e.into_inner()) = http;
        self.inner.session.lock().await.take();
    }

    fn http(&self) -> reqwest::Client {
        self.inner
            .http
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Client version currently used (bootstraps the session if needed).
    pub async fn client_version(&self) -> Result<String> {
        Ok(self.session().await?.cfg.client_version.clone())
    }

    async fn session(&self) -> Result<Arc<Session>> {
        let mut guard = self.inner.session.lock().await;
        if let Some(s) = guard.as_ref() {
            let ttl = if s.fallback {
                Duration::from_secs(300)
            } else {
                self.inner.opts.session_ttl
            };
            if s.created.elapsed() < ttl {
                return Ok(s.clone());
            }
        }
        let s = Arc::new(self.bootstrap().await?);
        *guard = Some(s.clone());
        Ok(s)
    }

    async fn invalidate(&self) {
        self.inner.session.lock().await.take();
    }

    /// Reads ytcfg from the HTML shell; on a parse failure falls back to a
    /// date-based client version. Network errors are returned as is.
    async fn bootstrap(&self) -> Result<Session> {
        let resp = self
            .http()
            .get(format!("{ORIGIN_URL}/"))
            .header(USER_AGENT, &self.inner.opts.user_agent)
            .header(ACCEPT_LANGUAGE, format!("{},en;q=0.8", self.inner.opts.hl))
            .header(COOKIE, CONSENT_COOKIE)
            .send()
            .await?;
        let status = resp.status();
        let html = resp.text().await?;
        if let Some(cfg) = ytcfg::parse(&html) {
            tracing::debug!(version = %cfg.client_version, gl = ?cfg.gl, "ytm session bootstrapped");
            return Ok(Session {
                cfg,
                created: Instant::now(),
                fallback: false,
            });
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        // Yesterday's date: today's build may not be rolled out everywhere.
        let version = ytcfg::fallback_client_version(now.saturating_sub(86_400));
        tracing::warn!(%status, %version, "ytcfg not found in music.youtube.com shell, using fallback");
        Ok(Session {
            cfg: YtCfg {
                client_version: version,
                visitor_data: None,
                api_key: None,
                gl: None,
                hl: None,
            },
            created: Instant::now(),
            fallback: true,
        })
    }

    fn context(&self, s: &Session) -> Value {
        let opts = &self.inner.opts;
        let mut client = json!({
            "clientName": CLIENT_NAME,
            "clientVersion": s.cfg.client_version,
            "hl": opts.hl,
        });
        if let Some(gl) = opts.gl.as_ref().or(s.cfg.gl.as_ref()) {
            client["gl"] = json!(gl);
        }
        if let Some(vd) = &s.cfg.visitor_data {
            client["visitorData"] = json!(vd);
        }
        json!({ "client": client, "user": {} })
    }

    fn headers(&self, s: &Session) -> HeaderMap {
        let mut h = HeaderMap::new();
        let mut put = |k: &'static str, v: &str| {
            if let Ok(v) = HeaderValue::from_str(v) {
                h.insert(k, v);
            }
        };
        put("x-youtube-client-name", CLIENT_NAME_ID);
        put("x-youtube-client-version", &s.cfg.client_version);
        if let Some(vd) = &s.cfg.visitor_data {
            put("x-goog-visitor-id", vd);
        }
        h.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        h.insert(ORIGIN, HeaderValue::from_static(ORIGIN_URL));
        h.insert(
            REFERER,
            HeaderValue::from_static("https://music.youtube.com/"),
        );
        if let Ok(v) = HeaderValue::from_str(&self.inner.opts.user_agent) {
            h.insert(USER_AGENT, v);
        }
        if let Ok(v) = HeaderValue::from_str(&self.inner.opts.hl) {
            h.insert(ACCEPT_LANGUAGE, v);
        }
        h
    }

    /// POSTs `body` (plus the client context) to `/youtubei/v1/{endpoint}`.
    /// A rejected request (400/401/403) re-bootstraps the session once,
    /// since a stale client version is the usual cause.
    pub(crate) async fn post(&self, endpoint: &str, body: Value) -> Result<Value> {
        let mut retried = false;
        loop {
            let session = self.session().await?;
            let mut payload = body.clone();
            if let Value::Object(m) = &mut payload {
                m.insert("context".into(), self.context(&session));
            }
            let resp = self
                .http()
                .post(format!(
                    "{ORIGIN_URL}/youtubei/v1/{endpoint}?prettyPrint=false"
                ))
                .headers(self.headers(&session))
                .json(&payload)
                .send()
                .await?;
            let status = resp.status();
            let bytes = resp.bytes().await?;
            if status.is_success() {
                return Ok(serde_json::from_slice(&bytes)?);
            }
            let retryable = matches!(
                status,
                StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            );
            if retryable && !retried {
                tracing::debug!(%status, endpoint, "InnerTube rejected request, refreshing session");
                retried = true;
                self.invalidate().await;
                continue;
            }
            return Err(Error::Status {
                endpoint: endpoint.to_owned(),
                status: status.as_u16(),
                message: error_message(&bytes),
            });
        }
    }
}

/// `error.message` of an InnerTube error body, or a snippet of the body.
fn error_message(body: &[u8]) -> String {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| String::from_utf8_lossy(&body[..body.len().min(200)]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_message_prefers_json() {
        assert_eq!(
            error_message(
                br#"{"error":{"code":400,"message":"Request contains an invalid argument."}}"#
            ),
            "Request contains an invalid argument."
        );
        assert_eq!(error_message(b"<html>nope</html>"), "<html>nope</html>");
    }
}
