use thiserror::Error;

/// Errors from [`crate::YtmClient`].
#[derive(Debug, Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("InnerTube {endpoint} returned HTTP {status}: {message}")]
    Status {
        endpoint: String,
        status: u16,
        message: String,
    },

    #[error("invalid JSON from InnerTube: {0}")]
    Json(#[from] serde_json::Error),

    #[error("could not read ytcfg from music.youtube.com: {0}")]
    Bootstrap(String),

    /// The response parsed but did not contain what was asked for
    /// (unknown id, region-blocked page, layout we cannot read).
    #[error("not found: {0}")]
    NotFound(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
