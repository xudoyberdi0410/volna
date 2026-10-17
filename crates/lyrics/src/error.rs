#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("HTTP {status} from {host}")]
    Status { status: u16, host: String },
    #[error("bad response from {0}")]
    Parse(String),
    #[error("timed out")]
    Timeout,
    #[error("youtube music: {0}")]
    Ytm(#[from] volna_ytm::Error),
    #[error("cache: {0}")]
    Cache(#[from] rusqlite::Error),
    #[error("cache json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
