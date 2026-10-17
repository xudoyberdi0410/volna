//! `volna://api/<endpoint>`: a hifi-api emulator backed by YouTube Music.
//!
//! The Monochrome UI was written against hifi-api (a TIDAL proxy). Instead
//! of rewriting every page, Rust answers the endpoints it calls with the
//! JSON shapes it expects, filled from [`volna_ytm`]. The contract — which
//! endpoint returns which envelope and which fields matter — is
//! `docs/hifi-api-contract.md`; the conversions live in [`shape`].
//!
//! Extra endpoints the patched UI uses: `/home/`, `/explore/`,
//! `/explore/genre/`, `/lyrics/`, `/search/suggest/` and `/proxy/` (an
//! allowlisted GET relay for the lyrics providers, so they also go through
//! the app's proxy).

mod cache;
mod endpoints;
#[cfg(test)]
mod live;
mod proxy;
pub mod shape;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use http::{Response, StatusCode, header};
use volna_net::NetManager;
use volna_ytm::{Album, Artist, Playlist, Track, YtmClient};

use crate::protocol::{Target, json_error, with_cors};
use cache::TtlCache;

/// Errors of one API call, mapped to the status codes the UI expects
/// (contract §1.1): 404 unknown id, 502 upstream failure.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("YouTube Music request failed: {0}")]
    Upstream(String),
}

impl ApiError {
    fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Upstream(_) => StatusCode::BAD_GATEWAY,
        }
    }
}

