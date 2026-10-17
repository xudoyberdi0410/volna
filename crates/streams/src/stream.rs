//! [`StreamService`]: proxies byte ranges of a resolved track from
//! googlevideo through the shared (proxied) client.
//!
//! Two front doors use it:
//! - [`StreamService::open`] streams any range without buffering (the
//!   loopback HTTP server, which is what `<audio>` plays from);
//! - [`StreamService::serve`] returns at most `chunk_cap` bytes in memory
//!   (the `volna://stream/` custom protocol, whose API needs the whole body).

use std::pin::Pin;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};
use reqwest::{StatusCode, header};

use crate::error::{Result, StreamError};
use crate::range::{self, RangeSpec, Span};
use crate::resolver::{Backend, Resolver};
use crate::ytdlp::{Quality, ResolvedStream, VideoId};

/// Largest body [`StreamService::serve`] returns. A short `206` makes the
/// media element ask for the next range, so memory stays bounded.
/// 2 MiB is ~2 minutes of 128 kbit/s Opus.
pub const DEFAULT_CHUNK_CAP: u64 = 2 * 1024 * 1024;

/// How many times a URL googlevideo rejects up front is resolved again.
/// Some yt-dlp URLs (web_embedded with a video-bound PO token experiment) are
/// refused straight away while the next resolve of the same video works.
const OPEN_RERESOLVES: usize = 3;

/// Bytes per upstream request. yt-dlp uses ~10 MB chunks against YouTube
/// throttling; a long open range is fetched as consecutive windows.
const UPSTREAM_WINDOW: u64 = 8 * 1024 * 1024;

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

/// One stream request.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub id: VideoId,
    pub quality: Quality,
    /// Raw `Range` header, if any.
    pub range: Option<String>,
    /// `HEAD`: headers only.
    pub head: bool,
}

/// A buffered, capped answer from [`StreamService::serve`].
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

/// Body of an [`OpenStream`]: audio bytes as they arrive from upstream.
pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

/// A streaming answer from [`StreamService::open`].
pub struct OpenStream {
    /// `206` for a Range request, `200` otherwise.
    pub status: u16,
    pub content_type: String,
    pub content_length: u64,
    /// Set for `206`.
    pub content_range: Option<String>,
    pub total: u64,
    /// `None` for `HEAD`.
    pub body: Option<BodyStream>,
}

impl std::fmt::Debug for OpenStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenStream")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("content_length", &self.content_length)
            .field("content_range", &self.content_range)
            .field("total", &self.total)
            .finish_non_exhaustive()
    }
}

