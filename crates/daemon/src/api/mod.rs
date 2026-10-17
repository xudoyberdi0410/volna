//! Native API for the Flutter window (`docs/flutter-ui-plan.md`): commands
//! that return `crates/ytm` models and other plain data directly, instead of
//! the hifi-api shapes of `api_emu`.
//!
//! Modules here take only `ytm` / `net` / image-cache / SQLite handles, never
//! the player, the socket or desktop-only crates: they move to `volna-core`
//! for mobile later. Each module answers its own commands and returns `None`
//! for the rest.
//!
//! Errors are strings `"<kind>: <message>"` like the `player_*` commands:
//! `bad_args`, `not_found`, `upstream`, `internal`.

use serde_json::Value;

use crate::daemon::Daemon;

pub mod catalog;
pub mod color;
pub mod library;
pub mod lyrics;
pub mod scrobble;

/// The handler of `cmd`, if one of these modules owns it.
pub async fn call(d: &Daemon, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    if let Some(r) = catalog::call(&d.app.api.ytm(), cmd, a).await {
        return Some(r);
    }
    if let Some(r) = color::call(&d.app.palettes, &d.app.images, cmd, a).await {
        return Some(r);
    }
    if let Some(r) = library::call(&d.library, cmd, a).await {
        return Some(r);
    }
    if let Some(r) = lyrics::call(d, cmd, a).await {
        return Some(r);
    }
    if cmd == "scrobble_import_legacy" {
        // The scrobbler keys `legacy_import` kept in the library database.
        return Some(scrobble::import_legacy_from_library(
            &d.scrobble,
            &d.library,
        ));
    }
    if let Some(r) = scrobble::call(&d.scrobble, cmd, a).await {
        return Some(r);
    }
    None
}
