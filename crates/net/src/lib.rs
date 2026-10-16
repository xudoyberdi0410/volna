//! Networking: a single `reqwest` client (rustls) built from the user's
//! network settings — direct, proxy URL (`socks5://` / `http://`) or a
//! `vless://` link served by a bundled xray sidecar that exposes a local
//! SOCKS port. Every outgoing request (InnerTube, audio, covers, yt-dlp's
//! `--proxy`) uses the same exit.

mod error;
pub mod vless;
pub mod xray;

pub use error::{NetError, Result};
pub use vless::{LinkError, VlessLink};
