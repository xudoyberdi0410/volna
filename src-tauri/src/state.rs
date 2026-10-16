//! Shared app state, built once in `setup` and reached from the protocol
//! handler and commands.
//!
//! The network starts in direct mode; `network::NetworkState::restore` then
//! applies the saved setting, and `network.rs` owns every later switch.
//!
//! Audio for `<audio>` goes through the loopback server (`stream_base`),
//! started in `lib.rs`; `volna://stream/` stays as a capped fallback for `fetch`.

use std::path::Path;
use std::sync::Arc;

use volna_net::{NetManager, NetOptions};
use volna_streams::{ImageCache, StreamService, YtDlp};

use crate::api_emu::ApiEmu;

pub type Streams = StreamService<YtDlp, NetManager>;
pub type Images = ImageCache<NetManager>;

/// Cheap to clone: every field is shared.
#[derive(Clone)]
pub struct AppState {
    /// The one HTTP client + proxy/xray owner. Starts as `Direct`.
    pub net: Arc<NetManager>,
    /// `None` when yt-dlp was not found; `volna://stream/` then answers 500.
    pub streams: Option<Streams>,
    pub images: Arc<Images>,
    /// `volna://api/` (hifi-api emulation over YouTube Music).
    pub api: Arc<ApiEmu>,
    /// Base URL of the loopback audio server, once it is running.
    pub stream_base: Option<String>,
}

impl AppState {
    /// `cache_dir` is the app cache directory; covers go to `<cache_dir>/images`.
    pub fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        let net = Arc::new(NetManager::new(NetOptions::default())?);
        let streams = match YtDlp::locate(None) {
            Ok(ytdlp) => {
                tracing::info!(binary = %ytdlp.binary().display(), origin = ?ytdlp.origin(), "using yt-dlp");
                Some(StreamService::new(ytdlp, net.clone()))
            }
            Err(e) => {
                tracing::error!("{e}; playback is disabled");
                None
            }
        };
        let images = Arc::new(ImageCache::new(net.clone(), cache_dir.join("images")));
        let api = Arc::new(ApiEmu::new(net.clone()));
        Ok(Self {
            net,
            streams,
            images,
            api,
            stream_base: None,
        })
    }
}