impl From<volna_ytm::Error> for ApiError {
    fn from(e: volna_ytm::Error) -> Self {
        match e {
            volna_ytm::Error::NotFound(what) => ApiError::NotFound(what),
            other => ApiError::Upstream(other.to_string()),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Parsed query string.
#[derive(Debug, Default)]
pub struct Query(Vec<(String, String)>);

impl Query {
    pub fn parse(raw: Option<&str>) -> Self {
        Self(
            raw.map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn usize(&self, key: &str) -> Option<usize> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// A YouTube id from `key`: `[A-Za-z0-9_-]`, at most 128 chars.
    pub fn id(&self, key: &str) -> ApiResult<&str> {
        let id = self
            .get(key)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::BadRequest(format!("missing {key}=")))?;
        if id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(ApiError::BadRequest(format!("invalid id {id:?}")));
        }
        Ok(id)
    }

    /// Stable cache key: the pairs sorted by key.
    fn cache_key(&self, path: &str) -> String {
        let mut pairs: Vec<_> = self.0.iter().collect();
        pairs.sort();
        let qs = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        format!("{path}?{qs}")
    }
}

/// Byte budget of the response cache. A page is 5–100 KB of JSON, so this
/// holds a long browsing session while capping a runaway one.
const RESPONSE_CACHE_BYTES: usize = 8 * 1024 * 1024;

/// The emulator: one YouTube Music client plus short-lived caches.
pub struct ApiEmu {
    ytm: YtmClient,
    net: Arc<NetManager>,
    /// Final JSON per request (path + sorted query), already serialised:
    /// a `serde_json::Value` tree takes several times the bytes of its text,
    /// and a hit needs the bytes anyway.
    responses: TtlCache<String, Arc<[u8]>>,
    /// Pages reused by several endpoints.
    artists: TtlCache<String, Arc<Artist>>,
    albums: TtlCache<String, Arc<Album>>,
    /// Radio queues (seed + about 50 tracks) by mix id.
    radios: TtlCache<String, Arc<Vec<Track>>>,
    /// Partially loaded playlists, extended as the UI pages through them.
    playlists: TtlCache<String, Arc<tokio::sync::Mutex<Playlist>>>,
    /// Loopback audio prefix, for `/track/` manifests.
    stream_base: OnceLock<String>,
}

impl ApiEmu {
    pub fn new(net: Arc<NetManager>) -> Self {
        let min = |m: u64| Duration::from_secs(60 * m);
        Self {
            ytm: YtmClient::new(net.client()),
            net,
            responses: TtlCache::with_budget(512, min(10), RESPONSE_CACHE_BYTES, |b| b.len()),
            artists: TtlCache::new(128, min(15)),
            albums: TtlCache::new(256, min(30)),
            radios: TtlCache::new(128, min(10)),
            playlists: TtlCache::new(64, min(15)),
            stream_base: OnceLock::new(),
        }
    }

    /// Set once the loopback audio server is listening.
    pub fn set_stream_base(&self, base: String) {
        let _ = self.stream_base.set(base);
    }

    /// A handle on the same YouTube Music client (shared session and HTTP
    /// client, so [`Self::refresh_http_client`] reaches it too).
    pub fn ytm(&self) -> YtmClient {
        self.ytm.clone()
    }

    /// Call after `NetManager::apply`: InnerTube must use the new route.
    pub async fn refresh_http_client(&self) {
        self.ytm.set_http_client(self.net.client()).await;
    }

    /// Answers one `volna://api/...` request.
    pub async fn handle(&self, target: &Target) -> Response<Vec<u8>> {
        let path = normalise_path(&target.path);
        let query = Query::parse(target.query.as_deref());

        if path == "/proxy" {
            return proxy::relay(&self.net, &query).await;
        }

        let key = query.cache_key(&path);
        if let Some(hit) = self.responses.get(&key) {
            return json_bytes_response(hit.to_vec());
        }
        let started = std::time::Instant::now();
        match self.dispatch(&path, &query).await {
            Ok(value) => {
                tracing::debug!(%key, ms = started.elapsed().as_millis() as u64, "api");
                let body =
                    serde_json::to_vec(&value).expect("a serde_json::Value always serialises");
                drop(value);
                self.responses.insert(key, Arc::from(body.as_slice()));
                json_bytes_response(body)
            }
            Err(e) => {
                tracing::warn!(%key, "api failed: {e}");
                json_error(e.status(), &e.to_string())
            }
        }
    }
}

/// `/album/similar/` → `/album/similar`; `/` stays `/`.
fn normalise_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn json_bytes_response(body: Vec<u8>) -> Response<Vec<u8>> {
    with_cors(Response::builder().status(StatusCode::OK))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("static response headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_normalised() {
        assert_eq!(normalise_path("/album/"), "/album");
        assert_eq!(normalise_path("/album"), "/album");
        assert_eq!(normalise_path("/artist/similar/"), "/artist/similar");
        assert_eq!(normalise_path("/"), "/");
    }

    #[test]
    fn query_ids_are_validated() {
        let q = Query::parse(Some("id=MPREb_abc-1&bad=..%2Fetc&s=%D1%81%D0%BA"));
        assert_eq!(q.id("id").unwrap(), "MPREb_abc-1");
        assert!(matches!(q.id("bad"), Err(ApiError::BadRequest(_))));
        assert!(matches!(q.id("missing"), Err(ApiError::BadRequest(_))));
        assert_eq!(q.get("s"), Some("ск"));
    }

    #[test]
    fn cache_keys_ignore_parameter_order() {
        let a = Query::parse(Some("id=x&offset=5"));
        let b = Query::parse(Some("offset=5&id=x"));
        assert_eq!(a.cache_key("/album"), b.cache_key("/album"));
        assert_ne!(
            a.cache_key("/album"),
            Query::parse(Some("id=x")).cache_key("/album")
        );
    }

    #[test]
    fn errors_map_to_contract_statuses() {
        let nf: ApiError = volna_ytm::Error::NotFound("x".into()).into();
        assert_eq!(nf.status(), StatusCode::NOT_FOUND);
        let up: ApiError = volna_ytm::Error::Bootstrap("x".into()).into();
        assert_eq!(up.status(), StatusCode::BAD_GATEWAY);
    }
}
