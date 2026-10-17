//! JavaScript the core cannot run itself. The mobile stream resolver needs
//! a JS engine for YouTube's signature / `n` challenges and to mint PO
//! tokens with BotGuard; the platform has one in its WebView. The core asks
//! through [`JsRuntime`]; over FFI that is [`CallbackJs`]: a request goes
//! out through `volna_core_set_js_callback`'s callback, the answer comes
//! back with `volna_core_js_result`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::oneshot;

pub type JsFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

/// Runs a script in the platform's JS engine. `request` is a JSON job, by
/// convention `{"script": "<source>", "args": …}`; the result is the JSON
/// value the script produced, or an error message.
pub trait JsRuntime: Send + Sync + 'static {
    fn eval(&self, request: Value) -> JsFuture;
}

type Sender = Box<dyn Fn(u64, String) + Send + Sync>;

/// [`JsRuntime`] over a callback `(request id, request JSON)` and
/// [`CallbackJs::complete`] for the answer.
#[derive(Default)]
pub struct CallbackJs {
    send: Mutex<Option<Arc<Sender>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next: AtomicU64,
}

impl CallbackJs {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Where requests go; `None` makes every request fail right away.
    pub fn set_sender(&self, send: Option<impl Fn(u64, String) + Send + Sync + 'static>) {
        let send = send.map(|f| Arc::new(Box::new(f) as Sender));
        *self.send.lock().unwrap_or_else(|e| e.into_inner()) = send;
    }

    /// The platform's answer to request `id`: `json` is the result when
    /// `ok`, an error message (JSON string or plain text) otherwise.
    /// False when nothing waits for `id`.
    pub fn complete(&self, id: u64, ok: bool, json: &str) -> bool {
        let Some(tx) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        else {
            return false;
        };
        let result = if ok {
            serde_json::from_str(json).map_err(|e| format!("bad JS result: {e}"))
        } else {
            Err(match serde_json::from_str::<Value>(json) {
                Ok(Value::String(s)) => s,
                _ => json.to_owned(),
            })
        };
        tx.send(result).is_ok()
    }
}

impl JsRuntime for CallbackJs {
    fn eval(&self, request: Value) -> JsFuture {
        let send = self.send.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(send) = send else {
            return Box::pin(async { Err("no JS runtime registered".to_owned()) });
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        send(id, request.to_string());
        Box::pin(async move {
            rx.await
                .unwrap_or_else(|_| Err("the JS runtime went away".to_owned()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn requests_round_trip_through_the_callback() {
        let js = CallbackJs::new();
        assert!(js.eval(json!({})).await.is_err());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s = sent.clone();
        js.set_sender(Some(move |id, req: String| s.lock().unwrap().push((id, req))));
        let a = js.eval(json!({"script": "1+1"}));
        let b = js.eval(json!({"script": "throw 1"}));
        let (ida, reqa) = sent.lock().unwrap()[0].clone();
        let (idb, _) = sent.lock().unwrap()[1].clone();
        assert_eq!(serde_json::from_str::<Value>(&reqa).unwrap()["script"], "1+1");
        assert!(js.complete(idb, false, "\"boom\""));
        assert!(js.complete(ida, true, "2"));
        assert!(!js.complete(ida, true, "2"), "answered once");
        assert_eq!(a.await.unwrap(), json!(2));
        assert_eq!(b.await.unwrap_err(), "boom");
    }
}
