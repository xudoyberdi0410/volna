//! `volna://stream/<videoId>[?q=low]` and `volna://img/?u=<url>[&s=<px>]`.

use http::{Request, Response, StatusCode, header};
use volna_streams::{Quality, StreamError, StreamRequest, VideoId};

use crate::protocol::{Target, json_error, with_cors};
use crate::state::AppState;

/// Covers never change for a given URL.
const IMAGE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

fn query_param(target: &Target, name: &str) -> Option<String> {
    let query = target.query.as_deref()?;
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

fn error_response(e: &StreamError) -> Response<Vec<u8>> {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut res = json_error(status, &e.to_string());
    if let StreamError::RangeNotSatisfiable { total } = e
        && let Ok(v) = format!("bytes */{total}").parse()
    {
        res.headers_mut().insert(header::CONTENT_RANGE, v);
    }
    res
}

pub async fn stream(
    state: &AppState,
    target: &Target,
    request: &Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let Some(streams) = &state.streams else {
        return error_response(&StreamError::YtDlpNotFound);
    };
    let id = match VideoId::parse(target.path.trim_matches('/')) {
        Ok(id) => id,
        Err(e) => return error_response(&e),
    };
    let req = StreamRequest {
        id,
        quality: Quality::from_param(query_param(target, "q").as_deref()),
        range: request
            .headers()
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        head: request.method() == http::Method::HEAD,
    };

    match streams.serve(&req).await {
        Ok(res) => {
            tracing::debug!(id = %req.id, range = ?req.range, served = %res.content_range, "stream");
            with_cors(Response::builder().status(res.status))
                .header(header::CONTENT_TYPE, res.content_type)
                .header(header::CONTENT_LENGTH, res.content_length)
                .header(header::CONTENT_RANGE, res.content_range)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CACHE_CONTROL, "no-store")
                .body(res.body.to_vec())
                .expect("stream response headers are valid")
        }
        Err(e) => {
            tracing::warn!(id = %req.id, range = ?req.range, "stream failed: {e}");
            error_response(&e)
        }
    }
}

pub async fn image(state: &AppState, target: &Target) -> Response<Vec<u8>> {
    let Some(url) = query_param(target, "u") else {
        return json_error(StatusCode::BAD_REQUEST, "missing u= parameter");
    };
    let size = query_param(target, "s").and_then(|s| s.parse::<u32>().ok());
    match state.images.get(&url, size).await {
        Ok(img) => with_cors(Response::builder().status(StatusCode::OK))
            .header(header::CONTENT_TYPE, img.content_type)
            .header(header::CACHE_CONTROL, IMAGE_CACHE_CONTROL)
            .body(img.bytes.to_vec())
            .expect("image response headers are valid"),
        Err(e) => {
            tracing::debug!(%url, "image failed: {e}");
            error_response(&e)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::handle;
    use crate::protocol::tests::test_state;
    use http::{Request, StatusCode, header};

    fn get(uri: &str, range: Option<&str>) -> Request<Vec<u8>> {
        let mut b = Request::get(uri);
        if let Some(r) = range {
            b = b.header(header::RANGE, r);
        }
        b.body(Vec::new()).unwrap()
    }

    #[tokio::test]
    async fn rejects_bad_video_id() {
        let state = test_state();
        if state.streams.is_none() {
            return; // no yt-dlp here: the route answers 500 instead.
        }
        let res = handle(&state, get("volna://stream/../../etc", None)).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    }

    #[tokio::test]
    async fn image_route_validates_input() {
        let state = test_state();
        let res = handle(&state, get("volna://img/", None)).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let res = handle(
            &state,
            get("volna://img/?u=https%3A%2F%2Fexample.com%2Fa.jpg", None),
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// The curl equivalent: real yt-dlp + googlevideo through `handle`.
    /// `cargo test -p volna live_stream_range -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "network + yt-dlp"]
    async fn live_stream_range() {
        let state = test_state();
        assert!(state.streams.is_some(), "yt-dlp not found");

        let res = handle(
            &state,
            get("volna://stream/dQw4w9WgXcQ", Some("bytes=0-65535")),
        )
        .await;
        assert_eq!(
            res.status(),
            StatusCode::PARTIAL_CONTENT,
            "{:?}",
            String::from_utf8_lossy(res.body())
        );
        let h = res.headers();
        println!("{h:?}");
        assert_eq!(h[header::CONTENT_TYPE], "audio/webm");
        assert_eq!(h[header::ACCEPT_RANGES], "bytes");
        assert_eq!(h[header::CONTENT_LENGTH], "65536");
        let cr = h[header::CONTENT_RANGE].to_str().unwrap().to_owned();
        assert!(cr.starts_with("bytes 0-65535/"), "{cr}");
        assert_eq!(res.body().len(), 65536);
        let total: u64 = cr.rsplit('/').next().unwrap().parse().unwrap();

        // What a media element does first: open-ended range, capped.
        let res = handle(&state, get("volna://stream/dQw4w9WgXcQ", Some("bytes=0-"))).await;
        assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            res.body().len() as u64,
            volna_streams::DEFAULT_CHUNK_CAP.min(total)
        );

        // Seek (Windows URL form), and the last bytes.
        let start = total - 100_000;
        let res = handle(
            &state,
            get(
                "http://volna.localhost/stream/dQw4w9WgXcQ",
                Some(&format!("bytes={start}-")),
            ),
        )
        .await;
        assert_eq!(
            res.headers()[header::CONTENT_RANGE].to_str().unwrap(),
            format!("bytes {start}-{}/{total}", total - 1)
        );
        assert_eq!(res.body().len(), 100_000);

        let res = handle(
            &state,
            get(
                "volna://stream/dQw4w9WgXcQ",
                Some(&format!("bytes={total}-")),
            ),
        )
        .await;
        assert_eq!(res.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            res.headers()[header::CONTENT_RANGE].to_str().unwrap(),
            format!("bytes */{total}")
        );

        let res = handle(
            &state,
            get(
                "volna://img/?u=https%3A%2F%2Fi.ytimg.com%2Fvi%2FdQw4w9WgXcQ%2Fhqdefault.jpg",
                None,
            ),
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers()[header::CONTENT_TYPE], "image/jpeg");
    }
}
