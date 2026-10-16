//! Network setting: restore on startup, hot-swap from the UI, and the Tauri
//! commands behind Settings → «Сеть».
//!
//! Switching the network means three things besides `NetManager::apply`:
//! the InnerTube client must pick up the new reqwest client, cached
//! googlevideo URLs are dropped (they only work from the IP that resolved
//! them), and the choice is saved so the next start uses it.

use std::sync::Mutex;

use serde::Serialize;
use tauri::State;
use volna_net::{HealthReport, NetStatus, ProxyConfig};

use crate::settings::SettingsStore;
use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

/// Managed next to [`AppState`]: where settings live and how the startup
/// restore went.
pub struct NetworkState {
    store: SettingsStore,
    /// Why the saved setting could not be applied at startup (the app then
    /// stays in direct mode). Cleared by the next successful apply.
    startup_error: Mutex<Option<String>>,
    /// Serialises apply+save, so two quick clicks cannot save the loser.
    applying: tokio::sync::Mutex<()>,
}

impl NetworkState {
    pub fn new(store: SettingsStore) -> Self {
        Self {
            store,
            startup_error: Mutex::new(None),
            applying: tokio::sync::Mutex::new(()),
        }
    }

    fn set_startup_error(&self, error: Option<String>) {
        *self.startup_error.lock().unwrap_or_else(|e| e.into_inner()) = error;
    }

    fn startup_error(&self) -> Option<String> {
        self.startup_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Loads the saved setting and applies it. Never fails: errors are logged
    /// and kept for the settings page, and the app stays in direct mode.
    pub async fn restore(&self, app: &AppState) {
        let _guard = self.applying.lock().await;
        let settings = match self.store.load() {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("{e}");
                self.set_startup_error(Some(e.to_string()));
                return;
            }
        };
        tracing::info!(path = %self.store.path().display(), network = ?settings.network, "restoring settings");
        if settings.network == ProxyConfig::Direct {
            return;
        }
        if let Err(e) = switch(app, settings.network).await {
            tracing::error!("cannot apply the saved network setting: {e}");
            self.set_startup_error(Some(e));
        }
    }

    /// Validates, applies and, on success, saves `config`.
    pub async fn apply(&self, app: &AppState, config: ProxyConfig) -> CmdResult<NetStatus> {
        let _guard = self.applying.lock().await;
        switch(app, config.clone()).await?;
        self.set_startup_error(None);
        let mut settings = self.store.load().unwrap_or_default();
        settings.network = config;
        self.store.save(&settings).map_err(|e| e.to_string())?;
        Ok(app.net.status())
    }
}

/// Hot-swaps the network and everything that depends on the exit IP.
async fn switch(app: &AppState, config: ProxyConfig) -> CmdResult<()> {
    config.validate().map_err(|e| e.to_string())?;
    app.net.apply(config).await.map_err(|e| e.to_string())?;
    app.api.refresh_http_client().await;
    if let Some(streams) = &app.streams {
        streams.clear_cache();
    }
    Ok(())
}

#[derive(Serialize)]
pub struct NetSettingsView {
    /// The saved/active setting, unredacted, so the form can be edited.
    config: ProxyConfig,
    status: NetStatus,
    startup_error: Option<String>,
}

#[tauri::command]
pub fn net_settings(app: State<'_, AppState>, net: State<'_, NetworkState>) -> NetSettingsView {
    NetSettingsView {
        config: app.net.config(),
        status: app.net.status(),
        startup_error: net.startup_error(),
    }
}

/// `config` is `{"mode":"direct"}` or `{"mode":"url"|"vless","value":"..."}`.
#[tauri::command]
pub async fn net_apply(
    app: State<'_, AppState>,
    net: State<'_, NetworkState>,
    config: ProxyConfig,
) -> CmdResult<NetStatus> {
    net.apply(&app, config).await
}

/// Checks the link without applying it (no network, no xray).
#[tauri::command]
pub fn net_validate(config: ProxyConfig) -> CmdResult<()> {
    config.validate().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn net_health(app: State<'_, AppState>) -> CmdResult<HealthReport> {
    Ok(app.net.health_check().await)
}

#[tauri::command]
pub fn net_xray_logs(app: State<'_, AppState>) -> Vec<String> {
    app.net.xray_logs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn temp_store(name: &str) -> (std::path::PathBuf, SettingsStore) {
        let dir = std::env::temp_dir().join(format!("volna-network-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = SettingsStore::new(&dir);
        (dir, store)
    }

    fn app() -> AppState {
        let cache =
            std::env::temp_dir().join(format!("volna-network-cache-{}", std::process::id()));
        AppState::new(&cache).unwrap()
    }

    #[tokio::test]
    async fn apply_saves_and_switches() {
        let (dir, store) = temp_store("apply");
        let net = NetworkState::new(store.clone());
        let app = app();
        let config = ProxyConfig::Url("socks5h://127.0.0.1:9".into());
        let status = net.apply(&app, config.clone()).await.unwrap();
        assert_eq!(status.mode, "url");
        assert_eq!(app.net.config(), config);
        assert_eq!(store.load().unwrap().network, config);

        // A fresh start restores it.
        let app2 = AppState::new(&dir.join("cache")).unwrap();
        NetworkState::new(store).restore(&app2).await;
        assert_eq!(app2.net.config(), config);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn invalid_setting_is_not_saved() {
        let (dir, store) = temp_store("invalid");
        let net = NetworkState::new(store.clone());
        let app = app();
        let err = net
            .apply(&app, ProxyConfig::Vless("vless://broken".into()))
            .await
            .unwrap_err();
        assert!(err.contains("vless"), "{err}");
        assert_eq!(app.net.config(), ProxyConfig::Direct);
        assert_eq!(store.load().unwrap(), Settings::default());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn broken_saved_file_is_reported_not_fatal() {
        let (dir, store) = temp_store("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(store.path(), "nope").unwrap();
        let net = NetworkState::new(store);
        let app = app();
        net.restore(&app).await;
        assert_eq!(app.net.config(), ProxyConfig::Direct);
        assert!(net.startup_error().is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
