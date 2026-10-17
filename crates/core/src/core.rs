//! [`Core`]: the state behind every command, built once per process by the
//! host (the desktop daemon or the mobile FFI).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::Value;
use volna_player::{FrameSink, Output};
use volna_streams::LoopbackServer;

use crate::api::library::Library;
use crate::api::yandex::YandexState;
use crate::network::NetworkState;
use crate::player::{self, PlayerState, forward::EventSink};
use crate::settings::SettingsStore;
use crate::state::{AppState, StreamBackend};
use crate::wave::WaveState;

/// The app identifier: the directory name under the XDG dirs (as Tauri used).
pub const IDENTIFIER: &str = "uz.khudoberdi.volna";

/// Where settings, databases and caches live.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Dirs {
    /// Desktop: the same directories Tauri's `app_*_dir` gave the
    /// single-process app (`~/.config/uz.khudoberdi.volna` and so on).
    pub fn new() -> anyhow::Result<Self> {
        let pick = |d: Option<PathBuf>, what: &str| {
            d.map(|d| d.join(IDENTIFIER))
                .ok_or_else(|| anyhow::anyhow!("no {what} directory"))
        };
        Ok(Self {
            config: pick(dirs::config_dir(), "config")?,
            data: pick(dirs::data_dir(), "data")?,
            cache: pick(dirs::cache_dir(), "cache")?,
        })
    }

    /// Mobile: everything under the app's data directory unless the
    /// platform gives a separate cache directory.
    pub fn under(data: PathBuf, cache: Option<PathBuf>) -> Self {
        Self {
            config: data.join("config"),
            cache: cache.unwrap_or_else(|| data.join("cache")),
            data,
        }
    }
}

/// How the host wants the core set up.
pub struct CoreConfig {
    pub dirs: Dirs,
    /// Proxy URL / VLESS networking (desktop). False: direct only (mobile).
    pub proxy: bool,
    /// Serve audio on `http://127.0.0.1:<port>/` for the player.
    pub audio_server: bool,
    /// How audio URLs are resolved; `None` disables playback.
    pub backend: Option<StreamBackend>,
}

/// The event sink the host installs (the window, the FFI callback).
#[derive(Default)]
struct Events {
    sink: RwLock<Option<Arc<dyn EventSink>>>,
}

