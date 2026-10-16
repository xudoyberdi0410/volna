//! [`StreamService`]: answers `volna://stream/<videoId>` requests by proxying
//! byte ranges from googlevideo through the shared (proxied) client.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use reqwest::{StatusCode, header};

use crate::error::{Result, StreamError};
use crate::range::{self, RangeSpec, Span};
use crate::resolver::{Backend, Resolver};
use crate::ytdlp::{Quality, ResolvedStream, VideoId};

/// Largest body served per request. A short `206` makes the media element
/// ask for the next range, so memory stays bounded whatever the track length.
/// 2 MiB is ~2 minutes of 128 kbit/s Opus: few round trips, small buffers.
pub const DEFAULT_CHUNK_CAP: u64 = 2 * 1024 * 1024;

/// Where the service gets its HTTP client and yt-dlp proxy from.
/// [`volna_net::NetManager`] in the app.
pub trait Network: Send + Sync + 'static {
    fn client(&self) -> reqwest::Client;
    /// Proxy URL for yt-dlp; `None` in direct mode.
    fn proxy_url(&self) -> Option<String>;
}

impl Network for volna_net::NetManager {
    fn client(&self) -> reqwest::Client {
        volna_net::NetManager::client(self)
    }
    fn proxy_url(&self) -> Option<String> {
        volna_net::NetManager::proxy_url(self)
    }
}

/// One `volna://stream/` request.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub id: VideoId,
    pub quality: Quality,
    /// Raw `Range` header, if any.
    pub range: Option<String>,
    /// `HEAD`: headers only.
    pub head: bool,
}

/// What to send back to the webview.
#[derive(Debug, Clone)]
pub struct StreamResponse {
    /// Always `206 Partial Content`: see [`StreamService::serve`].
    pub status: u16,
    pub content_type: String,
    pub content_length: u64,
    pub content_range: String,
    /// Full stream length.
    pub total: u64,
    /// Empty for `HEAD`.
    pub body: Bytes,
}

struct Inner<B, N> {
    resolver: Resolver<B>,
    net: Arc<N>,
    chunk_cap: u64,
}

/// Cheap to clone; clones share the cache.
pub struct StreamService<B, N> {
    inner: Arc<Inner<B, N>>,
}

impl<B, N> Clone for StreamService<B, N> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// What googlevideo gave us for one span.
struct Fetched {
    span: Span,
    total: u64,
    body: Bytes,
}

enum Attempt {
    Done(Fetched),
    /// The URL was rejected (403/410): re-resolve and try again.
    Expired,
}

impl<B: Backend, N: Network> StreamService<B, N> {
    pub fn new(backend: B, net: Arc<N>) -> Self {
        Self::with_resolver(Resolver::new(backend), net, DEFAULT_CHUNK_CAP)
    }

    pub fn with_resolver(resolver: Resolver<B>, net: Arc<N>, chunk_cap: u64) -> Self {
        assert!(chunk_cap > 0, "chunk cap must be positive");
        Self {
            inner: Arc::new(Inner {
                resolver,
                net,
                chunk_cap,
            }),
        }
    }

    pub fn backend(&self) -> &B {
        self.inner.resolver.backend()
    }

    pub fn net(&self) -> &Arc<N> {
        &self.inner.net
    }

    /// Resolves (or returns the cached) stream for `id`.
    pub async fn resolve(&self, id: &VideoId, quality: Quality) -> Result<Arc<ResolvedStream>> {
        let proxy = self.inner.net.proxy_url();
        self.inner.resolver.get(id, quality, proxy.as_deref()).await
    }

    /// Resolves `id` in the background so the next track starts instantly.
    /// Must be called inside a tokio runtime.
    pub fn prefetch(&self, id: VideoId, quality: Quality) {
        let this = self.clone();
        tokio::spawn(async move {
            if let Err(e) = this.resolve(&id, quality).await {
                tracing::debug!(%id, "prefetch failed: {e}");
            }
        });
    }

