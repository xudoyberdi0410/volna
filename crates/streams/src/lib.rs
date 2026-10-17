//! Audio stream resolution: a yt-dlp sidecar (feature `ytdlp`, default) or
//! in-process InnerTube with a host-provided JavaScript runtime (feature
//! `innertube`, for mobile) behind [`Backend`].
//!
//! Resolves a `videoId` to a googlevideo audio URL (through the same proxy
//! as everything else, since the URLs are IP-bound), caches results until
//! shortly before their `expire`, and backs the `volna://stream/<videoId>`
//! protocol and a loopback HTTP server (what `<audio>` plays from) with
//! HTTP Range support. Also proxies cover images with an
//! on-disk cache and handles yt-dlp self-updates.
//!
//! ```no_run
//! # #[cfg(feature = "ytdlp")]
//! # async fn demo() -> volna_streams::Result<()> {
//! use std::sync::Arc;
//! use volna_net::{NetManager, NetOptions};
//! use volna_streams::{Quality, StreamRequest, StreamService, VideoId, YtDlp};
//!
//! let net = Arc::new(NetManager::new(NetOptions::default()).unwrap());
//! let streams = StreamService::new(YtDlp::locate(None)?, net);
//! let res = streams
//!     .serve(&StreamRequest {
//!         id: VideoId::parse("dQw4w9WgXcQ")?,
//!         quality: Quality::High,
//!         range: Some("bytes=0-65535".into()),
//!         head: false,
//!     })
//!     .await?;
//! assert_eq!(res.status, 206);
//! # Ok(()) }
//! ```

mod error;
pub mod image;
#[cfg(feature = "innertube")]
pub mod innertube;
mod model;
pub mod range;
mod resolver;
mod server;
mod stream;
#[cfg(feature = "ytdlp")]
pub mod ytdlp;

pub use error::{FailureKind, Result, StreamError};
pub use image::{Image, ImageCache};
#[cfg(feature = "innertube-deno")]
pub use innertube::DenoRuntime;
#[cfg(feature = "innertube")]
pub use innertube::{CookieJar, InnerTube, InnerTubeOptions, JsError, JsRuntime};
pub use model::{Codec, Delivery, Quality, ResolvedStream, Tier, VideoId};
pub use resolver::{Backend, DEFAULT_EXPIRY_MARGIN, Failure, Resolver};
pub use server::LoopbackServer;
pub use stream::{
    BodyStream, DEFAULT_CHUNK_CAP, Network, OpenStream, StreamRequest, StreamResponse,
    StreamService,
};
#[cfg(feature = "ytdlp")]
pub use ytdlp::{COOKIE_BROWSERS, Cookies, Origin, UpdateReport, YtDlp};
