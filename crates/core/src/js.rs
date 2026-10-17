//! JavaScript the core cannot run itself. The mobile stream resolver
//! (`volna_streams::innertube`) needs a JS engine for YouTube's signature /
//! `n` challenges and to mint PO tokens with BotGuard; the platform has one
//! in its WebView. The trait is [`JsRuntime`] from `volna-streams` (one
//! definition, re-exported here); over FFI it is [`CallbackJs`]: a request
//! goes out through `volna_core_set_js_callback`'s callback, the answer comes
//! back with `volna_core_js_result`.
//!
//! The host loads [`PRELUDE`] once into its JS context (also available as
//! `volna_core_js_prelude()`), then for each request evaluates
//! `volnaJs(<request JSON>)` and answers with the string the promise
//! resolves to. The JSON is documented in `docs/streams.md` ("JsRuntime
//! contract").

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::oneshot;

pub use volna_streams::js::{JsError, JsFuture, JsRequest, JsRuntime, PRELUDE};

type Sender = Box<dyn Fn(u64, String) + Send + Sync>;

/// [`JsRuntime`] over a callback `(request id, request JSON)` and
/// [`CallbackJs::complete`] for the answer.
#[derive(Default)]
pub struct CallbackJs {
    send: Mutex<Option<Arc<Sender>>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<String, JsError>>>>,
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

    /// The platform's answer to request `id`: when `ok`, `text` is the
    /// string `volnaJs` resolved to (passed on unchanged); otherwise why the
    /// script could not run at all (JSON string or plain text). False when
    /// nothing waits for `id`.
    pub fn complete(&self, id: u64, ok: bool, text: &str) -> bool {
        let Some(tx) = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
        else {
            return false;
        };
        let result = if ok {
            Ok(text.to_owned())
        } else {
            Err(JsError::runtime(
                match serde_json::from_str::<serde_json::Value>(text) {
                    Ok(serde_json::Value::String(s)) => s,
                    _ => text.to_owned(),
                },
            ))
        };
        tx.send(result).is_ok()
    }
}

impl JsRuntime for CallbackJs {
    fn call(&self, request: String) -> JsFuture<'_> {
        let send = self.send.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(send) = send else {
            return Box::pin(async { Err(JsError::runtime("no JS runtime registered")) });
        };
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, tx);
        send(id, request);
        Box::pin(async move {
            rx.await
                .unwrap_or_else(|_| Err(JsError::runtime("the JS runtime went away")))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn requests_round_trip_through_the_callback() {
        let js = CallbackJs::new();
        assert!(js.call("{}".into()).await.is_err());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let s = sent.clone();
        js.set_sender(Some(move |id, req: String| {
            s.lock().unwrap().push((id, req))
        }));
        let a = js.call(JsRequest::Version.to_json());
        let b = js.call(JsRequest::Version.to_json());
        let (ida, reqa) = sent.lock().unwrap()[0].clone();
        let (idb, _) = sent.lock().unwrap()[1].clone();
        assert_eq!(reqa, r#"{"op":"version"}"#);
        assert!(js.complete(idb, false, "\"boom\""));
        assert!(js.complete(ida, true, r#"{"ok":{"version":1}}"#));
        assert!(!js.complete(ida, true, "2"), "answered once");
        assert_eq!(a.await.unwrap(), r#"{"ok":{"version":1}}"#);
        let err = b.await.unwrap_err();
        assert!(err.is("runtime") && err.message == "boom", "{err}");
    }
}
