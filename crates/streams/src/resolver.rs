//! Resolve cache: one yt-dlp run per `(videoId, quality)` until the URL is
//! about to expire, with concurrent requests for the same key sharing a run.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use crate::error::Result;
use crate::ytdlp::{Quality, ResolvedStream, VideoId, YtDlp};

/// Re-resolve this long before googlevideo's `expire`, so a track that
/// starts near the deadline can still be played to the end.
pub const DEFAULT_EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);
/// Above this many entries, finished and stale slots are dropped.
const PRUNE_THRESHOLD: usize = 256;

/// Something that turns a video id into a stream URL. [`YtDlp`] in the app,
/// a fake in tests.
pub trait Backend: Send + Sync + 'static {
    fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> impl Future<Output = Result<ResolvedStream>> + Send;
}

impl Backend for YtDlp {
    fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> impl Future<Output = Result<ResolvedStream>> + Send {
        YtDlp::resolve(self, id, quality, proxy)
    }
}

type Key = (VideoId, Quality);

#[derive(Clone)]
struct Cached {
    stream: Arc<ResolvedStream>,
    /// googlevideo URLs are bound to the exit IP: a URL resolved through a
    /// different proxy than the current one is useless.
    proxy: Option<String>,
}

/// One async mutex per key: holding it while resolving is what makes
/// concurrent callers wait for the first one instead of starting yt-dlp again.
type Slot = Arc<tokio::sync::Mutex<Option<Cached>>>;

pub struct Resolver<B> {
    backend: B,
    slots: Mutex<HashMap<Key, Slot>>,
    margin: Duration,
}

impl<B: Backend> Resolver<B> {
    pub fn new(backend: B) -> Self {
        Self::with_margin(backend, DEFAULT_EXPIRY_MARGIN)
    }

