//! Commands for the stream layer: `stream_*` everywhere, `ytdlp_*` on
//! desktop (feature `ytdlp`).

#[cfg(feature = "ytdlp")]
use serde::Serialize;
use volna_streams::{Quality, VideoId};
#[cfg(feature = "ytdlp")]
use volna_streams::{Origin, UpdateReport};

use crate::state::AppState;

/// Errors cross the IPC boundary as plain strings.
type CmdResult<T> = Result<T, String>;

/// What the UI is told when there is nothing to resolve streams with.
pub const NO_BACKEND: &str = "yt-dlp not found";

#[cfg(feature = "ytdlp")]
fn ytdlp(state: &AppState) -> CmdResult<&volna_streams::YtDlp> {
    state.ytdlp().ok_or_else(|| NO_BACKEND.to_owned())
}

#[cfg(feature = "ytdlp")]
#[derive(Serialize)]
pub struct YtDlpInfo {
    binary: String,
    origin: Origin,
    version: String,
}

#[cfg(feature = "ytdlp")]
pub async fn ytdlp_version(state: &AppState) -> CmdResult<YtDlpInfo> {
    let ytdlp = ytdlp(state)?;
    Ok(YtDlpInfo {
        binary: ytdlp.binary().display().to_string(),
        origin: ytdlp.origin(),
        version: ytdlp.version().await.map_err(|e| e.to_string())?,
    })
}

/// `yt-dlp -U` for a bundled binary (through the current proxy); for a
/// system binary only reports its version.
#[cfg(feature = "ytdlp")]
pub async fn ytdlp_update(state: &AppState) -> CmdResult<UpdateReport> {
    let proxy = state.net.proxy_url();
    ytdlp(state)?
        .self_update(proxy.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Resolves the next track ahead of time. `quality` is `"high"` (default) or `"low"`.
/// Async so it runs on the tokio runtime that `prefetch` spawns onto.
pub async fn stream_prefetch(
    state: &AppState,
    video_id: String,
    quality: Option<Quality>,
) -> CmdResult<()> {
    let streams = state.streams.as_ref().ok_or(NO_BACKEND)?;
    let id = VideoId::parse(&video_id).map_err(|e| e.to_string())?;
    streams.prefetch(id, quality.unwrap_or_default());
    Ok(())
}

/// The loopback audio URL prefix, also injected as `window.__VOLNA__.streamBase`.
pub fn stream_base(state: &AppState) -> Option<String> {
    state.stream_base.clone()
}

/// Why the last attempt to play `videoId` failed, or `None` if it did not.
/// `<audio>` only sees an HTTP error; the UI asks here to explain it.
pub fn stream_failure(
    state: &AppState,
    video_id: String,
) -> CmdResult<Option<volna_streams::Failure>> {
    let Some(streams) = state.streams.as_ref() else {
        return Ok(Some(volna_streams::Failure {
            kind: volna_streams::FailureKind::NoYtDlp,
            message: NO_BACKEND.to_owned(),
        }));
    };
    let id = VideoId::parse(&video_id).map_err(|e| e.to_string())?;
    Ok(streams.last_failure(&id))
}
