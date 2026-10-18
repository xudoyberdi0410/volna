//! `volna://` requests: proxied cover images and audio bytes.
//!
//! The same request arrives in two shapes depending on the platform:
//! - Linux / macOS / mobile: `volna://img/?u=x` — the route is the URI host.
//! - Windows (WebView2): `http://volna.localhost/img/?u=x` — the host is
//!   fixed and the route is the first path segment.
//!
//! [`Target::parse`] normalises both into a route plus the remaining path.

use http::{Request, Response, StatusCode, header};

use crate::media;
use crate::state::AppState;

/// Host used by WebView2 for custom schemes (`http://<scheme>.localhost/`).
const WINDOWS_HOST: &str = "volna.localhost";

/// Which service a request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Audio bytes with Range support (`volna://stream/<videoId>`).
    Stream,
    /// Proxied cover images (`volna://img/?u=<urlencoded>&s=<px>`).
    Img,
}

/// A parsed `volna://` request target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub route: Route,
    /// Path after the route, always starting with `/` (e.g. `/search/`).
    pub path: String,
    /// Raw query string without `?`, if any.
    pub query: Option<String>,
}

impl Target {
    /// Parses a request URI; returns `None` for unknown routes.
    pub fn parse(uri: &http::Uri) -> Option<Self> {
        let host = uri.host().unwrap_or_default();
        let raw_path = uri.path();

        let (route_name, rest) = if host.eq_ignore_ascii_case(WINDOWS_HOST) {
            let trimmed = raw_path.trim_start_matches('/');
            match trimmed.split_once('/') {
                Some((first, rest)) => (first, format!("/{rest}")),
                None => (trimmed, "/".to_owned()),
            }
        } else {
            let rest = if raw_path.is_empty() { "/" } else { raw_path };
            (host, rest.to_owned())
        };

        let route = match route_name.to_ascii_lowercase().as_str() {
            "stream" => Route::Stream,
            "img" => Route::Img,
            _ => return None,
        };

        Some(Self {
            route,
            path: rest,
            query: uri.query().map(str::to_owned),
        })
    }
}

/// Answers a `volna://` request: `OPTIONS` (CORS preflight), `stream` and
/// `img`; anything else is a JSON 404.
pub async fn handle(state: &AppState, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    if request.method() == http::Method::OPTIONS {
        return with_cors(Response::builder().status(StatusCode::NO_CONTENT))
            .body(Vec::new())
            .expect("static response is valid");
    }
    match Target::parse(request.uri()) {
        Some(target) if target.route == Route::Stream => {
            media::stream(state, &target, &request).await
        }
        Some(target) => media::image(state, &target).await,
        None => json_error(StatusCode::NOT_FOUND, "unknown volna:// route"),
    }
}

pub fn with_cors(builder: http::response::Builder) -> http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Length, Content-Range, Accept-Ranges, Content-Type",
        )
}

pub fn json_error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    let body = serde_json::to_vec(&serde_json::json!({
        "status": status.as_u16(),
        "error": message,
    }))
    .expect("JSON serialisation of a static shape cannot fail");

    with_cors(Response::builder().status(status))
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .expect("static response is valid")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// App state with a throwaway cache dir, as the daemon would build it.
    pub(crate) fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("volna-test-cache-{}", std::process::id()));
        AppState::for_tests(&dir)
    }

    fn parse(uri: &str) -> Option<Target> {
        Target::parse(&uri.parse().unwrap())
    }

    #[test]
    fn unix_style_uses_host_as_route() {
        let t = parse("volna://img/x?u=a%20b").unwrap();
        assert_eq!(t.route, Route::Img);
        assert_eq!(t.path, "/x");
        assert_eq!(t.query.as_deref(), Some("u=a%20b"));
    }

    #[test]
    fn windows_style_uses_first_segment_as_route() {
        let t = parse("http://volna.localhost/stream/dQw4w9WgXcQ").unwrap();
        assert_eq!(t.route, Route::Stream);
        assert_eq!(t.path, "/dQw4w9WgXcQ");
        assert_eq!(t.query, None);
    }

    #[test]
    fn route_without_path_gets_root() {
        assert_eq!(parse("volna://img").unwrap().path, "/");
        assert_eq!(parse("http://volna.localhost/img").unwrap().path, "/");
    }

    #[test]
    fn unknown_route_is_none() {
        assert_eq!(parse("volna://evil/x"), None);
        assert_eq!(parse("http://volna.localhost/evil/x"), None);
    }

    #[tokio::test]
    async fn unknown_route_answers_404() {
        let req = Request::get("volna://nope/").body(Vec::new()).unwrap();
        assert_eq!(
            handle(&test_state(), req).await.status(),
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn the_old_api_route_is_gone() {
        let req = Request::get("volna://api/").body(Vec::new()).unwrap();
        let res = handle(&test_state(), req).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }
}
