//! Shared app state: the network, the YouTube Music client, the stream
//! service and the image cache, built once and reached from the commands
//! and the `volna://` handler.
//!
//! The network starts in direct mode; `network::NetworkState::restore` then
//! applies the saved setting, and `network.rs` owns every later switch.
//!
//! Audio goes through the loopback server (`stream_base`); `volna://stream/`
//! stays as a capped fallback for `fetch`.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use volna_net::{NetManager, NetOptions};
#[cfg(feature = "ytdlp")]
use volna_streams::YtDlp;
use volna_streams::{Backend, ImageCache, Quality, ResolvedStream, StreamService, VideoId};
use volna_ytm::YtmClient;

pub type Streams = StreamService<StreamBackend, NetManager>;
pub type Images = ImageCache<NetManager>;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// [`Backend`] behind a pointer, so a host can inject one at init.
pub trait DynBackend: Send + Sync + 'static {
    fn resolve_dyn<'a>(
        &'a self,
        id: &'a VideoId,
        quality: Quality,
        proxy: Option<&'a str>,
    ) -> BoxFuture<'a, volna_streams::Result<ResolvedStream>>;
}

impl<B: Backend> DynBackend for B {
    fn resolve_dyn<'a>(
        &'a self,
        id: &'a VideoId,
        quality: Quality,
        proxy: Option<&'a str>,
    ) -> BoxFuture<'a, volna_streams::Result<ResolvedStream>> {
        Box::pin(self.resolve(id, quality, proxy))
    }
}

/// How audio URLs are resolved: yt-dlp on desktop, a backend the host
/// injects elsewhere (the mobile InnerTube resolver, test fakes).
pub enum StreamBackend {
    #[cfg(feature = "ytdlp")]
    YtDlp(YtDlp),
    /// The in-process resolver (mobile); kept by type so the YouTube
    /// session can be swapped at run time (`youtube_set_cookies`).
    #[cfg(feature = "innertube")]
    InnerTube(volna_streams::InnerTube),
    Injected(Arc<dyn DynBackend>),
}

impl StreamBackend {
    pub fn injected(backend: impl Backend) -> Self {
        Self::Injected(Arc::new(backend))
    }

    /// yt-dlp next to the executable or on `PATH`; `None` (logged) without it.
    #[cfg(feature = "ytdlp")]
    pub fn locate_ytdlp() -> Option<Self> {
        match YtDlp::locate(None) {
            Ok(ytdlp) => {
                tracing::info!(binary = %ytdlp.binary().display(), origin = ?ytdlp.origin(), "using yt-dlp");
                Some(Self::YtDlp(ytdlp))
            }
            Err(e) => {
                tracing::error!("{e}; playback is disabled");
                None
            }
        }
    }

    /// The yt-dlp sidecar, when that is the backend.
    #[cfg(feature = "ytdlp")]
    pub fn ytdlp(&self) -> Option<&YtDlp> {
        match self {
            Self::YtDlp(y) => Some(y),
            _ => None,
        }
    }

    /// The InnerTube resolver, when that is the backend.
    #[cfg(feature = "innertube")]
    pub fn innertube(&self) -> Option<&volna_streams::InnerTube> {
        match self {
            Self::InnerTube(b) => Some(b),
            _ => None,
        }
    }
}

impl Backend for StreamBackend {
    async fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> volna_streams::Result<ResolvedStream> {
        match self {
            #[cfg(feature = "ytdlp")]
            Self::YtDlp(y) => Backend::resolve(y, id, quality, proxy).await,
            #[cfg(feature = "innertube")]
            Self::InnerTube(b) => b.resolve(id, quality, proxy).await,
            Self::Injected(b) => b.resolve_dyn(id, quality, proxy).await,
        }
    }
}

/// Cheap to clone: every field is shared.
#[derive(Clone)]
pub struct AppState {
    /// The one HTTP client + proxy/xray owner. Starts as `Direct`.
    pub net: Arc<NetManager>,
    /// The InnerTube client (clones share the session and HTTP client).
    pub ytm: YtmClient,
    /// `None` without a backend (no yt-dlp found); `volna://stream/` then answers 500.
    pub streams: Option<Streams>,
    pub images: Arc<Images>,
    /// `color_palette` results per cover URL.
    pub palettes: Arc<crate::api::color::Palettes>,
    /// Base URL of the loopback audio server, once it is running.
    pub stream_base: Option<String>,
    /// The codecs the player takes (stream URLs and prefetches ask for them).
    pub codec: volna_streams::Codec,
}

impl AppState {
    /// Desktop: streams through yt-dlp (found next to the executable or on
    /// `PATH`). `cache_dir` is the app cache directory; covers go to
    /// `<cache_dir>/images`.
    #[cfg(feature = "ytdlp")]
    pub fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        Self::with_backend(cache_dir, StreamBackend::locate_ytdlp())
    }

    /// With the given stream backend (`None`: no playback).
    pub fn with_backend(cache_dir: &Path, backend: Option<StreamBackend>) -> anyhow::Result<Self> {
        let net = Arc::new(NetManager::new(NetOptions::default())?);
        let streams = backend.map(|b| StreamService::new(b, net.clone()));
        let images = Arc::new(ImageCache::new(net.clone(), cache_dir.join("images")));
        Ok(Self {
            ytm: YtmClient::new(net.client()),
            net,
            streams,
            images,
            palettes: Arc::default(),
            stream_base: None,
            codec: volna_streams::Codec::default(),
        })
    }

    /// What the daemon would build ([`Self::new`]), or no backend without yt-dlp.
    #[cfg(test)]
    pub(crate) fn for_tests(cache_dir: &Path) -> Self {
        #[cfg(feature = "ytdlp")]
        return Self::new(cache_dir).unwrap();
        #[cfg(not(feature = "ytdlp"))]
        return Self::with_backend(cache_dir, None).unwrap();
    }

    /// The yt-dlp sidecar, when streams resolve through it.
    #[cfg(feature = "ytdlp")]
    pub fn ytdlp(&self) -> Option<&YtDlp> {
        self.streams.as_ref().and_then(|s| s.backend().ytdlp())
    }

    /// Call after `NetManager::apply`: InnerTube must use the new route.
    pub async fn refresh_http_client(&self) {
        self.ytm.set_http_client(self.net.client()).await;
    }
}
