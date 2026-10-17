//! Import from Yandex Music (`docs/flutter-ui-plan.md` 3f).
//!
//! - `client`: the token and the library through the music API (direct, no proxy);
//! - `browser`: desktop only, the token from the browser's Yandex session via yt-dlp;
//! - `matching`: a Yandex track → the YouTube Music one (port of `ui/js/yandex-import.js`);
//! - `import`: matching the whole library and saving it through [`LibrarySink`].
//!
//! Commands (camelCase args): `yandex_status`, `yandex_set_token{token}`,
//! `yandex_import_start{likes?,playlists?,albums?,artists?,browser?}`,
//! `yandex_import_cancel`. Events: `yandex_import_progress` ([`Progress`]) and
//! `yandex_import_done` ([`Done`]).

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod browser;
pub mod client;
pub mod import;
pub mod matching;

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::watch;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use browser::desktop_state;
use client::{ApiError, Client, Library};
pub use import::{LibrarySink, Miss, Options, Progress, Summary};
use matching::Catalog;

/// Sends a daemon event (`name`, payload) to the window.
pub type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;
/// Looks for a token on its own (the browser session); `None` when there is none.
pub type TokenSource =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

pub const EVENT_PROGRESS: &str = "yandex_import_progress";
pub const EVENT_DONE: &str = "yandex_import_done";

/// `yandex_status` / `yandex_set_token` reply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub signed_in: bool,
    pub login: Option<String>,
    pub importing: bool,
}

/// `yandex_import_done` payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub ok: bool,
    pub cancelled: bool,
    /// No usable token: open the login webview, `yandex_set_token`, start again.
    pub need_login: bool,
    pub error: Option<String>,
    pub summary: Summary,
    pub misses: Vec<Miss>,
}

/// The token (kept for the daemon's lifetime) and the running import.
pub struct YandexState {
    token: Mutex<Option<String>>,
    /// Cancels the running import; `None` when idle.
    job: Mutex<Option<watch::Sender<bool>>>,
    sink: Arc<dyn LibrarySink>,
    browser: Option<TokenSource>,
}

impl YandexState {
    pub fn new(sink: Arc<dyn LibrarySink>, browser: Option<TokenSource>) -> Self {
        Self {
            token: Mutex::new(None),
            job: Mutex::new(None),
            sink,
            browser,
        }
    }

