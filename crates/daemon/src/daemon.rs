//! The `volna` daemon (`docs/native-player.md` §9): everything but the
//! window. It owns the player, the audio server, the network, the wave,
//! MPRIS and the tray, serves the window process over [`crate::ipc`], and
//! starts `volna-window` (the Flutter app) on demand. No GUI toolkit is
//! loaded here.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use volna_core::{Core, CoreConfig, StreamBackend};

pub use volna_core::{Dirs, IDENTIFIER};

use crate::ipc::{self, Frame, Msg, Role};
use crate::player::{self, forward::EventSink};

/// The connected window process, if any.
#[derive(Default)]
struct WindowLink {
    tx: Mutex<Option<(u64, UnboundedSender<Frame>)>>,
    next: AtomicU64,
}

impl WindowLink {
    fn attach(&self, tx: UnboundedSender<Frame>) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let old = self.tx.lock().expect("window link").replace((id, tx));
        if let Some((_, old)) = old {
            // A second window process: the newer one wins.
            let _ = old.send(Msg::Quit.into());
        }
        id
    }

    /// True when `id` was still the current window.
    fn detach(&self, id: u64) -> bool {
        let mut tx = self.tx.lock().expect("window link");
        if tx.as_ref().is_some_and(|(cur, _)| *cur == id) {
            *tx = None;
            true
        } else {
            false
        }
    }

    fn send(&self, f: Frame) -> bool {
        match self.tx.lock().expect("window link").as_ref() {
            Some((_, tx)) => tx.send(f).is_ok(),
            None => false,
        }
    }

    fn connected(&self) -> bool {
        self.tx.lock().expect("window link").is_some()
    }
}

/// The core plus the desktop host around it. Derefs to [`Core`]
/// (`d.app`, `d.player`, `d.library`, …).
pub struct Daemon {
    pub core: Arc<Core>,
    window: WindowLink,
    child: Mutex<Option<std::process::Child>>,
    quitting: AtomicBool,
}

impl std::ops::Deref for Daemon {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}

struct Sink(std::sync::Weak<Daemon>);

impl EventSink for Sink {
    fn connected(&self) -> bool {
        self.0.upgrade().is_some_and(|d| d.window.connected())
    }
    fn emit(&self, name: &str, payload: Value) {
        if let Some(d) = self.0.upgrade() {
            d.window.send(
                Msg::Event {
                    name: name.to_owned(),
                    payload,
                }
                .into(),
            );
        }
    }
}

/// The window binary: `VOLNA_WINDOW_BIN`, then (debug builds) the Flutter
/// bundle under `app/build/`, then `volna-window` next to this executable
/// (the Windows install dir), then `../lib/volna/volna-window` (the Arch
/// package: `/usr/bin/volna`, the Flutter bundle in `/usr/lib/volna/`).
pub fn window_exe() -> PathBuf {
    if let Some(p) = std::env::var_os("VOLNA_WINDOW_BIN") {
        return PathBuf::from(p);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(PathBuf::from));
    window_candidates(exe_dir.as_deref(), cfg!(debug_assertions))
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(WINDOW_BIN))
}

const WINDOW_BIN: &str = if cfg!(windows) {
    "volna-window.exe"
} else {
    "volna-window"
};

/// Where [`window_exe`] looks, in order.
fn window_candidates(exe_dir: Option<&std::path::Path>, dev: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if dev {
        // `flutter run` / `flutter build` output in this checkout; preferred
        // over a stale window next to target/debug/volna.
        let app = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../app/build");
        for mode in ["debug", "release"] {
            if cfg!(windows) {
                let mode = if mode == "debug" { "Debug" } else { "Release" };
                out.push(app.join(format!("windows/x64/runner/{mode}/{WINDOW_BIN}")));
            } else {
                out.push(app.join(format!("linux/x64/{mode}/bundle/{WINDOW_BIN}")));
            }
        }
    }
    if let Some(dir) = exe_dir {
        out.push(dir.join(WINDOW_BIN));
        out.push(dir.join("../lib/volna").join(WINDOW_BIN));
    }
    out
}

