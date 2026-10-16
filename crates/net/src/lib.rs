//! Networking: a single `reqwest` client (rustls) built from the user's
//! network settings — direct, proxy URL (`socks5://` / `http://`) or a
//! `vless://` link served by a bundled xray sidecar that exposes a local
//! SOCKS port. Every outgoing request (InnerTube, audio, covers, yt-dlp's
//! `--proxy`) uses the same exit.
//!
//! ```no_run
//! # async fn demo() -> volna_net::Result<()> {
//! use volna_net::{NetManager, NetOptions, ProxyConfig};
//!
//! let net = NetManager::new(NetOptions::default())?;
//! net.apply(ProxyConfig::Vless("vless://…".into())).await?;
//! let html = net.client().get("https://music.youtube.com/").send().await?;
//! let ytdlp_proxy = net.proxy_url(); // Some("socks5h://volna:…@127.0.0.1:…")
//! println!("{:?}", net.health_check().await);
//! net.shutdown().await;
//! # Ok(()) }
//! ```

mod config;
mod error;
mod health;
mod manager;
pub mod vless;
pub mod xray;

pub use config::ProxyConfig;
pub use error::{NetError, Result};
pub use health::{HealthReport, health_check};
pub use manager::{DEFAULT_USER_AGENT, NetManager, NetOptions, NetStatus};
pub use vless::{LinkError, VlessLink};