    fn token(&self) -> Option<String> {
        self.token.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_token(&self, token: Option<String>) {
        *self.token.lock().unwrap_or_else(|e| e.into_inner()) = token;
    }

    fn importing(&self) -> bool {
        self.job.lock().unwrap_or_else(|e| e.into_inner()).is_some()
    }

    /// `yandex_status`: checks the held token with Yandex (a dead one is dropped).
    pub async fn status(&self) -> Result<Status, String> {
        let importing = self.importing();
        let Some(token) = self.token() else {
            return Ok(Status {
                importing,
                ..Status::default()
            });
        };
        let client = Client::new(token).map_err(|e| e.to_string())?;
        match client.account().await {
            Ok(a) => Ok(Status {
                signed_in: true,
                login: a.login,
                importing,
            }),
            Err(ApiError::Unauthorized) => {
                self.set_token(None);
                Ok(Status {
                    importing,
                    ..Status::default()
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// `yandex_set_token{token}`: a bare token or the redirect URL with
    /// `#access_token=`; checked with Yandex before it is kept.
    pub async fn set_token_input(&self, input: &str) -> Result<Status, String> {
        let token = client::token_from_input(input)
            .ok_or_else(|| "bad_args: в строке нет токена Яндекса".to_owned())?;
        let account = Client::new(token.clone())
            .map_err(|e| format!("internal: {e}"))?
            .account()
            .await
            .map_err(|e| format!("upstream: {e}"))?;
        self.set_token(Some(token));
        Ok(Status {
            signed_in: true,
            login: account.login,
            importing: self.importing(),
        })
    }

    /// `yandex_import_cancel`: true when an import was running.
    pub fn cancel(&self) -> bool {
        match self.job.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            Some(tx) => {
                let _ = tx.send(true);
                true
            }
            None => false,
        }
    }

    /// `yandex_import_start`: runs the import in the background; progress and
    /// the result come as events. Errors only when one is already running.
    pub fn start<C: Catalog + 'static>(
        self: &Arc<Self>,
        catalog: C,
        emit: Emit,
        options: Options,
    ) -> Result<(), String> {
        let mut rx = {
            let mut job = self.job.lock().unwrap_or_else(|e| e.into_inner());
            if job.is_some() {
                return Err("импорт уже идёт".to_owned());
            }
            let (tx, rx) = watch::channel(false);
            *job = Some(tx);
            rx
        };
        let state = self.clone();
        tokio::spawn(async move {
            let run = state.run(&catalog, &emit, options);
            let done = tokio::select! {
                done = run => done,
                _ = rx.wait_for(|c| *c) => Done {
                    cancelled: true,
                    ..Done::default()
                },
            };
            *state.job.lock().unwrap_or_else(|e| e.into_inner()) = None;
            tracing::info!(
                ok = done.ok,
                cancelled = done.cancelled,
                need_login = done.need_login,
                error = ?done.error,
                summary = ?done.summary,
                "yandex import finished"
            );
            emit(EVENT_DONE, serde_json::to_value(&done).unwrap_or_default());
        });
        Ok(())
    }

    /// The token, the library, matching, saving. A cancel drops this at an
    /// await; saving has none, so it is never cut in half.
    async fn run<C: Catalog>(&self, catalog: &C, emit: &Emit, options: Options) -> Done {
        let progress =
            |p: Progress| emit(EVENT_PROGRESS, serde_json::to_value(p).unwrap_or_default());
        let library = match self.library(emit, options).await {
            Ok(l) => l,
            Err(done) => return done,
        };
        let matched = import::match_library(catalog, &library, &progress).await;
        progress(Progress::stage("saving", None));
        match import::save(self.sink.as_ref(), &library, &matched) {
            Ok(summary) => Done {
                ok: true,
                summary,
                misses: matched.misses,
                ..Done::default()
            },
            Err(e) => Done {
                error: Some(format!("не удалось сохранить в библиотеку: {e}")),
                summary: Summary {
                    matched: matched.tracks.len(),
                    total: matched.total,
                    ..Summary::default()
                },
                misses: matched.misses,
                ..Done::default()
            },
        }
    }

    /// The library with the held token, else the browser's session; a
    /// rejected token gets one try with the browser.
    async fn library(&self, emit: &Emit, options: Options) -> Result<Library, Done> {
        let progress = |stage: &str, detail: Option<String>| {
            emit(
                EVENT_PROGRESS,
                serde_json::to_value(Progress::stage(stage, detail)).unwrap_or_default(),
            )
        };
        let need_login = || Done {
            need_login: true,
            error: Some("нужен вход в Яндекс".to_owned()),
            ..Done::default()
        };
        let mut tried_browser = false;
        loop {
            let token = match self.token() {
                Some(t) => t,
                None => {
                    let source = self.browser.as_ref().filter(|_| options.browser);
                    let Some(source) = source.filter(|_| !tried_browser) else {
                        return Err(need_login());
                    };
                    tried_browser = true;
                    progress("browser", None);
                    let Some(t) = source().await else {
                        return Err(need_login());
                    };
                    self.set_token(Some(t.clone()));
                    t
                }
            };
            let client = Client::new(token).map_err(|e| Done {
                error: Some(e.to_string()),
                ..Done::default()
            })?;
            match client::export_parts(&client, options.parts(), &progress).await {
                Ok(library) => return Ok(library),
                Err(ApiError::Unauthorized) => self.set_token(None),
                Err(e) => {
                    return Err(Done {
                        error: Some(e.to_string()),
                        ..Done::default()
                    });
                }
            }
        }
    }
}

/// The `yandex_*` commands; `None` for any other. `emit` is only called by
/// `yandex_import_start`. Errors: `bad_args`, `upstream` (Yandex refused or is
/// unreachable), `busy` (an import is running).
pub async fn call<C: Catalog + 'static>(
    state: &Arc<YandexState>,
    catalog: C,
    emit: impl FnOnce() -> Emit,
    cmd: &str,
    a: &Value,
) -> Option<Result<Value, String>> {
    let r = match cmd {
        "yandex_status" => state.status().await.map(to_value).map_err(upstream),
        "yandex_set_token" => match a.get("token").and_then(Value::as_str) {
            None => Err("bad_args: `token` is required".to_owned()),
            Some(input) => state.set_token_input(input).await.map(to_value),
        },
        "yandex_import_start" => {
            let options = match a {
                Value::Null => Ok(Options::default()),
                a => serde_json::from_value(a.clone()).map_err(|e| format!("bad_args: {e}")),
            };
            match options {
                Ok(options) => state
                    .start(catalog, emit(), options)
                    .map(|()| serde_json::json!({ "started": true }))
                    .map_err(|e| format!("busy: {e}")),
                Err(e) => Err(e),
            }
        }
        "yandex_import_cancel" => Ok(Value::Bool(state.cancel())),
        _ => return None,
    };
    Some(r)
}

fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or_default()
}

fn upstream(e: String) -> String {
    format!("upstream: {e}")
}

/// The sink until `api/library.rs` is merged: every write fails, so an
/// import reports the save error instead of losing matches silently.
pub struct UnwiredLibrary;

impl LibrarySink for UnwiredLibrary {
    fn like_track(&self, _: &volna_ytm::Track, _: i64) -> anyhow::Result<bool> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn like_album(&self, _: &volna_ytm::AlbumSummary) -> anyhow::Result<bool> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn like_artist(&self, _: &volna_ytm::ArtistSummary) -> anyhow::Result<bool> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn imported_playlist(&self, _: &str, _: &str) -> anyhow::Result<Option<String>> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn create_playlist(&self, _: &str, _: &str, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn add_to_playlist(&self, _: &str, _: &[volna_ytm::Track]) -> anyhow::Result<()> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn playlist_track_ids(&self, _: &str) -> anyhow::Result<Vec<String>> {
        anyhow::bail!("библиотека ещё не подключена")
    }
    fn reorder_playlist(&self, _: &str, _: &[String]) -> anyhow::Result<()> {
        anyhow::bail!("библиотека ещё не подключена")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matching::tests::FakeCatalog;

    #[tokio::test]
    async fn without_a_token_the_import_asks_for_login() {
        let state = Arc::new(YandexState::new(Arc::new(UnwiredLibrary), None));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let emit: Emit = Arc::new(move |name, payload| {
            let _ = tx.send((name.to_owned(), payload));
        });
        state
            .start(FakeCatalog::default(), emit.clone(), Options::default())
            .unwrap();
        let (name, payload) = rx.recv().await.unwrap();
        assert_eq!(name, EVENT_DONE);
        assert_eq!(payload["needLogin"], true);
        assert_eq!(payload["ok"], false);
        assert_eq!(payload["summary"]["likedTracks"], 0);
        assert!(!state.importing());
        assert!(!state.cancel());
        assert_eq!(state.status().await.unwrap(), Status::default());
    }

    #[tokio::test]
    async fn a_pasted_line_without_a_token_is_refused() {
        let state = YandexState::new(Arc::new(UnwiredLibrary), None);
        assert!(state.set_token_input("hello world").await.is_err());
        assert_eq!(state.token(), None);
    }
}
