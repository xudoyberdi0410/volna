//! Errors of the wave engine.

/// Everything that can go wrong in the wave.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("wave database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("wave data is corrupt: {0}")]
    Json(#[from] serde_json::Error),
    #[error("YouTube Music: {0}")]
    Source(#[from] volna_ytm::Error),
    /// Candidate generation produced nothing playable (offline, all banned…).
    #[error("no candidates for the wave")]
    NoCandidates,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
