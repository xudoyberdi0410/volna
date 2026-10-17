//! The core state plus the hifi-api emulator for the old webview UI.
//! Derefs to [`volna_core::AppState`], so `state.net`, `state.streams` and
//! the core functions taking `&volna_core::AppState` work on it directly.

use std::path::Path;
use std::sync::Arc;

pub use volna_core::state::{Images, StreamBackend, Streams};

use crate::api_emu::ApiEmu;

#[derive(Clone)]
pub struct AppState {
    pub core: volna_core::AppState,
    /// `volna://api/` (hifi-api emulation over YouTube Music).
    pub api: Arc<ApiEmu>,
}

impl AppState {
    /// `cache_dir` is the app cache directory; covers go to `<cache_dir>/images`.
    pub fn new(cache_dir: &Path) -> anyhow::Result<Self> {
        Ok(Self::from_core(volna_core::AppState::new(cache_dir)?))
    }

    /// The emulator over the core's network and YouTube Music client.
    pub fn from_core(core: volna_core::AppState) -> Self {
        let api = Arc::new(ApiEmu::new(core.net.clone(), core.ytm.clone()));
        if let Some(base) = &core.stream_base {
            api.set_stream_base(base.clone());
        }
        Self { core, api }
    }
}

impl std::ops::Deref for AppState {
    type Target = volna_core::AppState;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}