impl Events {
    fn current(&self) -> Option<Arc<dyn EventSink>> {
        self.sink.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl EventSink for Events {
    fn connected(&self) -> bool {
        self.current().is_some_and(|s| s.connected())
    }
    fn emit(&self, name: &str, payload: Value) {
        if let Some(s) = self.current() {
            s.emit(name, payload);
        }
    }
}

pub struct Core {
    pub dirs: Dirs,
    pub app: AppState,
    pub net: NetworkState,
    pub wave: WaveState,
    pub library: Arc<Library>,
    pub player: PlayerState,
    /// Last.fm, Libre.fm, ListenBrainz, Maloja; off until configured.
    pub scrobble: volna_scrobble::Scrobbling,
    pub yandex: Arc<YandexState>,
    events: Arc<Events>,
}

impl Core {
    /// Builds everything and starts the background jobs (audio server,
    /// player, scrobble retries, network restore). Call inside a tokio
    /// runtime. `output` is where the player sounds; `None` disables it.
    pub async fn start<O: Output>(
        config: CoreConfig,
        output: Option<O>,
    ) -> anyhow::Result<Arc<Self>> {
        let CoreConfig {
            dirs,
            proxy,
            audio_server,
            backend,
        } = config;
        let audio_server = audio_server
            .then(|| {
                LoopbackServer::bind()
                    .inspect_err(|e| tracing::error!("cannot bind the audio server: {e}"))
                    .ok()
            })
            .flatten();
        let stream_base = audio_server.as_ref().map(LoopbackServer::stream_base);
        let mut app = AppState::with_backend(&dirs.cache, backend)?;
        if let (Some(server), Some(streams)) = (audio_server, app.streams.clone()) {
            app.stream_base = stream_base;
            tokio::spawn(async move {
                if let Err(e) = server.serve(streams).await {
                    tracing::error!("audio server stopped: {e}");
                }
            });
        }
        let store = SettingsStore::new(&dirs.config);
        let net = if proxy {
            NetworkState::new(store)
        } else {
            NetworkState::direct_only(store)
        };
        let wave = WaveState::open(&dirs.data, app.ytm.clone())?;
        let library = Arc::new(Library::open(&dirs.data, Some(wave.engine()))?);
        let scrobble = {
            let net = app.net.clone();
            let http = volna_scrobble::ReqwestHttp::new(move || net.client());
            volna_scrobble::Scrobbling::open(&dirs.data.join("scrobble.sqlite3"), Arc::new(http))
                .map_err(anyhow::Error::msg)?
        };
        scrobble.spawn_retry(Duration::from_secs(120));
        library.set_scrobble(scrobble.clone());
        let player = player::setup(
            &dirs.data,
            &dirs.config,
            &app,
            &wave,
            &library,
            &scrobble,
            output,
        );
        #[cfg(feature = "ytdlp")]
        let yandex = crate::api::yandex::desktop_state(&app, &dirs.cache, library.clone());
        #[cfg(not(feature = "ytdlp"))]
        let yandex = Arc::new(YandexState::new(library.clone(), None));
        let events = Arc::new(Events::default());
        let core = Arc::new(Self {
            dirs,
            app,
            net,
            wave,
            library,
            player,
            scrobble,
            yandex,
            events: events.clone(),
        });
        {
            // Restoring may start xray; nothing waits for it.
            let c = core.clone();
            tokio::spawn(async move { c.net.restore(&c.app).await });
        }
        {
            let sink = events.clone();
            core.library.set_listener(move |name, payload| {
                if sink.connected() {
                    sink.emit(name, payload);
                }
            });
        }
        let sink: Arc<dyn EventSink> = events;
        player::forward::start(sink, core.player.handle_opt().cloned(), &core.player.journal);
        Ok(core)
    }

    /// Where events go from now on (`None`: nowhere).
    pub fn set_event_sink(&self, sink: Option<Arc<dyn EventSink>>) {
        *self.events.sink.write().unwrap_or_else(|e| e.into_inner()) = sink;
    }

    /// True when an event sink is installed and listening.
    pub fn has_listener(&self) -> bool {
        self.events.connected()
    }

    /// An event sender for background jobs (`api::*`); silent without a sink.
    pub fn emitter(&self) -> crate::api::yandex::Emit {
        let events = self.events.clone();
        Arc::new(move |name, payload| events.emit(name, payload))
    }

    /// Runs one command: the same names, argument keys (camelCase) and
    /// results as the daemon IPC. Visualizer frames are dropped; hosts that
    /// show them use [`Core::call_with_vis`].
    pub async fn call(self: &Arc<Self>, cmd: &str, args: &Value) -> Result<Value, String> {
        self.call_with_vis(cmd, args, || Box::new(|_| false)).await
    }

    /// [`Core::call`] with the sink `player_visualizer_start` feeds.
    pub async fn call_with_vis(
        self: &Arc<Self>,
        cmd: &str,
        args: &Value,
        vis: impl FnOnce() -> FrameSink,
    ) -> Result<Value, String> {
        crate::dispatch::call(self, cmd, args, vis).await
    }

    /// One `volna://` request (images, audio).
    pub async fn proto(&self, request: http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
        crate::protocol::handle(&self.app, request).await
    }

    /// Saves the player and stops background networking (xray).
    pub async fn shutdown(&self) {
        if let Some(h) = self.player.handle_opt()
            && tokio::time::timeout(Duration::from_secs(5), h.shutdown())
                .await
                .is_err()
        {
            tracing::warn!("player shutdown timed out");
        }
        self.app.net.shutdown().await;
    }
}