    pub fn with_margin(backend: B, margin: Duration) -> Self {
        Self {
            backend,
            slots: Mutex::new(HashMap::new()),
            margin,
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Returns a fresh stream for `id`, resolving it if needed.
    pub async fn get(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> Result<Arc<ResolvedStream>> {
        let slot = self.slot(id, quality);
        let mut cached = slot.lock().await;
        if let Some(c) = cached.as_ref()
            && self.is_usable(c, proxy, SystemTime::now())
        {
            return Ok(c.stream.clone());
        }
        let stream = Arc::new(self.backend.resolve(id, quality, proxy).await?);
        *cached = Some(Cached {
            stream: stream.clone(),
            proxy: proxy.map(str::to_owned),
        });
        Ok(stream)
    }

    /// Drops the cached entry for `id` if it still holds `url` (so a URL that
    /// was already replaced by another request is not thrown away again).
    pub async fn invalidate(&self, id: &VideoId, quality: Quality, url: &str) {
        let slot = self.slot(id, quality);
        let mut cached = slot.lock().await;
        if cached.as_ref().is_some_and(|c| c.stream.url == url) {
            tracing::debug!(%id, "invalidating cached stream URL");
            *cached = None;
        }
    }

    /// Forgets every cached URL, e.g. after the network setting changed
    /// (googlevideo URLs only work from the exit IP they were resolved on).
    /// Resolves already running finish into their own, now detached, slot.
    pub fn clear(&self) {
        let mut slots = self.lock_slots();
        tracing::debug!(entries = slots.len(), "clearing the stream URL cache");
        slots.clear();
    }

    /// Number of keys with a slot (cached or resolving). For tests and stats.
    pub fn len(&self) -> usize {
        self.lock_slots().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn is_usable(&self, c: &Cached, proxy: Option<&str>, now: SystemTime) -> bool {
        c.proxy.as_deref() == proxy
            && c.stream
                .expires_at
                .checked_sub(self.margin)
                .is_some_and(|deadline| now < deadline)
    }

    fn slot(&self, id: &VideoId, quality: Quality) -> Slot {
        let mut slots = self.lock_slots();
        let key = (id.clone(), quality);
        if !slots.contains_key(&key) && slots.len() >= PRUNE_THRESHOLD {
            let now = SystemTime::now();
            // A slot that is locked is resolving right now: keep it.
            slots.retain(|_, slot| match slot.try_lock() {
                Ok(c) => c.as_ref().is_some_and(|c| {
                    c.stream
                        .expires_at
                        .checked_sub(self.margin)
                        .is_some_and(|d| now < d)
                }),
                Err(_) => true,
            });
        }
        slots.entry(key).or_default().clone()
    }

    fn lock_slots(&self) -> std::sync::MutexGuard<'_, HashMap<Key, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StreamError;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts resolves; each returns a URL with a unique `n=` and the given lifetime.
    struct Fake {
        calls: AtomicUsize,
        lifetime: Duration,
        delay: Duration,
        fail: bool,
    }

    impl Fake {
        fn new(lifetime: Duration) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                lifetime,
                delay: Duration::from_millis(50),
                fail: false,
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Backend for Fake {
        async fn resolve(
            &self,
            id: &VideoId,
            _quality: Quality,
            _proxy: Option<&str>,
        ) -> Result<ResolvedStream> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            if self.fail {
                return Err(StreamError::YtDlpFailed("boom".into()));
            }
            Ok(ResolvedStream {
                url: format!("https://example.invalid/{id}?n={n}"),
                format_id: "251".into(),
                mime: "audio/webm".into(),
                codec: Some("opus".into()),
                bitrate_kbps: None,
                filesize: Some(100),
                expires_at: SystemTime::now() + self.lifetime,
                user_agent: None,
            })
        }
    }

    fn id() -> VideoId {
        VideoId::parse("dQw4w9WgXcQ").unwrap()
    }

    #[tokio::test]
    async fn caches_until_expiry_margin() {
        let r = Resolver::new(Fake::new(Duration::from_secs(3600)));
        let a = r.get(&id(), Quality::High, None).await.unwrap();
        let b = r.get(&id(), Quality::High, None).await.unwrap();
        assert_eq!(a.url, b.url);
        assert_eq!(r.backend().calls(), 1);

        // Different quality is a different key.
        r.get(&id(), Quality::Low, None).await.unwrap();
        assert_eq!(r.backend().calls(), 2);
    }

    #[tokio::test]
    async fn url_inside_margin_is_re_resolved() {
        // Lives 4 minutes: already inside the 5 minute margin.
        let r = Resolver::new(Fake::new(Duration::from_secs(240)));
        r.get(&id(), Quality::High, None).await.unwrap();
        r.get(&id(), Quality::High, None).await.unwrap();
        assert_eq!(r.backend().calls(), 2);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_resolve() {
        let r = Arc::new(Resolver::new(Fake::new(Duration::from_secs(3600))));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let r = r.clone();
                tokio::spawn(
                    async move { r.get(&id(), Quality::High, None).await.unwrap().url.clone() },
                )
            })
            .collect();
        let mut urls = Vec::new();
        for t in tasks {
            urls.push(t.await.unwrap());
        }
        assert_eq!(r.backend().calls(), 1);
        assert!(urls.windows(2).all(|w| w[0] == w[1]));
    }

    #[tokio::test]
    async fn proxy_change_forces_resolve() {
        let r = Resolver::new(Fake::new(Duration::from_secs(3600)));
        r.get(&id(), Quality::High, None).await.unwrap();
        r.get(&id(), Quality::High, Some("socks5h://127.0.0.1:1"))
            .await
            .unwrap();
        assert_eq!(r.backend().calls(), 2);
    }

    #[tokio::test]
    async fn invalidate_only_matching_url() {
        let r = Resolver::new(Fake::new(Duration::from_secs(3600)));
        let first = r.get(&id(), Quality::High, None).await.unwrap();
        r.invalidate(&id(), Quality::High, "https://other").await;
        assert_eq!(
            r.get(&id(), Quality::High, None).await.unwrap().url,
            first.url
        );

        r.invalidate(&id(), Quality::High, &first.url).await;
        let second = r.get(&id(), Quality::High, None).await.unwrap();
        assert_ne!(second.url, first.url);
        assert_eq!(r.backend().calls(), 2);
    }

    #[tokio::test]
    async fn clear_forces_resolve() {
        let r = Resolver::new(Fake::new(Duration::from_secs(3600)));
        r.get(&id(), Quality::High, None).await.unwrap();
        r.clear();
        assert!(r.is_empty());
        r.get(&id(), Quality::High, None).await.unwrap();
        assert_eq!(r.backend().calls(), 2);
    }

    #[tokio::test]
    async fn errors_are_not_cached() {
        let mut fake = Fake::new(Duration::from_secs(3600));
        fake.fail = true;
        let r = Resolver::new(fake);
        assert!(r.get(&id(), Quality::High, None).await.is_err());
        assert!(r.get(&id(), Quality::High, None).await.is_err());
        assert_eq!(r.backend().calls(), 2);
    }
}
