//! Tauri command names → handlers. The window process forwards every
//! `invoke` it does not handle itself (`yandex_export`, `app_ready_to_close`)
//! as a [`crate::ipc::Msg::Call`]; `volna_core` runs it with the same names,
//! argument keys and results. Only `app_quit` is the daemon's own.

use serde_json::Value;

pub use volna_core::dispatch::{arg, opt, out};

use crate::daemon::Daemon;

pub async fn call(d: &std::sync::Arc<Daemon>, cmd: &str, a: &Value) -> Result<Value, String> {
    if cmd == "app_quit" {
        d.quit();
        return Ok(Value::Null);
    }
    let vis = {
        let d = d.clone();
        move || -> volna_player::FrameSink { Box::new(move |frame: Vec<u8>| d.send_vis(frame)) }
    };
    d.core.call_with_vis(cmd, a, vis).await
}
