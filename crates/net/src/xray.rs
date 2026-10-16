//! The xray child process: finding the binary, starting it with a config on
//! stdin, waiting for the SOCKS port, collecting its log, killing it.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{NetError, Result};

/// Target triple this crate was built for, as Tauri uses in sidecar names.
pub const TARGET_TRIPLE: &str = env!("VOLNA_NET_TARGET");

/// How many log lines a [`LogBuffer`] keeps.
pub const LOG_CAPACITY: usize = 500;

/// Lines shown in startup error messages.
const ERROR_TAIL: usize = 15;

/// Bounded, shareable log of xray's stdout and stderr.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::with_capacity(LOG_CAPACITY)
    }
}

impl LogBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity: capacity.max(1),
        }
    }

    pub fn push(&self, line: impl Into<String>) {
        let mut buf = self.lock();
        if buf.len() == self.capacity {
            buf.pop_front();
        }
        buf.push_back(line.into());
    }

    /// All buffered lines, oldest first.
    pub fn lines(&self) -> Vec<String> {
        self.lock().iter().cloned().collect()
    }

    /// The last `n` lines joined with newlines.
    pub fn tail(&self, n: usize) -> String {
        let buf = self.lock();
        let skip = buf.len().saturating_sub(n);
        buf.iter()
            .skip(skip)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<String>> {
        // A panic while holding the lock cannot leave a VecDeque half-updated.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Finds the xray binary: an explicit path, then `$VOLNA_XRAY`, then a
/// sidecar next to the running executable (`volna-xray` as Tauri installs
/// the bundled one, or `volna-xray-<triple>`/`xray-<triple>` as in a source
/// tree), then `xray` on `PATH`.
///
/// A plain `xray` next to the executable is not treated as ours: a deb puts
/// the app in /usr/bin, where that is the distro's xray.
pub fn find_xray(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return if p.is_file() {
            Ok(p.to_path_buf())
        } else {
            Err(NetError::XrayNotFound)
        };
    }
    if let Some(p) = std::env::var_os("VOLNA_XRAY").map(PathBuf::from)
        && p.is_file()
    {
        return Ok(p);
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    resolve_xray_in(exe_dir.as_deref(), std::env::var_os("PATH").as_deref())
        .ok_or(NetError::XrayNotFound)
}

fn resolve_xray_in(exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> Option<PathBuf> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let sidecars = [
        format!("volna-xray{suffix}"),
        format!("volna-xray-{TARGET_TRIPLE}{suffix}"),
        format!("xray-{TARGET_TRIPLE}{suffix}"),
    ];

    let next_to_exe = exe_dir
        .into_iter()
        .flat_map(|dir| sidecars.iter().map(move |name| dir.join(name)));
    let on_path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| dir.join(format!("xray{suffix}")));

    next_to_exe.chain(on_path).find(|p| p.is_file())
}

/// Asks the OS for a free TCP port on 127.0.0.1. Another process can grab it
/// before xray binds; callers retry on [`NetError::XrayExited`].
pub fn free_local_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(NetError::NoFreePort)?;
    let port = listener.local_addr().map_err(NetError::NoFreePort)?.port();
    Ok(port)
}

/// A running xray. Killed when dropped.
#[derive(Debug)]
pub struct XrayProcess {
    child: Mutex<Child>,
    port: u16,
}

