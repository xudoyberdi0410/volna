//! The JavaScript side of the resolver: a [`JsRuntime`] the host provides and
//! the JSON requests Rust sends it.
//!
//! YouTube's player hides the stream URLs behind JavaScript (the `n` and
//! signature challenges) and gates them with BotGuard (PO tokens). Rust does
//! every HTTP request; only the JavaScript runs elsewhere: in the platform
//! WebView on mobile (`evaluateJavaScript`), in Deno on desktop
//! ([`DenoRuntime`](super::DenoRuntime), feature `innertube-deno`).
//!
//! A runtime loads [`PRELUDE`] once, then answers each request by evaluating
//! `volnaJs(<request JSON>)` and returning the string that promise resolves
//! to. [`JsRequest::to_script`] builds that call. The exact JSON is
//! documented in `docs/streams.md` ("JsRuntime contract").

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The script a runtime evaluates once before any request: yt-dlp's EJS
/// challenge solver (Unlicense; bundles meriyah, ISC, and astring, MIT) and
/// Volna's dispatcher with the BotGuard client (after BgUtils, MIT). It
/// defines the global `volnaJs`. Licenses: `crates/streams/js/`.
pub const PRELUDE: &str = concat!(
    include_str!("../../js/ejs/yt.solver.lib.min.js"),
    "\nObject.assign(globalThis, lib);\n",
    include_str!("../../js/ejs/yt.solver.core.min.js"),
    "\nglobalThis.jsc = jsc;\n",
    include_str!("../../js/volna.js"),
);

/// Version `volnaJs({"op":"version"})` answers with; bumped when the
/// request or response shapes change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Boxed future of [`JsRuntime::call`].
pub type JsFuture<'a> = Pin<Box<dyn Future<Output = Result<String, JsError>> + Send + 'a>>;

/// Runs JavaScript for the resolver. Object safe, so hosts can hand in an
/// `Arc<dyn JsRuntime>`.
///
/// `request` is a [`JsRequest`] serialized as JSON. The runtime evaluates
/// `volnaJs(request)` in a context where [`PRELUDE`] is loaded (keeping that
/// context between calls: it caches the preprocessed player and the
/// BotGuard VM) and returns the string the promise resolves to. Only a
/// transport failure (no WebView, process died, timeout) is an `Err`; errors
/// raised by the scripts come back inside the string.
pub trait JsRuntime: Send + Sync {
    fn call(&self, request: String) -> JsFuture<'_>;
}

impl<T: JsRuntime + ?Sized> JsRuntime for Arc<T> {
    fn call(&self, request: String) -> JsFuture<'_> {
        (**self).call(request)
    }
}

impl<T: JsRuntime + ?Sized> JsRuntime for Box<T> {
    fn call(&self, request: String) -> JsFuture<'_> {
        (**self).call(request)
    }
}

/// A [`JsRuntime`] from a closure, for hosts that bridge to a callback (the
/// mobile FFI hook): the closure gets the request JSON and returns the
/// string `volnaJs` resolved to.
pub struct JsCallback<F>(pub F);

impl<F, Fut> JsRuntime for JsCallback<F>
where
    F: Fn(String) -> Fut + Send + Sync,
    Fut: Future<Output = Result<String, JsError>> + Send + 'static,
{
    fn call(&self, request: String) -> JsFuture<'_> {
        Box::pin((self.0)(request))
    }
}

/// A failure reported by the scripts or by the runtime itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct JsError {
    /// `need_player`, `unknown_session`, `solver`, `botguard`, `no_solver`,
    /// `bad_request`, `js_error` from the scripts; `runtime` for transport
    /// failures; `bad_response` when the answer is not the expected JSON.
    pub code: String,
    pub message: String,
}

impl JsError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    /// A transport failure: the runtime could not run the request at all.
    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new("runtime", message)
    }

    pub fn is(&self, code: &str) -> bool {
        self.code == code
    }
}

/// One request to `volnaJs`. Serialized with an `op` tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum JsRequest {
    /// Solve `n` and signature challenges with the EJS solver.
    Solve(SolveRequest),
    /// Run a BotGuard challenge and take a snapshot.
    BotguardSnapshot(SnapshotRequest),
    /// Mint PO tokens in a BotGuard session.
    BotguardMint(MintRequest),
    /// Answer [`PROTOCOL_VERSION`]; a cheap liveness check.
    Version,
}

/// `{"op":"solve", ...}` → [`SolveResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveRequest {
    /// Player id (`/s/player/<id>/`) the challenges belong to. The runtime
    /// keeps the preprocessed player under this id.
    pub player_id: String,
    /// The player's `base.js`. Sent only when the runtime does not have the
    /// player yet; without it the runtime answers `need_player`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub player: Option<Arc<str>>,
    /// `n` URL parameters to transform.
    #[serde(default)]
    pub n: Vec<String>,
    /// Encrypted signatures (`s` of a `signatureCipher`) to decrypt.
    #[serde(default)]
    pub sig: Vec<String>,
}

