//! Loopback HTTP server for `<audio>` playback.
//!
//! WebKitGTK plays media through GStreamer, whose source element only
//! handles `http(s)`/`blob` URIs: `<audio src="volna://stream/…">` fails with
//! `MEDIA_ERR_SRC_NOT_SUPPORTED` before any request is made (checked on
//! WebKitGTK 2.54). So audio is served from `http://127.0.0.1:<port>/<token>/stream/<videoId>`,
//! which also lets the body stream instead of being buffered.
//!
//! The random path token keeps other local programs from using the server
//! as an open YouTube proxy. Only loopback is bound, so Windows does not
//! show a firewall prompt.

use std::collections::hash_map::RandomState;
use std::convert::Infallible;
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdListener};
use std::sync::Arc;

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::body::{Frame, Incoming};
use hyper::header::{self, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;

use crate::error::StreamError;
use crate::resolver::Backend;
use crate::stream::{Network, StreamRequest, StreamService};
use crate::ytdlp::{Quality, VideoId};

type Body = UnsyncBoxBody<Bytes, std::io::Error>;

/// A bound (not yet serving) loopback server.
pub struct LoopbackServer {
    listener: StdListener,
    addr: SocketAddr,
    token: String,
}

impl LoopbackServer {
    /// Binds `127.0.0.1` on a random port. Synchronous, so the URL is known
    /// before the webview starts; call [`serve`](Self::serve) on a runtime.
    pub fn bind() -> std::io::Result<Self> {
        let listener = StdListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        Ok(Self {
            listener,
            addr,
            token: random_token(),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// `http://127.0.0.1:<port>/<token>/stream`; append `/<videoId>[?q=low]`.
    pub fn stream_base(&self) -> String {
        format!("http://{}/{}/stream", self.addr, self.token)
    }

    /// Accepts connections forever.
    pub async fn serve<B: Backend, N: Network>(
        self,
        svc: StreamService<B, N>,
    ) -> std::io::Result<()> {
        let listener = tokio::net::TcpListener::from_std(self.listener)?;
        let prefix: Arc<str> = format!("/{}/stream/", self.token).into();
        tracing::info!("audio server on {}", self.addr);
        loop {
            let (sock, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("audio server accept: {e}");
                    continue;
                }
            };
            let (svc, prefix) = (svc.clone(), prefix.clone());
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let (svc, prefix) = (svc.clone(), prefix.clone());
                    async move { Ok::<_, Infallible>(handle(&svc, &prefix, req).await) }
                });
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(sock), service)
                    .await
                {
                    // The media element drops connections when it seeks.
                    tracing::trace!("audio connection ended: {e}");
                }
            });
        }
    }
}

async fn handle<B: Backend, N: Network>(
    svc: &StreamService<B, N>,
    prefix: &str,
    req: Request<Incoming>,
) -> Response<Body> {
    if req.method() == Method::OPTIONS {
        return cors(Response::builder().status(StatusCode::NO_CONTENT), empty());
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return error(StatusCode::METHOD_NOT_ALLOWED, "only GET and HEAD");
    }
    let Some(raw_id) = req.uri().path().strip_prefix(prefix) else {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    let id = match VideoId::parse(raw_id.trim_end_matches('/')) {
        Ok(id) => id,
        Err(e) => return stream_error(&e),
    };
    let quality = Quality::from_param(
        req.uri()
            .query()
            .and_then(|q| url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == "q"))
            .map(|(_, v)| v.into_owned())
            .as_deref(),
    );
    let request = StreamRequest {
        id,
        quality,
        range: req
            .headers()
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        head: req.method() == Method::HEAD,
    };

    match svc.open(&request).await {
        Ok(open) => {
            tracing::debug!(id = %request.id, range = ?request.range, status = open.status, length = open.content_length, "audio");
            let mut builder = Response::builder()
                .status(open.status)
                .header(header::CONTENT_TYPE, open.content_type)
                .header(header::CONTENT_LENGTH, open.content_length)
                .header(header::ACCEPT_RANGES, "bytes")
                .header(header::CACHE_CONTROL, "no-store");
            if let Some(cr) = open.content_range {
                builder = builder.header(header::CONTENT_RANGE, cr);
            }
            let body = match open.body {
                Some(stream) => {
                    let id = request.id.clone();
                    let frames = stream
                        .map_err(move |e| {
                            tracing::warn!(%id, "audio body failed: {e}");
                            std::io::Error::other(e)
                        })
                        .map_ok(Frame::data);
                    BodyExt::boxed_unsync(StreamBody::new(frames))
                }
                None => empty(),
            };
            cors(builder, body)
        }
        Err(e) => {
            tracing::warn!(id = %request.id, range = ?request.range, "audio failed: {e}");
            stream_error(&e)
        }
    }
}

fn cors(builder: hyper::http::response::Builder, body: Body) -> Response<Body> {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, HEAD, OPTIONS")
        .header(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            "Content-Length, Content-Range, Accept-Ranges, Content-Type",
        )
        .body(body)
        .expect("static headers are valid")
}

fn empty() -> Body {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed_unsync()
}

fn error(status: StatusCode, message: &str) -> Response<Body> {
    let json = serde_json::json!({ "status": status.as_u16(), "error": message }).to_string();
    let body = Full::new(Bytes::from(json))
        .map_err(|never| match never {})
        .boxed_unsync();
    cors(
        Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json"),
        body,
    )
}

fn stream_error(e: &StreamError) -> Response<Body> {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut res = error(status, &e.to_string());
    if let StreamError::RangeNotSatisfiable { total } = e
        && let Ok(v) = HeaderValue::from_str(&format!("bytes */{total}"))
    {
        res.headers_mut().insert(header::CONTENT_RANGE, v);
    }
    res
}

/// 128 bits from the OS-seeded std hasher: enough to keep other local
/// users off the port, which is all the token is for.
fn random_token() -> String {
    (0..2)
        .map(|_| format!("{:016x}", RandomState::new().build_hasher().finish()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_hex() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn stream_base_contains_token() {
        let server = LoopbackServer::bind().unwrap();
        let base = server.stream_base();
        assert!(base.starts_with("http://127.0.0.1:"));
        assert!(base.ends_with(&format!("/{}/stream", server.token)));
    }
}
