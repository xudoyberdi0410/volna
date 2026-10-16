use std::path::PathBuf;

use thiserror::Error;

use crate::vless::LinkError;

/// Everything that can go wrong while (re)configuring the network stack.
#[derive(Debug, Error)]
pub enum NetError {
    #[error("invalid proxy URL: {0}")]
    InvalidProxyUrl(String),

    #[error("invalid vless:// link: {0}")]
    InvalidVlessLink(#[from] LinkError),

    #[error("xray binary not found (looked next to the executable and on PATH)")]
    XrayNotFound,

    #[error("failed to start xray at {path}: {source}")]
    XraySpawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("xray exited during startup ({status}); last log lines:\n{log}")]
    XrayExited { status: String, log: String },

    #[error(
        "xray did not open its SOCKS port {port} within {timeout_ms} ms; last log lines:\n{log}"
    )]
    XrayTimeout {
        port: u16,
        timeout_ms: u128,
        log: String,
    },

    #[error("could not reserve a local port: {0}")]
    NoFreePort(#[source] std::io::Error),

    #[error("failed to build HTTP client: {0}")]
    Client(#[from] reqwest::Error),
}

pub type Result<T, E = NetError> = std::result::Result<T, E>;
