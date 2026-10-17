//! Import from Yandex Music for the old UI: get a token, then read the
//! account's library (`volna_daemon::api::yandex::client`, shared with the
//! daemon's own import).
//!
//! Two ways to the token:
//! - the browser: yt-dlp dumps the cookies of the browser set for YouTube in
//!   Settings → «Сеть», and `oauth.yandex.<tld>/authorize` with its Yandex session
//!   answers with the token straight away (the music client is auto-approved);
//! - a login window, when the browser has no Yandex session. The window keeps its
//!   cookies, so the next import only flashes it. It has no IPC capabilities.
//!
//! Matching the tracks to YouTube Music is the UI's job (`ui/js/yandex-import.js`).

use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use tokio::sync::oneshot;
use volna_daemon::api::yandex::browser;
use volna_daemon::api::yandex::client::{
    ApiError, Client, Library, authorize_url, export, token_from_url,
};

use crate::state::AppState;

type CmdResult<T> = Result<T, String>;

const LOGIN_WINDOW: &str = "yandex-login";

/// Managed state: the token of the last sign-in, kept for the app's lifetime.
#[derive(Default)]
pub struct YandexState {
    token: Mutex<Option<String>>,
    /// One import at a time; a second click gets an error instead of a second window.
    busy: tokio::sync::Mutex<()>,
}

impl YandexState {
    fn token(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_token(&self, token: Option<String>) {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = token;
    }
}

/// Progress for the UI, emitted as the `yandex-progress` event.
#[derive(Clone, Serialize)]
struct Progress<'a> {
    stage: &'a str,
    detail: Option<String>,
}

fn progress(app: &AppHandle, stage: &str, detail: Option<String>) {
    let _ = app.emit("yandex-progress", Progress { stage, detail });
}

/// Signs in if needed and returns the whole library. Tries the browser's session
/// first, then the login window; a stale token gets one fresh sign-in.
#[tauri::command]
pub async fn yandex_export(
    app: AppHandle,
    state: State<'_, YandexState>,
    app_state: State<'_, AppState>,
) -> CmdResult<Library> {
    let _busy = state
        .busy
        .try_lock()
        .map_err(|_| "импорт уже идёт".to_owned())?;
    let (mut tried_browser, mut tried_window) = (false, false);
    loop {
        let token = match state.token() {
            Some(t) => t,
            None => {
                let from_browser = if tried_browser {
                    None
                } else {
                    tried_browser = true;
                    browser_token(&app, &app_state).await
                };
                let t = match from_browser {
                    Some(t) => t,
                    None => {
                        tried_window = true;
                        progress(&app, "login", None);
                        sign_in(&app).await?
                    }
                };
                state.set_token(Some(t.clone()));
                t
            }
        };
        let client = Client::new(token).map_err(|e| e.to_string())?;
        match export(&client, &|stage, detail| progress(&app, stage, detail)).await {
            Err(ApiError::Unauthorized) if !tried_window => state.set_token(None),
            Err(ApiError::Unauthorized) => {
                state.set_token(None);
                return Err("Яндекс не принял вход, попробуйте ещё раз".to_owned());
            }
            Err(e) => return Err(e.to_string()),
            Ok(library) => return Ok(library),
        }
    }
}

/// A token from the browser's Yandex session, or `None`.
async fn browser_token(app: &AppHandle, app_state: &AppState) -> Option<String> {
    let streams = app_state.streams.as_ref()?;
    let dir = app.path().app_cache_dir().ok()?;
    progress(app, "browser", None);
    browser::browser_token(streams.backend(), &dir).await
}

/// Opens the login window and waits for the redirect with the token.
async fn sign_in(app: &AppHandle) -> CmdResult<String> {
    if let Some(old) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = old.destroy();
    }
    let (tx, rx) = oneshot::channel::<Option<String>>();
    let tx = std::sync::Arc::new(Mutex::new(Some(tx)));
    let send = {
        let tx = tx.clone();
        move |value: Option<String>| {
            if let Some(tx) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = tx.send(value);
            }
        }
    };

    let on_nav = send.clone();
    let window = WebviewWindowBuilder::new(
        app,
        LOGIN_WINDOW,
        WebviewUrl::External(authorize_url("oauth.yandex.ru").parse().expect("valid URL")),
    )
    .title("Вход в Яндекс")
    .inner_size(480.0, 720.0)
    // Some engines report the redirect with its fragment here; page loads and
    // the poll below cover the ones that do not.
    .on_navigation(move |url| match token_from_url(url) {
        Some(token) => {
            on_nav(Some(token));
            false
        }
        None => true,
    })
    .on_page_load({
        let send = send.clone();
        move |_, payload| {
            if let Some(token) = token_from_url(payload.url()) {
                send(Some(token));
            }
        }
    })
    .build()
    .map_err(|e| format!("не удалось открыть окно входа: {e}"))?;

    let on_close = send.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            on_close(None);
        }
    });

    let poll = {
        let window = window.clone();
        let send = send.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                match window.url() {
                    Ok(url) => {
                        if let Some(token) = token_from_url(&url) {
                            send(Some(token));
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        })
    };

    let token = rx.await.ok().flatten();
    poll.abort();
    let _ = window.destroy();
    token.ok_or_else(|| "вход в Яндекс отменён".to_owned())
}