struct Inner<B, N> {
    resolver: Resolver<B>,
    net: Arc<N>,
    chunk_cap: u64,
    upstream_window: u64,
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

/// googlevideo's answer to one window request, body not read yet.
enum Upstream {
    Ok {
        resp: reqwest::Response,
        total: u64,
    },
    /// The URL was rejected (403/410): re-resolve and try again.
    Expired,
}

/// The first window is open; the rest of the body comes from [`body_stream`].
struct Opened {
    stream: Arc<ResolvedStream>,
    span: Span,
    total: u64,
    first: Option<reqwest::Response>,
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
                upstream_window: UPSTREAM_WINDOW,
            }),
        }
    }

    /// Overrides the per-request upstream window (tests use tiny ones).
    /// Call before cloning the service.
    pub fn with_upstream_window(mut self, bytes: u64) -> Self {
        assert!(bytes > 0, "window must be positive");
        Arc::get_mut(&mut self.inner)
            .expect("with_upstream_window is called before the service is cloned")
            .upstream_window = bytes;
        self
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

    /// Why the last resolve of `id` failed, if it did.
    pub fn last_failure(&self, id: &VideoId) -> Option<crate::Failure> {
        self.inner.resolver.last_failure(id)
    }

    /// Drops every cached stream URL. Call after switching the network.
    pub fn clear_cache(&self) {
        self.inner.resolver.clear();
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

    /// Streams the requested range (or the whole track) without a size cap.
    /// Upstream is read window by window as the consumer pulls the body; an
    /// expired URL is re-resolved once, also in the middle of the body.
    pub async fn open(&self, req: &StreamRequest) -> Result<OpenStream> {
        let opened = self.open_span(req, u64::MAX).await?;
        let partial = req.range.as_deref().and_then(RangeSpec::parse).is_some();
        let body = opened
            .first
            .map(|first| body_stream(self.clone(), req, &opened.stream, opened.span, first));
        Ok(OpenStream {
            status: if partial { 206 } else { 200 },
            content_type: opened.stream.mime.clone(),
            content_length: opened.span.len(),
            content_range: partial.then(|| opened.span.content_range(opened.total)),
            total: opened.total,
            body: if req.head { None } else { body },
        })
    }

    /// Serves at most `chunk_cap` bytes, buffered.
    ///
    /// The answer is always a `206`, even without a `Range` header: a `200`
    /// with a partial body would look like a short file, while a `206` makes
    /// the client ask for the next range.
    pub async fn serve(&self, req: &StreamRequest) -> Result<StreamResponse> {
        let opened = self.open_span(req, self.inner.chunk_cap).await?;
        let mut body = BytesMut::new();
        if let (Some(first), false) = (opened.first, req.head) {
            let mut stream = body_stream(self.clone(), req, &opened.stream, opened.span, first);
            while let Some(chunk) = stream.next().await {
                body.extend_from_slice(&chunk?);
            }
        }
        // Upstream may end early; describe what we actually have.
        let span = if req.head || body.len() as u64 == opened.span.len() {
            opened.span
        } else if body.is_empty() {
            return Err(StreamError::UpstreamStatus(StatusCode::NO_CONTENT.as_u16()));
        } else {
            Span {
                start: opened.span.start,
                end: opened.span.start + body.len() as u64 - 1,
            }
        };
        Ok(StreamResponse {
            status: StatusCode::PARTIAL_CONTENT.as_u16(),
            content_type: opened.stream.mime.clone(),
            content_length: span.len(),
            content_range: span.content_range(opened.total),
            total: opened.total,
            body: body.freeze(),
        })
    }

    /// Resolves, plans the span (at most `cap` bytes) and opens the first
    /// upstream window, re-resolving once if the cached URL is rejected.
    async fn open_span(&self, req: &StreamRequest, cap: u64) -> Result<Opened> {
        let spec = req.range.as_deref().and_then(RangeSpec::parse);
        let mut stream = self.resolve(&req.id, req.quality).await?;

        for attempt in 0..=OPEN_RERESOLVES {
            if req.head
                && let Some(total) = stream.filesize
            {
                let span = range::plan(spec, Some(total), cap)?;
                return Ok(Opened {
                    stream,
                    span,
                    total,
                    first: None,
                });
            }
            match self.try_open(&stream, spec, cap).await? {
                Some((span, total, resp)) => {
                    return Ok(Opened {
                        stream,
                        span,
                        total,
                        first: Some(resp),
                    });
                }
                None if attempt < OPEN_RERESOLVES => {
                    tracing::info!(id = %req.id, "googlevideo rejected the URL, re-resolving");
                    self.inner
                        .resolver
                        .invalidate(&req.id, req.quality, &stream.url)
                        .await;
                    stream = self.resolve(&req.id, req.quality).await?;
                }
                None => break,
            }
        }
        Err(StreamError::UpstreamStatus(StatusCode::FORBIDDEN.as_u16()))
    }

    /// `Ok(None)` means the URL expired.
    async fn try_open(
        &self,
        stream: &ResolvedStream,
        spec: Option<RangeSpec>,
        cap: u64,
    ) -> Result<Option<(Span, u64, reqwest::Response)>> {
        let mut total = stream.filesize;
        if total.is_none() && matches!(spec, Some(RangeSpec::Suffix(_))) {
            // A suffix range needs the length: ask for one byte to learn it.
            match self.request(stream, Span { start: 0, end: 0 }).await? {
                Upstream::Ok { total: t, .. } => total = Some(t),
                Upstream::Expired => return Ok(None),
            }
        }
        let mut span = range::plan(spec, total, cap)?;
        let window = Span {
            start: span.start,
            end: span
                .end
                .min(span.start.saturating_add(self.inner.upstream_window - 1)),
        };
        match self.request(stream, window).await? {
            Upstream::Ok { resp, total } => {
                if span.start >= total {
                    return Err(StreamError::RangeNotSatisfiable { total });
                }
                // Only now known when yt-dlp had no filesize.
                span.end = span.end.min(total - 1);
                Ok(Some((span, total, resp)))
            }
            Upstream::Expired => Ok(None),
        }
    }

    /// Requests `span` and checks the answer; the body is left unread.
    async fn request(&self, stream: &ResolvedStream, span: Span) -> Result<Upstream> {
        let mut request = self
            .inner
            .net
            .client()
            .get(&stream.url)
            .header(header::RANGE, span.range_header());
        if let Some(ua) = &stream.user_agent {
            request = request.header(header::USER_AGENT, ua);
        }
        let resp = request.send().await?;

        let status = resp.status();
        let total = match status {
            StatusCode::FORBIDDEN | StatusCode::GONE => return Ok(Upstream::Expired),
            StatusCode::RANGE_NOT_SATISFIABLE => {
                let total = header_str(&resp, header::CONTENT_RANGE)
                    .and_then(|v| v.strip_prefix("bytes */")?.trim().parse().ok())
                    .or(stream.filesize)
                    .unwrap_or(0);
                return Err(StreamError::RangeNotSatisfiable { total });
            }
            StatusCode::PARTIAL_CONTENT => {
                let (upstream, total) = header_str(&resp, header::CONTENT_RANGE)
                    .and_then(range::parse_content_range)
                    .ok_or(StreamError::UpstreamStatus(206))?;
                if upstream.start != span.start {
                    return Err(StreamError::UpstreamStatus(206));
                }
                total
                    .or(stream.filesize)
                    .ok_or(StreamError::UpstreamStatus(206))?
            }
            // Range ignored, whole file follows: fine if we start at 0, the
            // body reader stops at the span end.
            StatusCode::OK if span.start == 0 => resp
                .content_length()
                .or(stream.filesize)
                .ok_or(StreamError::UpstreamStatus(200))?,
            other => return Err(StreamError::UpstreamStatus(other.as_u16())),
        };
        Ok(Upstream::Ok { resp, total })
    }
}

/// Pull-based body: drains the current upstream response, then requests
/// the next window from where it stopped, until `span.end`.
struct BodyState<B, N> {
    svc: StreamService<B, N>,
    id: VideoId,
    quality: Quality,
    stream: Arc<ResolvedStream>,
    next: u64,
    end: u64,
    current: Option<reqwest::Response>,
    /// Bytes read from `current`; a window that yields nothing is an error.
    current_read: u64,
    retried: bool,
    done: bool,
}

fn body_stream<B: Backend, N: Network>(
    svc: StreamService<B, N>,
    req: &StreamRequest,
    stream: &Arc<ResolvedStream>,
    span: Span,
    first: reqwest::Response,
) -> BodyStream {
    let state = BodyState {
        svc,
        id: req.id.clone(),
        quality: req.quality,
        stream: stream.clone(),
        next: span.start,
        end: span.end,
        current: Some(first),
        current_read: 0,
        retried: false,
        done: false,
    };
    Box::pin(futures_util::stream::unfold(state, step))
}

async fn step<B: Backend, N: Network>(
    mut st: BodyState<B, N>,
) -> Option<(Result<Bytes>, BodyState<B, N>)> {
    macro_rules! fail {
        ($e:expr) => {{
            st.done = true;
            return Some((Err($e), st));
        }};
    }
    if st.done {
        return None;
    }
    loop {
        if st.next > st.end {
            return None;
        }
        if let Some(resp) = st.current.as_mut() {
            match resp.chunk().await {
                Ok(Some(mut chunk)) => {
                    let remaining = st.end - st.next + 1;
                    if chunk.len() as u64 > remaining {
                        chunk.truncate(remaining as usize);
                    }
                    if chunk.is_empty() {
                        continue;
                    }
                    st.next += chunk.len() as u64;
                    st.current_read += chunk.len() as u64;
                    return Some((Ok(chunk), st));
                }
                Ok(None) if st.current_read == 0 => {
                    fail!(StreamError::UpstreamStatus(StatusCode::NO_CONTENT.as_u16()))
                }
                // Window finished (or the connection ended early): carry on
                // from `next` with a fresh request.
                Ok(None) => st.current = None,
                Err(e) => fail!(e.into()),
            }
            continue;
        }

        let window = Span {
            start: st.next,
            end: st
                .end
                .min(st.next.saturating_add(st.svc.inner.upstream_window - 1)),
        };
        match st.svc.request(&st.stream, window).await {
            Ok(Upstream::Ok { resp, .. }) => {
                st.current = Some(resp);
                st.current_read = 0;
            }
            Ok(Upstream::Expired) if !st.retried => {
                st.retried = true;
                tracing::info!(id = %st.id, at = st.next, "URL expired mid-stream, re-resolving");
                st.svc
                    .inner
                    .resolver
                    .invalidate(&st.id, st.quality, &st.stream.url)
                    .await;
                match st.svc.resolve(&st.id, st.quality).await {
                    Ok(stream) => st.stream = stream,
                    Err(e) => fail!(e),
                }
            }
            Ok(Upstream::Expired) => fail!(StreamError::UpstreamStatus(403)),
            Err(e) => fail!(e),
        }
    }
}

fn header_str(resp: &reqwest::Response, name: header::HeaderName) -> Option<&str> {
    resp.headers().get(name)?.to_str().ok()
}