impl Daemon {
    pub fn has_window(&self) -> bool {
        self.window.connected()
    }

    /// One visualizer frame to the window; false when there is none.
    pub fn send_vis(&self, frame: Vec<u8>) -> bool {
        self.window.send(Frame {
            msg: Msg::Vis,
            body: frame,
        })
    }

    /// Focuses the window, or starts the window process.
    pub fn show_window(&self) {
        if self.quitting.load(Ordering::SeqCst) {
            return;
        }
        if self.window.send(Msg::Show.into()) {
            return;
        }
        let mut child = self.child.lock().expect("window child");
        if let Some(c) = child.as_mut()
            && matches!(c.try_wait(), Ok(None))
        {
            // Still starting up (not connected yet).
            return;
        }
        let exe = window_exe();
        match std::process::Command::new(&exe)
            .env("VOLNA_SOCKET", ipc::socket_path())
            .spawn()
        {
            Ok(c) => {
                tracing::info!(pid = c.id(), "window process started");
                *child = Some(c);
            }
            Err(e) => tracing::error!("cannot start {}: {e}", exe.display()),
        }
    }

    fn window_gone(self: &Arc<Self>) {
        tracing::info!("window process disconnected");
        if let Some(h) = self.player.handle_opt() {
            let h = h.clone();
            tokio::spawn(async move {
                let _ = h.visualizer_stop().await;
            });
        }
        let d = self.clone();
        tokio::spawn(async move {
            // Reap the exited process.
            tokio::time::sleep(Duration::from_secs(2)).await;
            if let Some(c) = d.child.lock().expect("window child").as_mut() {
                let _ = c.try_wait();
            }
        });
    }

    /// The real exit: close the window, save the player, stop xray, leave.
    pub fn quit(self: &Arc<Self>) {
        if self.quitting.swap(true, Ordering::SeqCst) {
            return;
        }
        let d = self.clone();
        tokio::spawn(async move {
            tracing::info!("quitting");
            d.window.send(Msg::Quit.into());
            d.core.shutdown().await;
            #[cfg(unix)]
            let _ = std::fs::remove_file(ipc::socket_path());
            // Give the window a moment to get the Quit frame.
            tokio::time::sleep(Duration::from_millis(200)).await;
            std::process::exit(0);
        });
    }
}

/// How `volna` was started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Show the window (the desktop entry).
    Show,
    /// Stay in the tray (`--tray`, autostart).
    Tray,
    /// Ask a running daemon to quit (`--quit`).
    Quit,
}

impl Launch {
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Self {
        let mut launch = Launch::Show;
        for a in args {
            match a.as_str() {
                "--tray" | "--hidden" => launch = Launch::Tray,
                "--quit" => launch = Launch::Quit,
                _ => {}
            }
        }
        launch
    }
}

/// Hands the launch to a running daemon. True when one answered.
async fn forward_to_running(launch: Launch) -> bool {
    let Ok(stream) = ipc::connect().await else {
        return false;
    };
    let on_closed = Box::new(|| {});
    let Ok((client, _)) =
        ipc::Client::start(stream, Role::Control, Box::new(|_| {}), on_closed).await
    else {
        return false;
    };
    match launch {
        Launch::Show => {
            client.send(Msg::Show.into());
        }
        Launch::Quit => {
            client.send(Msg::Quit.into());
        }
        Launch::Tray => {}
    }
    // Let the writer flush before the process exits.
    tokio::time::sleep(Duration::from_millis(100)).await;
    true
}

