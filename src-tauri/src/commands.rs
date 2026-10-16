//! Tauri commands for the stream layer.

use serde::Serialize;
use tauri::State;
use volna_streams::{Origin, Quality, UpdateReport, VideoId};

use crate::state::AppState;

/// Errors cross the IPC boundary as plain strings.
type CmdResult<T> = Result<T, String>;

fn ytdlp(state: &AppState) -> CmdResult<&volna_streams::YtDlp> {
    state
        .streams
        .as_ref()
        .map(|s| s.backend())
        .ok_or_else(|| "yt-dlp not found".to_owned())
}

#[derive(Serialize)]
pub struct YtDlpInfo {
    binary: String,
    origin: Origin,
    version: String,
}

#[tauri::command]
pub async fn ytdlp_version(state: State<'_, AppState>) -> CmdResult<YtDlpInfo> {
    let ytdlp = ytdlp(&state)?;
    Ok(YtDlpInfo {
        binary: ytdlp.binary().display().to_string(),
        origin: ytdlp.origin(),
        version: ytdlp.version().await.map_err(|e| e.to_string())?,
    })
}

/// `yt-dlp -U` for a bundled binary (through the current proxy); for a
/// system binary only reports its version.
#[tauri::command]
pub async fn ytdlp_update(state: State<'_, AppState>) -> CmdResult<UpdateReport> {
    let proxy = state.net.proxy_url();
    ytdlp(&state)?
        .self_update(proxy.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// Resolves the next track ahead of time. `quality` is `"high"` (default) or `"low"`.
/// Async so it runs on the tokio runtime that `prefetch` spawns onto.
#[tauri::command]
pub async fn stream_prefetch(
    state: State<'_, AppState>,
    video_id: String,
    quality: Option<Quality>,
) -> CmdResult<()> {
    let streams = state.streams.as_ref().ok_or("yt-dlp not found")?;
    let id = VideoId::parse(&video_id).map_err(|e| e.to_string())?;
    streams.prefetch(id, quality.unwrap_or_default());
    Ok(())
}

/// The loopback audio URL prefix, also injected as `window.__VOLNA__.streamBase`.
#[tauri::command]
pub fn stream_base(state: State<'_, AppState>) -> Option<String> {
    state.stream_base.clone()
}

/// Why the last attempt to play `videoId` failed, or `None` if it did not.
/// `<audio>` only sees an HTTP error; the UI asks here to explain it.
#[tauri::command]
pub fn stream_failure(
    state: State<'_, AppState>,
    video_id: String,
) -> CmdResult<Option<volna_streams::Failure>> {
    let Some(streams) = state.streams.as_ref() else {
        return Ok(Some(volna_streams::Failure {
            kind: volna_streams::FailureKind::NoYtDlp,
            message: "yt-dlp not found".to_owned(),
        }));
    };
    let id = VideoId::parse(&video_id).map_err(|e| e.to_string())?;
    Ok(streams.last_failure(&id))
}
