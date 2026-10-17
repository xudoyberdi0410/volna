//! Tauri command names → daemon handlers. The window process forwards every
//! `invoke` it does not handle itself (`yandex_export`, `app_ready_to_close`)
//! as a [`crate::ipc::Msg::Call`]; this module runs it with the same names,
//! argument keys (camelCase, as the page sends them) and results as the old
//! in-process commands.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::daemon::Daemon;
use crate::{commands, network, wave};

/// A required argument.
pub fn arg<T: DeserializeOwned>(a: &Value, key: &str) -> Result<T, String> {
    let v = a.get(key).cloned().unwrap_or(Value::Null);
    serde_json::from_value(v).map_err(|e| format!("invalid args `{key}`: {e}"))
}

/// An optional argument (missing or null → `None`).
pub fn opt<T: DeserializeOwned>(a: &Value, key: &str) -> Result<Option<T>, String> {
    match a.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("invalid args `{key}`: {e}")),
    }
}

/// A handler result as JSON.
pub fn out<T: Serialize>(r: Result<T, String>) -> Result<Value, String> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

pub async fn call(d: &std::sync::Arc<Daemon>, cmd: &str, a: &Value) -> Result<Value, String> {
    let vis = {
        let d = d.clone();
        move || -> volna_player::FrameSink { Box::new(move |frame: Vec<u8>| d.send_vis(frame)) }
    };
    if let Some(r) = crate::player::commands::call(&d.player, cmd, a, vis).await {
        return r;
    }
    if let Some(r) = crate::api::call(d, cmd, a).await {
        return r;
    }
    let (app, net, w) = (&d.app, &d.net, &d.wave);
    match cmd {
        "ytdlp_version" => out(commands::ytdlp_version(app).await),
        "ytdlp_update" => out(commands::ytdlp_update(app).await),
        "stream_prefetch" => {
            out(commands::stream_prefetch(app, arg(a, "videoId")?, opt(a, "quality")?).await)
        }
        "stream_base" => out(Ok(commands::stream_base(app))),
        "stream_failure" => out(commands::stream_failure(app, arg(a, "videoId")?)),
        "net_settings" => out(Ok(network::net_settings(app, net))),
        "net_set_cookies" => out(network::net_set_cookies(app, net, arg(a, "cookies")?).await),
        "net_apply" => out(network::net_apply(app, net, arg(a, "config")?).await),
        "net_validate" => out(network::net_validate(arg(a, "config")?)),
        "net_health" => out(network::net_health(app).await),
        "net_audio_check" => out(network::net_audio_check(app).await),
        "net_xray_logs" => out(Ok(network::net_xray_logs(app))),
        "wave_status" => out(wave::wave_status(w).await),
        "wave_set_settings" => out(wave::wave_set_settings(w, arg(a, "settings")?).await),
        "wave_activities" => out(wave::wave_activities(w).await),
        "wave_artist_search" => out(wave::wave_artist_search(w, arg(a, "query")?).await),
        "wave_related_artists" => out(wave::wave_related_artists(w, arg(a, "id")?).await),
        "wave_cold_start" => out(wave::wave_cold_start(w, arg(a, "artists")?).await),
        "wave_start" => out(wave::wave_start(w, opt(a, "count")?).await),
        "wave_next" => out(wave::wave_next(w, opt(a, "count")?).await),
        "wave_requeue" => out(wave::wave_requeue(w, arg(a, "dropped")?, opt(a, "count")?).await),
        "wave_stop" => out(wave::wave_stop(w).await),
        "wave_event" => {
            let ev: wave::UiEvent = arg(a, "event")?;
            // Likes reach the daemon here today: love on the scrobblers too.
            if let wave::UiEvent::Like { track, liked } = &ev {
                let t = volna_ytm::Track::from(track.clone());
                crate::api::scrobble::love_on_like(&d.scrobble, &t, *liked);
            }
            out(wave::wave_event(w, ev).await)
        }
        "wave_sync_likes" => out(wave::wave_sync_likes(w, arg(a, "tracks")?).await),
        "app_quit" => {
            d.quit();
            Ok(Value::Null)
        }
        other => Err(format!("unknown command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn args_by_js_key() {
        let a = json!({"videoId": "abc", "count": null, "keepCurrent": true});
        assert_eq!(arg::<String>(&a, "videoId").unwrap(), "abc");
        assert_eq!(opt::<usize>(&a, "count").unwrap(), None);
        assert_eq!(opt::<usize>(&a, "missing").unwrap(), None);
        assert!(arg::<bool>(&a, "keepCurrent").unwrap());
        assert!(
            arg::<String>(&a, "missing")
                .unwrap_err()
                .contains("missing")
        );
    }
}
