//! Pure JSON → model conversion. Nothing here does I/O, so every parser is
//! unit-tested against captured responses in `tests/fixtures`.
//!
//! Rule of thumb for all parsers: a missing or unknown field skips the item
//! (or leaves the field empty); it never fails the whole response.

pub mod browse;
pub mod items;
pub mod json;
pub mod pages;
pub mod runs;
pub mod search;
pub mod watch;
