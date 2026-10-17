//! [`JsRuntime`] backed by a long-lived `deno` process (feature
//! `innertube-deno`): for desktop development and the live tests. Mobile
//! uses the platform WebView instead.
//!
//! The process runs `js/deno_host.js` with [`PRELUDE`] and talks JSON lines
//! over stdin/stdout. BotGuard needs a DOM, which the host takes from jsdom
//! (`npm:jsdom`, downloaded by Deno into its cache on first use). The
//! process gets no network or file permissions: Rust does every request.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};

use super::js::{JsError, JsFuture, JsRuntime, PRELUDE};

const HOST: &str = include_str!("../../js/deno_host.js");
/// First start downloads jsdom; later calls are quick except the first
/// `solve` of a player (~0.5 s) and a BotGuard snapshot (~0.3 s).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

type Pending = Arc<std::sync::Mutex<HashMap<u64, oneshot::Sender<String>>>>;

struct Process {
    child: Child,
    stdin: ChildStdin,
    pending: Pending,
}

/// Runs the resolver's JavaScript in Deno.
pub struct DenoRuntime {
    deno: PathBuf,
    deno_dir: Option<PathBuf>,
    script_dir: PathBuf,
    timeout: Duration,
    process: Mutex<Option<Process>>,
    next_id: AtomicU64,
}

impl DenoRuntime {
    /// Finds Deno: `$VOLNA_DENO`, the `volna-deno` sidecar next to the
    /// executable (what the Windows package ships), then `deno` on `PATH`.
    pub fn locate() -> Option<PathBuf> {
        let suffix = std::env::consts::EXE_SUFFIX;
        if let Some(p) = std::env::var_os("VOLNA_DENO").map(PathBuf::from)
            && p.is_file()
        {
            return Some(p);
        }
        let sidecar = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|d| d.join(format!("volna-deno{suffix}"))))
            .filter(|p| p.is_file());
        sidecar.or_else(|| {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|d| d.join(format!("deno{suffix}")))
                    .find(|p| p.is_file())
            })
        })
    }

    /// `deno_dir` overrides `DENO_DIR` (where jsdom is cached); `None` keeps
    /// Deno's default. The host script is written to `script_dir`.
    pub fn new(
        deno: impl Into<PathBuf>,
        deno_dir: Option<PathBuf>,
        script_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            deno: deno.into(),
            deno_dir,
            script_dir: script_dir.into(),
            timeout: DEFAULT_TIMEOUT,
            process: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Stops the process; the next call starts a fresh one (and the
    /// resolver resends the player and attests again).
    pub async fn restart(&self) {
        if let Some(mut p) = self.process.lock().await.take() {
            let _ = p.child.kill().await;
        }
    }

    fn write_host(&self) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.script_dir)?;
        let path = self.script_dir.join("volna-deno-host.js");
        let prelude = serde_json::to_string(PRELUDE).expect("string serializes");
        let source = format!("globalThis.__VOLNA_PRELUDE__ = {prelude};\n{HOST}");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(source.as_str()) {
            std::fs::write(&path, source)?;
        }
        Ok(path)
    }

    async fn spawn(&self) -> Result<Process, JsError> {
        let host = self
            .write_host()
            .map_err(|e| JsError::runtime(format!("cannot write the Deno host script: {e}")))?;
        let mut cmd = Command::new(&self.deno);
        cmd.args(["run", "--quiet", "--no-prompt", "--no-config", "--no-lock"])
            .arg("--node-modules-dir=none")
            // jsdom reads a few environment variables; nothing else is allowed.
            .arg("--allow-env")
            .arg(path_arg(&host))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .kill_on_drop(true);
        if let Some(dir) = &self.deno_dir {
            cmd.env("DENO_DIR", dir);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| JsError::runtime(format!("cannot start {}: {e}", self.deno.display())))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let stderr = child.stderr.take().expect("piped");
        let pending: Pending = Arc::default();

        let reader_pending = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                #[derive(serde::Deserialize)]
                struct Reply {
                    id: u64,
                    response: String,
                }
                match serde_json::from_str::<Reply>(&line) {
                    Ok(r) => {
                        let tx = lock(&reader_pending).remove(&r.id);
                        if let Some(tx) = tx {
                            let _ = tx.send(r.response);
                        }
                    }
                    Err(_) => tracing::debug!("deno: {line}"),
                }
            }
            // Process gone: dropping the senders fails every waiting call.
            lock(&reader_pending).clear();
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("deno stderr: {line}");
            }
        });
        Ok(Process {
            child,
            stdin,
            pending,
        })
    }

    async fn send(&self, request: String) -> Result<String, JsError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.process.lock().await;
            let alive = match guard.as_mut() {
                Some(p) => p.child.try_wait().ok().flatten().is_none(),
                None => false,
            };
            if !alive {
                *guard = Some(self.spawn().await?);
            }
            let p = guard.as_mut().expect("just spawned");
            lock(&p.pending).insert(id, tx);
            let line = format!("{{\"id\":{id},\"request\":{request}}}\n");
            if let Err(e) = p.stdin.write_all(line.as_bytes()).await {
                lock(&p.pending).remove(&id);
                *guard = None;
                return Err(JsError::runtime(format!("Deno stdin: {e}")));
            }
            let _ = p.stdin.flush().await;
        }
        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(JsError::runtime(
                "Deno exited (see the debug log for its stderr)",
            )),
            Err(_) => Err(JsError::runtime(format!(
                "Deno did not answer within {:?}",
                self.timeout
            ))),
        }
    }
}

impl JsRuntime for DenoRuntime {
    fn call(&self, request: String) -> JsFuture<'_> {
        Box::pin(self.send(request))
    }
}

fn path_arg(p: &Path) -> String {
    p.display().to_string()
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
