//! Native API for the Flutter window (`docs/flutter-ui-plan.md`): commands
//! that return `crates/ytm` models and other plain data directly, instead of
//! the hifi-api shapes of `api_emu`.
//!
//! Modules here take only `ytm` / `net` / image-cache / SQLite handles, never
//! the player, the socket or desktop-only crates: they move to `volna-core`
//! for mobile later.
//!
//! Errors are strings `"<kind>: <message>"` like the `player_*` commands:
//! `bad_args`, `not_found`, `upstream`, `internal`.

pub mod catalog;
pub mod color;
