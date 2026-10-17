//! §3.3 commands, dispatched from IPC calls. Arguments arrive with the JS
//! (camelCase) keys the page passes to `invoke`. Errors are
//! `"<code>: <message>"`; every command replies as soon as the player state
//! is updated.

use serde::Deserialize;
use serde_json::{Value, json};
use volna_player::{ContinuationKind, DspConfig, EnqueueAt, FrameSink, Repeat, TrackInput};

use super::PlayerState;
use crate::dispatch::{arg, opt, out};

fn err(e: volna_player::Error) -> String {
    e.to_string()
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum At {
    Next,
    End,
}

fn rev(r: Result<u64, volna_player::Error>) -> Result<Value, String> {
    r.map(|rev| json!({ "rev": rev })).map_err(err)
}

/// `None` when `cmd` is not a player command. `vis_sink` makes the frame
/// sink for `player_visualizer_start` (frames go to the connected window).
pub async fn call(
    p: &PlayerState,
    cmd: &str,
    a: &Value,
    vis_sink: impl FnOnce() -> FrameSink,
) -> Option<Result<Value, String>> {
    if !cmd.starts_with("player_") {
        return None;
    }
    let journal = &p.journal;
    let r: Result<Value, String> = async {
        match cmd {
            "player_journal" => {
                return out(Ok::<_, String>(
                    journal.after(opt(a, "after")?.unwrap_or(0)),
                ));
            }
            "player_journal_ack" => {
                journal.ack(arg(a, "upTo")?);
                return Ok(Value::Null);
            }
            _ => {}
        }
        let h = p.handle()?;
        match cmd {
            "player_snapshot" => out(h.snapshot().await.map_err(err)),
            "player_play" => out(h.play().await.map_err(err)),
            "player_pause" => out(h.pause().await.map_err(err)),
            "player_toggle" => out(h.toggle().await.map_err(err)),
            "player_stop" => out(h.stop().await.map_err(err)),
            "player_next" => out(h.next().await.map_err(err)),
            "player_prev" => out(h.prev().await.map_err(err)),
            "player_seek" => out(h
                .seek(arg(a, "position")?, opt(a, "resume")?.unwrap_or(false))
                .await
                .map_err(err)),
            "player_play_uid" => out(h
                .play_uid(arg(a, "uid")?, opt(a, "position")?)
                .await
                .map_err(err)),
            "player_set_queue" => {
                let tracks: Vec<TrackInput> = arg(a, "tracks")?;
                let start = opt::<i64>(a, "startIndex")?.unwrap_or(0).max(0) as usize;
                rev(h
                    .set_queue(
                        tracks,
                        start,
                        opt(a, "play")?.unwrap_or(false),
                        opt(a, "position")?,
                    )
                    .await)
            }
            "player_enqueue" => {
                let at = match arg::<At>(a, "at")? {
                    At::Next => EnqueueAt::Next,
                    At::End => EnqueueAt::End,
                };
                rev(h.enqueue(arg(a, "tracks")?, at).await)
            }
            "player_remove" => rev(h.remove(arg(a, "uid")?).await),
            "player_move" => {
                let to: i64 = arg(a, "to")?;
                if to < 0 {
                    return Err("bad_args: negative index".into());
                }
                rev(h.move_to(arg(a, "uid")?, to as usize).await)
            }
            "player_clear" => rev(h.clear(arg(a, "keepCurrent")?).await),
            "player_set_shuffle" => rev(h.set_shuffle(arg(a, "on")?).await),
            "player_set_repeat" => rev(h.set_repeat(arg::<Repeat>(a, "mode")?).await),
            "player_set_settings" => {
                out(h.set_settings(arg(a, "patch")?, false).await.map_err(err))
            }
            "player_set_dsp" => out(h.set_dsp(arg::<DspConfig>(a, "config")?).await.map_err(err)),
            "player_set_sleep_timer" => h
                .set_sleep_timer(opt(a, "minutes")?)
                .await
                .map(|ends_at| json!({ "endsAt": ends_at }))
                .map_err(err),
            "player_set_continuation" => out(h
                .set_continuation(
                    arg::<ContinuationKind>(a, "kind")?,
                    opt(a, "seeds")?.unwrap_or_default(),
                )
                .await
                .map_err(err)),
            "player_wave_start" => rev(h.wave_start(opt(a, "count")?.unwrap_or(5)).await),
            "player_wave_stop" => out(h.wave_stop().await.map_err(err)),
            "player_wave_dislike" => out(h.wave_dislike().await.map_err(err)),
            "player_wave_requeue" => rev(h.wave_requeue().await),
            "player_visualizer_start" => out(h
                .visualizer_start(opt(a, "fps")?.unwrap_or(60), vis_sink())
                .await
                .map_err(err)),
            "player_visualizer_stop" => out(h.visualizer_stop().await.map_err(err)),
            other => Err(format!("unknown command {other}")),
        }
    }
    .await;
    Some(r)
}