/// Runs the daemon; returns when another one is already running.
pub async fn run(launch: Launch) -> anyhow::Result<()> {
    if forward_to_running(launch).await {
        tracing::info!("Volna is already running; handed over");
        return Ok(());
    }
    if launch == Launch::Quit {
        return Ok(());
    }
    let listener = listen()?;

    let config = CoreConfig {
        dirs: Dirs::new()?,
        proxy: true,
        audio_server: true,
        backend: StreamBackend::locate_ytdlp(),
        codec: volna_streams::Codec::PreferOpus,
    };
    let core = Core::start(config, player::gstreamer_output()).await?;
    let daemon = Arc::new(Daemon {
        core,
        window: WindowLink::default(),
        child: Mutex::new(None),
        quitting: AtomicBool::new(false),
    });
    // Core events (player, library, Yandex) go to the window while one is connected.
    let sink: Arc<dyn EventSink> = Arc::new(Sink(Arc::downgrade(&daemon)));
    daemon.core.set_event_sink(Some(sink));
    let handle = daemon.player.handle_opt().cloned();
    player::tray::start(daemon.clone(), handle.clone());
    #[cfg(target_os = "linux")]
    if let Some(h) = handle {
        player::mpris::start(daemon.clone(), h, daemon.app.images.clone());
    }
    spawn_signal_handler(daemon.clone());
    if launch == Launch::Show {
        daemon.show_window();
    }
    accept_loop(daemon, listener).await;
    Ok(())
}

#[cfg(unix)]
fn spawn_signal_handler(d: Arc<Daemon>) {
    use tokio::signal::unix::{SignalKind, signal};
    tokio::spawn(async move {
        let (Ok(mut term), Ok(mut int)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::interrupt()),
        ) else {
            return;
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
        d.quit();
    });
}

#[cfg(not(unix))]
fn spawn_signal_handler(d: Arc<Daemon>) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            d.quit();
        }
    });
}

#[cfg(unix)]
type Listener = tokio::net::UnixListener;
#[cfg(windows)]
type Listener = (tokio::net::TcpListener, String);

#[cfg(unix)]
fn listen() -> anyhow::Result<Listener> {
    use std::os::unix::fs::PermissionsExt;
    let path = ipc::socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // Nobody answered on it: a stale socket from a crash.
    let _ = std::fs::remove_file(&path);
    let l = tokio::net::UnixListener::bind(&path)?;
    tracing::info!(path = %path.display(), "daemon listening");
    Ok(l)
}

#[cfg(windows)]
fn listen() -> anyhow::Result<Listener> {
    let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    std_listener.set_nonblocking(true)?;
    let port = std_listener.local_addr()?.port();
    let token: String = (0..32)
        .map(|_| char::from(b'a' + fastrand_byte() % 26))
        .collect();
    let path = ipc::socket_path().with_extension("port");
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, format!("{port} {token}"))?;
    Ok((tokio::net::TcpListener::from_std(std_listener)?, token))
}

#[cfg(windows)]
fn fastrand_byte() -> u8 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(std::process::id() as u64);
    h.finish() as u8
}

