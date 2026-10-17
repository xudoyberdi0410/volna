//! Native API handlers for the Flutter UI (`docs/flutter-ui-plan.md`).
//! Each module answers its own commands and returns `None` for the rest.

use serde_json::Value;

use crate::daemon::Daemon;

pub mod lyrics;

/// The handler of `cmd`, if one of these modules owns it.
pub async fn call(d: &Daemon, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    if let Some(r) = lyrics::call(d, cmd, a).await {
        return Some(r);
    }
    None
}