    /// Serves one request.
    ///
    /// The answer is always a `206` with at most `chunk_cap` bytes, even
    /// without a `Range` header: the custom-protocol API cannot stream a
    /// body, and a `200` with a partial body would look like a short file.
    /// Media elements in WebKitGTK and WebView2 accept the `206` and keep
    /// requesting further ranges.
    pub async fn serve(&self, req: &StreamRequest) -> Result<StreamResponse> {
        let spec = req.range.as_deref().and_then(RangeSpec::parse);
        let mut stream = self.resolve(&req.id, req.quality).await?;

        for attempt in 0..2 {
            if req.head
                && let Some(total) = stream.filesize
            {
                let span = range::plan(spec, Some(total), self.inner.chunk_cap)?;
                return Ok(self.response(&stream, span, total, Bytes::new()));
            }

            match self.fetch(&stream, spec).await? {
                Attempt::Done(f) => {
                    let body = if req.head { Bytes::new() } else { f.body };
                    return Ok(self.response(&stream, f.span, f.total, body));
                }
                Attempt::Expired if attempt == 0 => {
                    tracing::info!(id = %req.id, "googlevideo rejected the URL, re-resolving");
                    self.inner
                        .resolver
                        .invalidate(&req.id, req.quality, &stream.url)
                        .await;
                    stream = self.resolve(&req.id, req.quality).await?;
                }
                Attempt::Expired => break,
            }
        }
        Err(StreamError::UpstreamStatus(StatusCode::FORBIDDEN.as_u16()))
    }

    fn response(
        &self,
        stream: &ResolvedStream,
        span: Span,
        total: u64,
        body: Bytes,
    ) -> StreamResponse {
        StreamResponse {
            status: StatusCode::PARTIAL_CONTENT.as_u16(),
            content_type: stream.mime.clone(),
            content_length: span.len(),
            content_range: span.content_range(total),
            total,
            body,
        }
    }

    async fn fetch(&self, stream: &ResolvedStream, spec: Option<RangeSpec>) -> Result<Attempt> {
        let client = self.inner.net.client();
        let mut total = stream.filesize;
        if total.is_none() && matches!(spec, Some(RangeSpec::Suffix(_))) {
            // A suffix range needs the length: ask for one byte to learn it.
            match self
                .fetch_span(&client, stream, Span { start: 0, end: 0 })
                .await?
            {
                Attempt::Done(f) => total = Some(f.total),
                Attempt::Expired => return Ok(Attempt::Expired),
            }
        }
        let span = range::plan(spec, total, self.inner.chunk_cap)?;
        self.fetch_span(&client, stream, span).await
    }

    async fn fetch_span(
        &self,
        client: &reqwest::Client,
        stream: &ResolvedStream,
        span: Span,
    ) -> Result<Attempt> {
        let mut request = client
            .get(&stream.url)
            .header(header::RANGE, span.range_header());
        if let Some(ua) = &stream.user_agent {
            request = request.header(header::USER_AGENT, ua);
        }
        let mut resp = request.send().await?;

        let status = resp.status();
        let (upstream_span, total) = match status {
            StatusCode::FORBIDDEN | StatusCode::GONE => return Ok(Attempt::Expired),
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total = header_str(&resp, header::CONTENT_RANGE)
                    .and_then(|v| v.strip_prefix("bytes */")?.trim().parse().ok())
                    .or(stream.filesize)
                    .unwrap_or(0);
                return Err(StreamError::RangeNotSatisfiable { total });
            }
            StatusCode::PARTIAL_CONTENT => {
                let (span, total) = header_str(&resp, header::CONTENT_RANGE)
                    .and_then(range::parse_content_range)
                    .ok_or(StreamError::UpstreamStatus(206))?;
                let total = total
                    .or(stream.filesize)
                    .ok_or(StreamError::UpstreamStatus(206))?;
                (span, total)
            }
            // The server ignored Range and sends the whole file: usable only
            // if we wanted the beginning anyway.
            StatusCode::OK if span.start == 0 => {
                let total = resp
                    .content_length()
                    .or(stream.filesize)
                    .ok_or(StreamError::UpstreamStatus(200))?;
                let end = span.end.min(total.saturating_sub(1));
                (Span { start: 0, end }, total)
            }
            other => return Err(StreamError::UpstreamStatus(other.as_u16())),
        };
        if upstream_span.start != span.start {
            return Err(StreamError::UpstreamStatus(status.as_u16()));
        }

        // Read at most what we asked for; stop early instead of draining.
        let want = span.len().min(upstream_span.len()) as usize;
        let mut buf = BytesMut::with_capacity(want);
        while buf.len() < want {
            match resp.chunk().await? {
                Some(chunk) => buf.extend_from_slice(&chunk),
                None => break,
            }
        }
        buf.truncate(want);
        if buf.is_empty() {
            return Err(StreamError::UpstreamStatus(status.as_u16()));
        }
        let span = Span {
            start: span.start,
            end: span.start + buf.len() as u64 - 1,
        };
        Ok(Attempt::Done(Fetched {
            span,
            total,
            body: buf.freeze(),
        }))
    }
}

fn header_str(resp: &reqwest::Response, name: header::HeaderName) -> Option<&str> {
    resp.headers().get(name)?.to_str().ok()
}