async fn accept_loop(d: Arc<Daemon>, listener: Listener) {
    loop {
        #[cfg(unix)]
        let accepted = listener.accept().await.map(|(s, _)| s);
        #[cfg(windows)]
        let accepted = listener.0.accept().await.map(|(s, _)| s);
        match accepted {
            Ok(stream) => {
                let d = d.clone();
                #[cfg(windows)]
                let token = Some(listener.1.clone());
                #[cfg(not(windows))]
                let token: Option<String> = None;
                tokio::spawn(async move { serve(d, stream, token).await });
            }
            Err(e) => {
                tracing::warn!("accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

async fn serve(d: Arc<Daemon>, stream: ipc::Stream, token: Option<String>) {
    let (mut rd, tx) = ipc::split(stream);
    let role = match ipc::read_frame(&mut rd).await {
        Ok(Frame {
            msg: Msg::Hello { role, token: t },
            ..
        }) if token.is_none() || token == t => role,
        _ => return,
    };
    let _ = tx.send(
        Msg::Welcome {
            stream_base: d.app.stream_base.clone(),
        }
        .into(),
    );
    let link = (role == Role::Window).then(|| d.window.attach(tx.clone()));
    if link.is_some() {
        tracing::info!("window process connected");
    }
    loop {
        let f = match ipc::read_frame(&mut rd).await {
            Ok(f) => f,
            Err(_) => break,
        };
        match f.msg {
            Msg::Show => d.show_window(),
            Msg::Quit => d.quit(),
            Msg::Call { id, cmd, args } => {
                let (d, tx) = (d.clone(), tx.clone());
                tokio::spawn(async move {
                    let msg = match crate::dispatch::call(&d, &cmd, &args).await {
                        Ok(ok) => Msg::Reply { id, ok, err: None },
                        Err(e) => Msg::Reply {
                            id,
                            ok: Value::Null,
                            err: Some(e),
                        },
                    };
                    let _ = tx.send(msg.into());
                });
            }
            Msg::Proto {
                id,
                method,
                uri,
                headers,
            } => {
                let (d, tx) = (d.clone(), tx.clone());
                tokio::spawn(async move {
                    let frame = protocol_call(&d, id, method, uri, headers, f.body).await;
                    let _ = tx.send(frame);
                });
            }
            other => tracing::debug!("unexpected IPC message {other:?}"),
        }
    }
    if let Some(id) = link
        && d.window.detach(id)
    {
        d.window_gone();
    }
}

/// A `volna://` request from the window, answered by the same handler the
/// single-process app used.
async fn protocol_call(
    d: &Daemon,
    id: u64,
    method: String,
    uri: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
) -> Frame {
    let mut req = http::Request::builder()
        .method(method.as_str())
        .uri(uri.as_str());
    for (k, v) in &headers {
        req = req.header(k, v);
    }
    let resp = match req.body(body) {
        Ok(req) => crate::protocol::handle(&d.app, req).await,
        Err(e) => crate::protocol::json_error(http::StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let (parts, body) = resp.into_parts();
    let headers = parts
        .headers
        .iter()
        .filter_map(|(k, v)| Some((k.to_string(), v.to_str().ok()?.to_owned())))
        .collect();
    Frame {
        msg: Msg::ProtoReply {
            id,
            status: parts.status.as_u16(),
            headers,
        },
        body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_flags() {
        let l = |a: &[&str]| Launch::from_args(a.iter().map(|s| s.to_string()));
        assert_eq!(l(&[]), Launch::Show);
        assert_eq!(l(&["--tray"]), Launch::Tray);
        assert_eq!(l(&["--quit"]), Launch::Quit);
    }

    #[test]
    fn window_lookup_order() {
        let dir = std::path::Path::new("/usr/bin");
        let installed = window_candidates(Some(dir), false);
        assert_eq!(
            installed,
            [
                dir.join(WINDOW_BIN),
                dir.join("../lib/volna").join(WINDOW_BIN)
            ]
        );
        let dev = window_candidates(Some(dir), true);
        assert_eq!(dev.len(), 4);
        assert!(dev[0].ends_with(if cfg!(windows) {
            "app/build/windows/x64/runner/Debug/volna-window.exe"
        } else {
            "app/build/linux/x64/debug/bundle/volna-window"
        }));
        assert_eq!(dev[2..], installed[..]);
    }

    #[test]
    fn window_link_keeps_the_newest() {
        let link = WindowLink::default();
        let (a, mut ra) = tokio::sync::mpsc::unbounded_channel();
        let (b, _rb) = tokio::sync::mpsc::unbounded_channel();
        let ia = link.attach(a);
        let ib = link.attach(b);
        // The older window was told to quit.
        assert_eq!(ra.try_recv().unwrap().msg, Msg::Quit);
        assert!(!link.detach(ia));
        assert!(link.connected());
        assert!(link.detach(ib));
        assert!(!link.connected());
    }
}
