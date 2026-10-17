//! What a service call can end in, and what the queue does about it.

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// No connection, timeout, TLS: try again later.
    #[error("network: {0}")]
    Network(String),
    /// 5xx, rate limit, "service offline": try again later.
    #[error("server: {0}")]
    Server(String),
    /// Bad session, token or key: kept in the queue until the user fixes it.
    #[error("auth: {0}")]
    Auth(String),
    /// The service refused this item for good (bad params, unknown track):
    /// dropped from the queue.
    #[error("rejected: {0}")]
    Rejected(String),
    /// The service is off or missing its credentials.
    #[error("not configured: {0}")]
    NotConfigured(String),
}

impl Error {
    /// Whether a queued item should stay for another attempt.
    pub fn keep(&self) -> bool {
        !matches!(self, Error::Rejected(_))
    }

    /// Short machine-readable kind, for the UI.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Network(_) => "network",
            Error::Server(_) => "server",
            Error::Auth(_) => "auth",
            Error::Rejected(_) => "rejected",
            Error::NotConfigured(_) => "not_configured",
        }
    }
}

/// Classifies a non-2xx HTTP status of a JSON API (ListenBrainz, Maloja).
pub fn from_status(status: u16, body: &str) -> Error {
    let msg = format!(
        "HTTP {status}: {}",
        body.chars().take(300).collect::<String>()
    );
    match status {
        401 | 403 => Error::Auth(msg),
        408 | 429 => Error::Server(msg),
        s if s >= 500 => Error::Server(msg),
        _ => Error::Rejected(msg),
    }
}
