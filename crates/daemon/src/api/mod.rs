//! The native API the Flutter window (and later `volna-core`) calls
//! (`docs/flutter-ui-plan.md`). Handlers here only take SQLite / `ytm` /
//! `net` handles, never GStreamer or the socket.

pub mod library;

use serde_json::Value;

use crate::daemon::Daemon;

/// Runs `cmd` if one of the API modules has it.
pub async fn call(d: &Daemon, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    library::call(&d.library, cmd, a).await
}
