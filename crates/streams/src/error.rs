use std::time::Duration;

/// Everything that can go wrong between a `videoId` and audio bytes.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("invalid video id {0:?}")]
    InvalidVideoId(String),

    /// The requested range starts past the end of the stream (HTTP 416).
    #[error("range not satisfiable (stream is {total} bytes)")]
    RangeNotSatisfiable { total: u64 },

    #[error("yt-dlp not found (looked for a sidecar next to the executable and on PATH)")]
    YtDlpNotFound,

    #[error("failed to run yt-dlp: {0}")]
    YtDlpSpawn(#[source] std::io::Error),

    #[error("yt-dlp did not finish within {0:?}")]
    YtDlpTimeout(Duration),

    /// YouTube says the video does not exist, is private or is region locked.
    #[error("video unavailable: {0}")]
    Unavailable(String),

    /// YouTube refused this exit IP ("Sign in to confirm you're not a bot").
    /// Another network route or browser cookies are needed.
    #[error("YouTube bot check: {0}")]
    BotCheck(String),

    /// yt-dlp failed for another reason (outdated extractor, ...).
    #[error("yt-dlp failed: {0}")]
    YtDlpFailed(String),

    #[error("unexpected yt-dlp output: {0}")]
    BadYtDlpOutput(String),

    #[error("upstream request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("upstream answered HTTP {0}")]
    UpstreamStatus(u16),

    #[error("image host not allowed: {0}")]
    HostNotAllowed(String),

    #[error("image too large (over {0} bytes)")]
    ImageTooLarge(u64),

    #[error("invalid URL: {0}")]
    InvalidUrl(String),
}

impl StreamError {
    /// The HTTP status the `volna://` handler should answer with.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::InvalidVideoId(_) | Self::InvalidUrl(_) => 400,
            Self::HostNotAllowed(_) | Self::BotCheck(_) => 403,
            Self::Unavailable(_) => 404,
            Self::RangeNotSatisfiable { .. } => 416,
            Self::YtDlpNotFound | Self::YtDlpSpawn(_) => 500,
            Self::YtDlpTimeout(_) => 504,
            Self::YtDlpFailed(_)
            | Self::BadYtDlpOutput(_)
            | Self::Http(_)
            | Self::UpstreamStatus(_)
            | Self::ImageTooLarge(_) => 502,
        }
    }
}

/// Why a track could not be resolved, in the terms the UI explains to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The exit IP is flagged by YouTube; see [`StreamError::BotCheck`].
    BotCheck,
    /// The video is gone, private or blocked in this region.
    Unavailable,
    /// yt-dlp is missing or could not be started.
    NoYtDlp,
    /// Anything else (timeouts, extractor breakage, network errors).
    Other,
}

impl StreamError {
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::BotCheck(_) => FailureKind::BotCheck,
            Self::Unavailable(_) => FailureKind::Unavailable,
            Self::YtDlpNotFound | Self::YtDlpSpawn(_) => FailureKind::NoYtDlp,
            _ => FailureKind::Other,
        }
    }
}

pub type Result<T, E = StreamError> = std::result::Result<T, E>;
