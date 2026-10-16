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
use volna_streams::Cookies;

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
        tracing::info!(path = %self.store.path().display(), network = ?settings.network, cookies = ?settings.ytdlp_cookies, "restoring settings");
        if let Some(streams) = &app.streams
            && let Err(e) = streams.backend().set_cookies(settings.ytdlp_cookies)
        {
            // E.g. the cookies file was moved: play without cookies.
            tracing::error!("cannot apply the saved yt-dlp cookies: {e}");
            self.set_startup_error(Some(e.to_string()));
        }
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

    /// Switches yt-dlp's cookie source and saves it. Cached stream URLs and
    /// failures are dropped so the next play tries again with the cookies.
    pub async fn set_cookies(&self, app: &AppState, cookies: Cookies) -> CmdResult<()> {
        let _guard = self.applying.lock().await;
        let streams = app.streams.as_ref().ok_or("yt-dlp not found")?;
        streams
            .backend()
            .set_cookies(cookies.clone())
            .map_err(|e| e.to_string())?;
        streams.clear_cache();
        let mut settings = self.store.load().unwrap_or_default();
        settings.ytdlp_cookies = cookies;
        self.store.save(&settings).map_err(|e| e.to_string())
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
    /// Where yt-dlp takes YouTube cookies from.
    cookies: Cookies,
}

#[tauri::command]
pub fn net_settings(app: State<'_, AppState>, net: State<'_, NetworkState>) -> NetSettingsView {
    NetSettingsView {
        config: app.net.config(),
        status: app.net.status(),
        startup_error: net.startup_error(),
        cookies: app
            .streams
            .as_ref()
            .map(|s| s.backend().cookies())
            .unwrap_or_default(),
    }
}

/// `cookies` is `{"mode":"none"}`, `{"mode":"browser","value":"firefox"}`
/// or `{"mode":"file","value":"/path/cookies.txt"}`.
#[tauri::command]
pub async fn net_set_cookies(
    app: State<'_, AppState>,
    net: State<'_, NetworkState>,
    cookies: Cookies,
) -> CmdResult<()> {
    net.set_cookies(&app, cookies).await
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

/// A popular, region-free music video used to test audio resolution.
/// (Not "Never Gonna Give You Up": YouTube lets that one through even from
/// IPs that are bot-checked for everything else.)
const AUDIO_PROBE_ID: &str = "kJQP7kiw5Fk";

#[derive(Debug, Serialize)]
pub struct AudioCheck {
    ok: bool,
    /// Set when `ok` is false.
    kind: Option<volna_streams::FailureKind>,
    message: Option<String>,
    latency_ms: u64,
}

/// Can audio be resolved through the current network? The catalogue may
/// work while yt-dlp is refused with a bot check, so «Проверить» asks both.
#[tauri::command]
pub async fn net_audio_check(app: State<'_, AppState>) -> CmdResult<AudioCheck> {
    let Some(streams) = &app.streams else {
        return Ok(AudioCheck {
            ok: false,
            kind: Some(volna_streams::FailureKind::NoYtDlp),
            message: Some("yt-dlp not found".to_owned()),
            latency_ms: 0,
        });
    };
    let id = volna_streams::VideoId::parse(AUDIO_PROBE_ID).map_err(|e| e.to_string())?;
    let started = std::time::Instant::now();
    let result = streams.resolve(&id, volna_streams::Quality::High).await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    Ok(match result {
        Ok(_) => AudioCheck {
            ok: true,
            kind: None,
            message: None,
            latency_ms,
        },
        Err(e) => AudioCheck {
            ok: false,
            kind: Some(e.kind()),
            message: Some(e.to_string()),
            latency_ms,
        },
    })
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
    async fn cookies_are_saved_and_restored() {
        let (dir, store) = temp_store("cookies");
        let net = NetworkState::new(store.clone());
        let app = app();
        if app.streams.is_none() {
            return; // no yt-dlp on this machine
        }
        let cookies = Cookies::Browser("firefox".into());
        net.set_cookies(&app, cookies.clone()).await.unwrap();
        assert_eq!(store.load().unwrap().ytdlp_cookies, cookies);
        // A bad value is rejected and the saved one stays.
        assert!(
            net.set_cookies(&app, Cookies::Browser("bogus".into()))
                .await
                .is_err()
        );
        assert_eq!(store.load().unwrap().ytdlp_cookies, cookies);

        let app2 = AppState::new(&dir.join("cache")).unwrap();
        NetworkState::new(store).restore(&app2).await;
        assert_eq!(app2.streams.unwrap().backend().cookies(), cookies);
        std::fs::remove_dir_all(dir).unwrap();
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