impl XrayProcess {
    /// Starts `binary run -format json -c stdin:` with `config`, then waits
    /// until 127.0.0.1:`socks_port` accepts connections. The config never
    /// touches the disk.
    pub async fn spawn(
        binary: &Path,
        config: &Value,
        socks_port: u16,
        logs: &LogBuffer,
        timeout: Duration,
    ) -> Result<Self> {
        let mut cmd = Command::new(binary);
        cmd.args(["run", "-format", "json", "-c", "stdin:"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        #[cfg(target_os = "linux")]
        die_with_parent(&mut cmd);

        let mut child = cmd.spawn().map_err(|source| NetError::XraySpawn {
            path: binary.to_path_buf(),
            source,
        })?;
        tracing::info!(pid = child.id(), port = socks_port, "xray started");

        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump_lines(out, logs.clone()));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump_lines(err, logs.clone()));
        }
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = serde_json::to_vec(config).expect("a serde_json::Value always serializes");
            // If xray dies before reading, the error surfaces in the wait below.
            let _ = stdin.write_all(&bytes).await;
            let _ = stdin.shutdown().await;
        }

        let process = XrayProcess {
            child: Mutex::new(child),
            port: socks_port,
        };
        process.wait_ready(logs, timeout).await?;
        Ok(process)
    }

    async fn wait_ready(&self, logs: &LogBuffer, timeout: Duration) -> Result<()> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let started = Instant::now();
        loop {
            if let Some(status) = self.exit_status() {
                // Let the log pumps drain the final lines.
                tokio::time::sleep(Duration::from_millis(50)).await;
                return Err(NetError::XrayExited {
                    status,
                    log: logs.tail(ERROR_TAIL),
                });
            }
            if tokio::net::TcpStream::connect(addr).await.is_ok() {
                tracing::debug!(port = self.port, elapsed = ?started.elapsed(), "xray ready");
                return Ok(());
            }
            if started.elapsed() >= timeout {
                self.kill();
                return Err(NetError::XrayTimeout {
                    port: self.port,
                    timeout_ms: timeout.as_millis(),
                    log: logs.tail(ERROR_TAIL),
                });
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn pid(&self) -> Option<u32> {
        self.lock().id()
    }

    /// `true` while the process has not exited.
    pub fn is_running(&self) -> bool {
        self.exit_status().is_none()
    }

    fn exit_status(&self) -> Option<String> {
        match self.lock().try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(e) => Some(format!("unknown status: {e}")),
        }
    }

    fn kill(&self) {
        let _ = self.lock().start_kill();
    }

    /// Kills the process and waits for it to exit.
    pub async fn shutdown(mut self) {
        let child = self.child.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill().await;
        tracing::info!(port = self.port, "xray stopped");
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Child> {
        self.child.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for XrayProcess {
    fn drop(&mut self) {
        // kill_on_drop covers the child too; this keeps it explicit and logged.
        if let Ok(child) = self.child.get_mut()
            && matches!(child.try_wait(), Ok(None))
            && child.start_kill().is_ok()
        {
            tracing::info!(port = self.port, "xray killed");
        }
    }
}

async fn pump_lines(stream: impl AsyncRead + Unpin, logs: LogBuffer) {
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tracing::debug!(target: "volna_net::xray_log", "{line}");
        logs.push(line);
    }
}

/// kill_on_drop only helps when the app exits normally. If it is killed
/// (SIGKILL, a crash, `tauri dev` restarting it), xray would keep running
/// and hold its port, so ask the kernel to SIGKILL it when we die.
///
/// The signal fires when the *thread* that spawned the child exits; spawns
/// happen on tokio worker threads, which live as long as the runtime.
#[cfg(target_os = "linux")]
fn die_with_parent(cmd: &mut Command) {
    // SAFETY: the closure runs between fork and exec and only calls
    // async-signal-safe functions (prctl, getppid).
    let parent = std::process::id() as libc::pid_t;
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died before prctl took effect.
            if libc::getppid() != parent {
                return Err(std::io::Error::other("parent exited before xray started"));
            }
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_is_bounded() {
        let logs = LogBuffer::with_capacity(3);
        for i in 0..5 {
            logs.push(format!("line {i}"));
        }
        assert_eq!(logs.lines(), ["line 2", "line 3", "line 4"]);
        assert_eq!(logs.tail(2), "line 3\nline 4");
        assert_eq!(logs.tail(10), "line 2\nline 3\nline 4");
    }

    #[test]
    fn resolves_sidecar_before_path() {
        let root = scratch_dir("resolve");
        let exe_dir = root.join("app");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&bin_dir).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let on_path = bin_dir.join(format!("xray{suffix}"));
        std::fs::write(&on_path, b"").unwrap();
        let path_var = std::env::join_paths([&bin_dir]).unwrap();

        // Only on PATH.
        assert_eq!(
            resolve_xray_in(Some(&exe_dir), Some(&path_var)),
            Some(on_path.clone())
        );

        // A plain xray next to the app (/usr/bin/xray from the distro) is not ours.
        std::fs::write(exe_dir.join(format!("xray{suffix}")), b"").unwrap();
        assert_eq!(
            resolve_xray_in(Some(&exe_dir), Some(&path_var)),
            Some(on_path.clone())
        );

        // Source-tree sidecar with the target triple wins over PATH.
        let triple = exe_dir.join(format!("xray-{TARGET_TRIPLE}{suffix}"));
        std::fs::write(&triple, b"").unwrap();
        assert_eq!(
            resolve_xray_in(Some(&exe_dir), Some(&path_var)),
            Some(triple)
        );

        // The installed bundle sidecar wins over everything.
        let bundled = exe_dir.join(format!("volna-xray{suffix}"));
        std::fs::write(&bundled, b"").unwrap();
        assert_eq!(
            resolve_xray_in(Some(&exe_dir), Some(&path_var)),
            Some(bundled)
        );

        assert_eq!(resolve_xray_in(None, None), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn free_port_is_bindable() {
        let port = free_local_port().unwrap();
        assert_ne!(port, 0);
        TcpListener::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "volna-net-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
}
