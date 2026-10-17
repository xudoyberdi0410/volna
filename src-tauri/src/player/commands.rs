//! §3.3 commands. Errors are `"<code>: <message>"`; every command replies as
//! soon as the player state is updated.

use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, State};
use volna_player::{
    ContinuationKind, DspConfig, EnqueueAt, JournalEntry, PlayerSettings, PlayerSnapshot, Repeat,
    TrackInput,
};

use super::PlayerState;

type CmdResult<T> = Result<T, String>;

fn err(e: volna_player::Error) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct Rev {
    rev: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepReply {
    ends_at: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum At {
    Next,
    End,
}

#[tauri::command]
pub async fn player_snapshot(p: State<'_, PlayerState>) -> CmdResult<PlayerSnapshot> {
    p.handle()?.snapshot().await.map_err(err)
}

#[tauri::command]
pub async fn player_play(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.play().await.map_err(err)
}

#[tauri::command]
pub async fn player_pause(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.pause().await.map_err(err)
}

#[tauri::command]
pub async fn player_toggle(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.toggle().await.map_err(err)
}

#[tauri::command]
pub async fn player_stop(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.stop().await.map_err(err)
}

#[tauri::command]
pub async fn player_next(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.next().await.map_err(err)
}

#[tauri::command]
pub async fn player_prev(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.prev().await.map_err(err)
}

#[tauri::command]
pub async fn player_seek(
    p: State<'_, PlayerState>,
    position: f64,
    resume: Option<bool>,
) -> CmdResult<()> {
    p.handle()?
        .seek(position, resume.unwrap_or(false))
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn player_play_uid(
    p: State<'_, PlayerState>,
    uid: String,
    position: Option<f64>,
) -> CmdResult<()> {
    p.handle()?.play_uid(uid, position).await.map_err(err)
}

#[tauri::command]
pub async fn player_set_queue(
    p: State<'_, PlayerState>,
    tracks: Vec<TrackInput>,
    start_index: Option<i64>,
    play: Option<bool>,
    position: Option<f64>,
) -> CmdResult<Rev> {
    let start = start_index.unwrap_or(0).max(0) as usize;
    let rev = p
        .handle()?
        .set_queue(tracks, start, play.unwrap_or(false), position)
        .await
        .map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_enqueue(
    p: State<'_, PlayerState>,
    tracks: Vec<TrackInput>,
    at: At,
) -> CmdResult<Rev> {
    let at = match at {
        At::Next => EnqueueAt::Next,
        At::End => EnqueueAt::End,
    };
    let rev = p.handle()?.enqueue(tracks, at).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_remove(p: State<'_, PlayerState>, uid: String) -> CmdResult<Rev> {
    let rev = p.handle()?.remove(uid).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_move(p: State<'_, PlayerState>, uid: String, to: i64) -> CmdResult<Rev> {
    if to < 0 {
        return Err("bad_args: negative index".into());
    }
    let rev = p.handle()?.move_to(uid, to as usize).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_clear(p: State<'_, PlayerState>, keep_current: bool) -> CmdResult<Rev> {
    let rev = p.handle()?.clear(keep_current).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_set_shuffle(p: State<'_, PlayerState>, on: bool) -> CmdResult<Rev> {
    let rev = p.handle()?.set_shuffle(on).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_set_repeat(p: State<'_, PlayerState>, mode: Repeat) -> CmdResult<Rev> {
    let rev = p.handle()?.set_repeat(mode).await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_set_settings(
    p: State<'_, PlayerState>,
    patch: serde_json::Map<String, serde_json::Value>,
) -> CmdResult<PlayerSettings> {
    p.handle()?.set_settings(patch, false).await.map_err(err)
}

#[tauri::command]
pub async fn player_set_dsp(p: State<'_, PlayerState>, config: DspConfig) -> CmdResult<()> {
    p.handle()?.set_dsp(config).await.map_err(err)
}

#[tauri::command]
pub async fn player_set_sleep_timer(
    p: State<'_, PlayerState>,
    minutes: Option<f64>,
) -> CmdResult<SleepReply> {
    let ends_at = p.handle()?.set_sleep_timer(minutes).await.map_err(err)?;
    Ok(SleepReply { ends_at })
}

#[tauri::command]
pub async fn player_set_continuation(
    p: State<'_, PlayerState>,
    kind: ContinuationKind,
    seeds: Option<Vec<TrackInput>>,
) -> CmdResult<()> {
    p.handle()?
        .set_continuation(kind, seeds.unwrap_or_default())
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn player_wave_start(p: State<'_, PlayerState>, count: Option<usize>) -> CmdResult<Rev> {
    let rev = p
        .handle()?
        .wave_start(count.unwrap_or(5))
        .await
        .map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_wave_stop(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.wave_stop().await.map_err(err)
}

#[tauri::command]
pub async fn player_wave_requeue(p: State<'_, PlayerState>) -> CmdResult<Rev> {
    let rev = p.handle()?.wave_requeue().await.map_err(err)?;
    Ok(Rev { rev })
}

#[tauri::command]
pub async fn player_wave_dislike(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.wave_dislike().await.map_err(err)
}

/// Binary §3.5 frames over `channel` until `player_visualizer_stop`, a new
/// subscription, or a failed send (the page went away).
#[tauri::command]
pub async fn player_visualizer_start(
    p: State<'_, PlayerState>,
    channel: Channel<InvokeResponseBody>,
    fps: Option<u32>,
) -> CmdResult<()> {
    let sink = Box::new(move |frame: Vec<u8>| channel.send(InvokeResponseBody::Raw(frame)).is_ok());
    p.handle()?
        .visualizer_start(fps.unwrap_or(60), sink)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn player_visualizer_stop(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.handle()?.visualizer_stop().await.map_err(err)
}

#[tauri::command]
pub async fn player_journal(
    p: State<'_, PlayerState>,
    after: Option<u64>,
) -> CmdResult<Vec<JournalEntry>> {
    Ok(p.journal.after(after.unwrap_or(0)))
}

#[tauri::command]
pub async fn player_journal_ack(p: State<'_, PlayerState>, up_to: u64) -> CmdResult<()> {
    p.journal.ack(up_to);
    Ok(())
}

#[tauri::command]
pub async fn app_ready_to_close(p: State<'_, PlayerState>) -> CmdResult<()> {
    p.close.ready();
    Ok(())
}

#[tauri::command]
pub async fn app_quit(app: AppHandle) -> CmdResult<()> {
    super::window::quit(&app);
    Ok(())
}
