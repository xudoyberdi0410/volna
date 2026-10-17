//! Native API handlers for the Flutter window (`docs/flutter-ui-plan.md`).
//! Modules here take `ytm`/`net`/SQLite handles only, never the player or the
//! socket, so they move into `volna-core` for mobile as they are.

pub mod yandex;
