//! `volna://api/proxy/?u=<url>`: GET relay for the lyrics providers the UI's
//! `am-lyrics` component and Genius helper call.
//!
//! The webview must not reach third parties directly (PLAN.md: all traffic
//! takes the app's proxy route), and some of these hosts send no CORS
//! headers. The allowlist keeps this from being an open proxy.

use http::{Response, StatusCode, header};
use url::Url;
use volna_net::NetManager;

use super::Query;
use crate::protocol::{json_error, with_cors};

/// Exact hosts that may be relayed.
const ALLOWED_HOSTS: &[&str] = &[
    "lrclib.net",
    "lrc.red",
    "lyrics-api.binimum.org",
    "lyricsplus.binimum.org",
    "lyricsplus.prjktla.workers.dev",
    "lyricsplus-seven.vercel.app",
    "lyrics-plus-backend.vercel.app",
    "unison.boidu.dev",
    "fetch-genius.samidy.workers.dev",
    "api.genius.com",
    "translate.googleapis.com",
];

/// Lyrics and search JSON are small; anything bigger is not what we relay.
const MAX_BODY: usize = 4 * 1024 * 1024;

/// Validates `u`: https, an allowlisted host, default port, no credentials.
pub fn check_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|e| format!("bad url: {e}"))?;
    if url.scheme() != "https" {
        return Err("only https is relayed".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return Err("credentials and ports are not relayed".into());
    }
    let host = url.host_str().unwrap_or_default();
    if !ALLOWED_HOSTS.contains(&host) {
        return Err(format!("host {host} is not on the lyrics allowlist"));
    }
    Ok(url)
}

pub async fn relay(net: &NetManager, query: &Query) -> Response<Vec<u8>> {
    let Some(raw) = query.get("u") else {
        return json_error(StatusCode::BAD_REQUEST, "missing u= parameter");
    };
    let url = match check_url(raw) {
        Ok(u) => u,
        Err(e) => return json_error(StatusCode::FORBIDDEN, &e),
    };

    let upstream = match net.client().get(url.clone()).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(%url, "proxy failed: {e}");
            return json_error(StatusCode::BAD_GATEWAY, &format!("upstream: {e}"));
        }
    };
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = upstream
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    if upstream
        .content_length()
        .is_some_and(|n| n > MAX_BODY as u64)
    {
        return json_error(StatusCode::BAD_GATEWAY, "upstream body too large");
    }
    let body = match upstream.bytes().await {
        Ok(b) if b.len() <= MAX_BODY => b.to_vec(),
        Ok(_) => return json_error(StatusCode::BAD_GATEWAY, "upstream body too large"),
        Err(e) => return json_error(StatusCode::BAD_GATEWAY, &format!("upstream: {e}")),
    };
    tracing::debug!(%url, %status, bytes = body.len(), "proxy");
    with_cors(Response::builder().status(status))
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .expect("relayed headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_only_listed_https_hosts() {
        assert!(check_url("https://lrclib.net/api/get?track_name=x").is_ok());
        assert!(check_url("https://lrc.red/s/USUM.ttml").is_ok());
        assert!(check_url("http://lrclib.net/api/get").is_err());
        assert!(check_url("https://evil.lrclib.net/").is_err());
        assert!(check_url("https://lrclib.net.evil.org/").is_err());
        assert!(check_url("https://lrclib.net:8443/").is_err());
        assert!(check_url("https://user:pw@lrclib.net/").is_err());
        assert!(check_url("https://music.youtube.com/").is_err());
        assert!(check_url("not a url").is_err());
    }
}
