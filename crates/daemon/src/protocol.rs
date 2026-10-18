//! `volna://` custom protocol: the window's only door to the outside world
//! (covers through `volna://img/`, audio through `volna://stream/`). The
//! handler is `volna_core::protocol`; the window sends requests as IPC
//! `proto` frames.

pub use volna_core::protocol::{Route, Target, handle, json_error, with_cors};
