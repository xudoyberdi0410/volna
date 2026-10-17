//! Player events → the window process (§3.4), only while one is connected.
//! Without a window nothing crosses IPC.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use volna_player::{PlayerEvent, PlayerHandle};

use super::journal::Journal;

/// Where events go: the connected window, if any.
pub trait EventSink: Send + Sync + 'static {
    fn connected(&self) -> bool;
    fn emit(&self, name: &str, payload: Value);
}

fn to_value<T: serde::Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The §3.4 name and payload of an event, `None` for internal ones.
pub fn event_payload(ev: PlayerEvent) -> Option<(&'static str, Value)> {
    Some(match ev {
        PlayerEvent::State(s) => ("player:state", to_value(s)),
        PlayerEvent::Position(p) => ("player:position", to_value(p)),
        PlayerEvent::Queue(q) => ("player:queue", to_value(q)),
        PlayerEvent::Error(e) => ("player:error", to_value(e)),
        PlayerEvent::Settings {
            settings,
            dsp,
            external: true,
        } => (
            "player:settings",
            json!({ "settings": settings, "dsp": dsp }),
        ),
        PlayerEvent::Settings { .. } | PlayerEvent::Seeked { .. } => return None,
        PlayerEvent::Sleep { ends_at } => ("player:sleep", json!({ "endsAt": ends_at })),
        PlayerEvent::Wave { active } => ("player:wave", json!({ "active": active })),
        PlayerEvent::QueueLow { remaining } => {
            ("player:queue-low", json!({ "remaining": remaining }))
        }
    })
}

pub fn start(sink: Arc<dyn EventSink>, handle: Option<PlayerHandle>, journal: &Journal) {
    {
        let sink = sink.clone();
        journal.set_listener(move |entry| {
            if sink.connected() {
                sink.emit("player:journal", to_value(entry));
            }
        });
    }
    let Some(handle) = handle else {
        return;
    };
    let mut rx = handle.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                // The UI resyncs on a rev gap (§3.6).
                Err(RecvError::Lagged(n)) => {
                    tracing::debug!("event forwarder lagged by {n}");
                    continue;
                }
                Err(RecvError::Closed) => break,
            };
            if !sink.connected() {
                continue;
            }
            if let Some((name, payload)) = event_payload(ev) {
                sink.emit(name, payload);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_names_follow_the_contract() {
        let (n, p) = event_payload(PlayerEvent::Sleep { ends_at: Some(5.0) }).unwrap();
        assert_eq!(n, "player:sleep");
        assert_eq!(p, json!({"endsAt": 5.0}));
        let (n, p) = event_payload(PlayerEvent::QueueLow { remaining: 2 }).unwrap();
        assert_eq!((n, p), ("player:queue-low", json!({"remaining": 2})));
        assert!(
            event_payload(PlayerEvent::Seeked {
                uid: None,
                position: 1.0
            })
            .is_none()
        );
    }
}