/// Results keyed by the challenge they answer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SolveResponse {
    #[serde(default)]
    pub n: HashMap<String, String>,
    #[serde(default)]
    pub sig: HashMap<String, String>,
}

/// `{"op":"botguard_snapshot", ...}` → [`SnapshotResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRequest {
    /// Name the runtime keeps this BotGuard VM under, for later mints.
    pub session: String,
    /// The interpreter script from the challenge (run with `new Function`).
    pub interpreter_js: String,
    /// The bytecode program for the VM.
    pub program: String,
    /// Global the interpreter defines (`globalThis[global_name].a(...)`).
    pub global_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotResponse {
    /// What Rust sends to `GenerateIT` for an integrity token.
    pub botguard_response: String,
}

/// `{"op":"botguard_mint", ...}` → [`MintResponse`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MintRequest {
    pub session: String,
    /// Integrity token from `GenerateIT` (base64).
    pub integrity_token: String,
    /// Values to bind tokens to: a video id, visitor data or data sync id.
    pub content_bindings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintResponse {
    /// base64url PO tokens, one per content binding, in order.
    pub po_tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionResponse {
    pub version: u32,
}

/// What `volnaJs` resolves to.
#[derive(Deserialize)]
#[serde(untagged)]
enum Envelope<T> {
    Ok { ok: T },
    Err { error: JsError },
}

impl JsRequest {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("JsRequest serializes")
    }

    /// The expression a WebView evaluates for this request:
    /// `volnaJs({...})`, a promise of the response string.
    pub fn to_script(&self) -> String {
        format!("volnaJs({})", self.to_json())
    }
}

/// Parses what `volnaJs` resolved to.
pub fn parse_response<T: DeserializeOwned>(raw: &str) -> Result<T, JsError> {
    match serde_json::from_str::<Envelope<T>>(raw) {
        Ok(Envelope::Ok { ok }) => Ok(ok),
        Ok(Envelope::Err { error }) => Err(error),
        Err(e) => Err(JsError::new(
            "bad_response",
            format!("{e}: {}", raw.chars().take(200).collect::<String>()),
        )),
    }
}

/// Sends `request` through `rt` and decodes the typed answer.
pub async fn request<T: DeserializeOwned>(
    rt: &dyn JsRuntime,
    request: &JsRequest,
) -> Result<T, JsError> {
    let raw = rt.call(request.to_json()).await?;
    parse_response(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_serialize_with_an_op_tag() {
        let solve = JsRequest::Solve(SolveRequest {
            player_id: "0229cd64".into(),
            player: None,
            n: vec!["abc".into()],
            sig: vec![],
        });
        assert_eq!(
            solve.to_json(),
            r#"{"op":"solve","player_id":"0229cd64","n":["abc"],"sig":[]}"#
        );
        assert!(solve.to_script().starts_with("volnaJs({\"op\":\"solve\""));

        let mint = JsRequest::BotguardMint(MintRequest {
            session: "bg1".into(),
            integrity_token: "tok".into(),
            content_bindings: vec!["dQw4w9WgXcQ".into()],
        });
        assert_eq!(
            mint.to_json(),
            r#"{"op":"botguard_mint","session":"bg1","integrity_token":"tok","content_bindings":["dQw4w9WgXcQ"]}"#
        );
        assert_eq!(JsRequest::Version.to_json(), r#"{"op":"version"}"#);
        let back: JsRequest = serde_json::from_str(&mint.to_json()).unwrap();
        assert_eq!(back, mint);
    }

    #[test]
    fn responses_decode_ok_and_error() {
        let ok: SolveResponse = parse_response(r#"{"ok":{"n":{"abc":"xyz"},"sig":{}}}"#).unwrap();
        assert_eq!(ok.n["abc"], "xyz");
        let err =
            parse_response::<SolveResponse>(r#"{"error":{"code":"need_player","message":"x"}}"#)
                .unwrap_err();
        assert!(err.is("need_player"));
        let bad = parse_response::<SolveResponse>("nonsense").unwrap_err();
        assert!(bad.is("bad_response"));
    }

    #[test]
    fn prelude_defines_the_entry_points() {
        assert!(PRELUDE.contains("var lib="));
        assert!(PRELUDE.contains("var jsc="));
        assert!(PRELUDE.contains("globalThis.volnaJs"));
    }

    #[tokio::test]
    async fn callback_runtime_round_trips() {
        let rt = JsCallback(|req: String| async move {
            assert!(req.contains(r#""op":"version""#));
            Ok::<_, JsError>(r#"{"ok":{"version":1}}"#.to_owned())
        });
        let v: VersionResponse = request(&rt, &JsRequest::Version).await.unwrap();
        assert_eq!(v.version, PROTOCOL_VERSION);
    }
}
