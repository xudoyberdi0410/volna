//! Daemon ↔ window IPC (`docs/native-player.md` §9).
//!
//! A Unix socket at `$XDG_RUNTIME_DIR/volna/daemon.sock` (`VOLNA_SOCKET`
//! overrides it; Windows uses loopback TCP, see [`connect`]). Each frame is
//! `u32 LE header length, u32 LE body length, JSON header, raw body`. The
//! body carries what should not go through JSON: `volna://` response bodies
//! and visualizer frames.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// Frames bigger than this are a protocol error (covers are ≤ 8 MB).
const MAX_PART: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The `volna-window` process: calls, `volna://` requests, events.
    Window,
    /// A second `volna` launch: one request, then it exits.
    Control,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Msg {
    Hello {
        role: Role,
        /// Windows only: the token from the port file.
        #[serde(default)]
        token: Option<String>,
    },
    Welcome {
        stream_base: Option<String>,
    },
    /// A Tauri command, forwarded as is (`args` with the JS camelCase keys).
    Call {
        id: u64,
        cmd: String,
        args: Value,
    },
    Reply {
        id: u64,
        #[serde(default)]
        ok: Value,
        #[serde(default)]
        err: Option<String>,
    },
    /// A `volna://` request; the body is the request body.
    Proto {
        id: u64,
        method: String,
        uri: String,
        headers: Vec<(String, String)>,
    },
    /// The body is the response body.
    ProtoReply {
        id: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    /// Daemon → window: re-emit to the page.
    Event {
        name: String,
        payload: Value,
    },
    /// Daemon → window: one visualizer frame in the body.
    Vis,
    /// Control → daemon: show the window. Daemon → window: focus it.
    Show,
    /// Daemon → window: exit now. Control → daemon: quit.
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub msg: Msg,
    pub body: Vec<u8>,
}

impl From<Msg> for Frame {
    fn from(msg: Msg) -> Self {
        Self {
            msg,
            body: Vec::new(),
        }
    }
}

pub async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<Frame> {
    let mut len = [0u8; 8];
    r.read_exact(&mut len).await?;
    let head = u32::from_le_bytes(len[..4].try_into().expect("4 bytes")) as usize;
    let body = u32::from_le_bytes(len[4..].try_into().expect("4 bytes")) as usize;
    if head > MAX_PART || body > MAX_PART {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame too large",
        ));
    }
    let mut h = vec![0u8; head];
    r.read_exact(&mut h).await?;
    let mut b = vec![0u8; body];
    r.read_exact(&mut b).await?;
    let msg =
        serde_json::from_slice(&h).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Frame { msg, body: b })
}

pub async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, f: &Frame) -> io::Result<()> {
    let head = serde_json::to_vec(&f.msg).map_err(io::Error::other)?;
    let mut buf = Vec::with_capacity(8 + head.len() + f.body.len());
    buf.extend_from_slice(&(head.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(f.body.len() as u32).to_le_bytes());
    buf.extend_from_slice(&head);
    buf.extend_from_slice(&f.body);
    w.write_all(&buf).await?;
    w.flush().await
}

/// Splits a stream into a frame reader and a queued writer task.
pub fn split<S>(
    stream: S,
) -> (
    impl AsyncRead + Unpin + Send + 'static,
    mpsc::UnboundedSender<Frame>,
)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (rd, mut wr) = tokio::io::split(stream);
    let (tx, mut rx) = mpsc::unbounded_channel::<Frame>();
    tokio::spawn(async move {
        while let Some(f) = rx.recv().await {
            if let Err(e) = write_frame(&mut wr, &f).await {
                tracing::debug!("IPC write failed: {e}");
                break;
            }
        }
    });
    (rd, tx)
}

// ---- endpoints ---------------------------------------------------------------

#[cfg(unix)]
pub type Stream = tokio::net::UnixStream;
#[cfg(windows)]
pub type Stream = tokio::net::TcpStream;

/// Where the daemon listens. `VOLNA_SOCKET` wins (test instances).
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("VOLNA_SOCKET") {
        return PathBuf::from(p);
    }
    let base = dirs::runtime_dir().unwrap_or_else(|| {
        // No XDG_RUNTIME_DIR (rare): a per-user directory in the temp dir.
        std::env::temp_dir().join(format!("volna-{}", user_tag()))
    });
    base.join("volna").join("daemon.sock")
}

fn user_tag() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".into())
}

#[cfg(unix)]
pub async fn connect() -> io::Result<Stream> {
    tokio::net::UnixStream::connect(socket_path()).await
}

/// Windows: the daemon writes `port token` next to the socket path; the
/// client sends the token in its `Hello`.
#[cfg(windows)]
pub async fn connect() -> io::Result<Stream> {
    let (port, _) = read_port_file()?;
    tokio::net::TcpStream::connect(("127.0.0.1", port)).await
}

#[cfg(windows)]
pub fn read_port_file() -> io::Result<(u16, String)> {
    let text = std::fs::read_to_string(socket_path().with_extension("port"))?;
    let mut parts = text.split_whitespace();
    let port = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad port file"))?;
    Ok((port, parts.next().unwrap_or_default().to_owned()))
}

/// The token a client puts in its `Hello` (Windows only).
pub fn client_token() -> Option<String> {
    #[cfg(windows)]
    {
        read_port_file().ok().map(|(_, t)| t)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

// ---- client ---------------------------------------------------------------------

/// What a connected client receives besides replies.
pub type OnPush = Box<dyn Fn(Frame) + Send + Sync>;

/// The window's (or a control launch's) connection to the daemon.
pub struct Client {
    tx: mpsc::UnboundedSender<Frame>,
    next: AtomicU64,
    pending: std::sync::Arc<Mutex<HashMap<u64, oneshot::Sender<Frame>>>>,
}

impl Client {
    /// Says hello, waits for `Welcome`, then routes replies to callers and
    /// everything else to `on_push`. `on_closed` runs when the daemon goes away.
    pub async fn start(
        stream: Stream,
        role: Role,
        on_push: OnPush,
        on_closed: Box<dyn FnOnce() + Send>,
    ) -> io::Result<(Self, Option<String>)> {
        let (mut rd, tx) = split(stream);
        tx.send(
            Msg::Hello {
                role,
                token: client_token(),
            }
            .into(),
        )
        .map_err(|_| io::Error::other("IPC writer gone"))?;
        let stream_base = match read_frame(&mut rd).await?.msg {
            Msg::Welcome { stream_base } => stream_base,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("expected Welcome, got {other:?}"),
                ));
            }
        };
        let pending: std::sync::Arc<Mutex<HashMap<u64, oneshot::Sender<Frame>>>> =
            Default::default();
        let p = pending.clone();
        tokio::spawn(async move {
            loop {
                let f = match read_frame(&mut rd).await {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::info!("daemon connection closed: {e}");
                        break;
                    }
                };
                let id = match &f.msg {
                    Msg::Reply { id, .. } | Msg::ProtoReply { id, .. } => Some(*id),
                    _ => None,
                };
                match id {
                    Some(id) => {
                        if let Some(tx) = p.lock().expect("pending").remove(&id) {
                            let _ = tx.send(f);
                        }
                    }
                    None => on_push(f),
                }
            }
            p.lock().expect("pending").clear();
            on_closed();
        });
        Ok((
            Self {
                tx,
                next: AtomicU64::new(1),
                pending,
            },
            stream_base,
        ))
    }

    pub fn send(&self, f: Frame) -> bool {
        self.tx.send(f).is_ok()
    }

    async fn request(&self, make: impl FnOnce(u64) -> Frame) -> Result<Frame, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending").insert(id, tx);
        if !self.send(make(id)) {
            self.pending.lock().expect("pending").remove(&id);
            return Err("unavailable: the Volna daemon is not running".into());
        }
        rx.await
            .map_err(|_| "unavailable: the Volna daemon went away".to_owned())
    }

    /// A forwarded command: `Ok(json)` or the command's error string.
    pub async fn call(&self, cmd: &str, args: Value) -> Result<Value, String> {
        let f = self
            .request(|id| {
                Msg::Call {
                    id,
                    cmd: cmd.to_owned(),
                    args,
                }
                .into()
            })
            .await?;
        match f.msg {
            Msg::Reply { err: Some(e), .. } => Err(e),
            Msg::Reply { ok, .. } => Ok(ok),
            other => Err(format!("internal: unexpected reply {other:?}")),
        }
    }

    /// A forwarded `volna://` request: (status, headers, body).
    pub async fn proto(
        &self,
        method: String,
        uri: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<(u16, Vec<(String, String)>, Vec<u8>), String> {
        let f = self
            .request(|id| Frame {
                msg: Msg::Proto {
                    id,
                    method,
                    uri,
                    headers,
                },
                body,
            })
            .await?;
        match f.msg {
            Msg::ProtoReply {
                status, headers, ..
            } => Ok((status, headers, f.body)),
            other => Err(format!("unexpected reply {other:?}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_round_trip() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        let f = Frame {
            msg: Msg::Call {
                id: 7,
                cmd: "player_seek".into(),
                args: serde_json::json!({"position": 12.5}),
            },
            body: vec![1, 2, 3],
        };
        write_frame(&mut a, &f).await.unwrap();
        write_frame(&mut a, &Msg::Vis.into()).await.unwrap();
        assert_eq!(read_frame(&mut b).await.unwrap(), f);
        assert_eq!(read_frame(&mut b).await.unwrap().msg, Msg::Vis);
        let reply: Msg =
            serde_json::from_str(r#"{"t":"reply","id":3,"err":"bad_args: x"}"#).unwrap();
        assert_eq!(
            reply,
            Msg::Reply {
                id: 3,
                ok: Value::Null,
                err: Some("bad_args: x".into())
            }
        );
    }

    /// Byte fixtures shared with the Flutter client (`app/test/frame_test.dart`):
    /// change both sides together.
    const FIXTURE_HELLO: &str = "2a000000000000007b2274223a2268656c6c6f222c22726f6c65223a2277696e646f77222c22746f6b656e223a6e756c6c7d";
    const FIXTURE_PROTO: &str = "62000000020000007b2274223a2270726f746f222c226964223a322c226d6574686f64223a22474554222c22757269223a22766f6c6e613a2f2f696d672f3f753d7826733d3634222c2268656164657273223a5b5b22616363657074222c22696d6167652f2a225d5d7ddead";
    const FIXTURE_PROTO_REPLY: &str = "50000000030000007b2274223a2270726f746f5f7265706c79222c226964223a322c22737461747573223a3230302c2268656164657273223a5b5b22636f6e74656e742d74797065222c22696d6167652f706e67225d5d7d010203";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn frames_match_the_dart_fixtures() {
        let cases = [
            (
                FIXTURE_HELLO,
                Frame::from(Msg::Hello {
                    role: Role::Window,
                    token: None,
                }),
            ),
            (
                FIXTURE_PROTO,
                Frame {
                    msg: Msg::Proto {
                        id: 2,
                        method: "GET".into(),
                        uri: "volna://img/?u=x&s=64".into(),
                        headers: vec![("accept".into(), "image/*".into())],
                    },
                    body: vec![0xde, 0xad],
                },
            ),
            (
                FIXTURE_PROTO_REPLY,
                Frame {
                    msg: Msg::ProtoReply {
                        id: 2,
                        status: 200,
                        headers: vec![("content-type".into(), "image/png".into())],
                    },
                    body: vec![1, 2, 3],
                },
            ),
        ];
        for (fixture, frame) in cases {
            let mut out = Vec::new();
            write_frame(&mut out, &frame).await.unwrap();
            assert_eq!(hex(&out), fixture);
            let bytes = unhex(fixture);
            assert_eq!(read_frame(&mut bytes.as_slice()).await.unwrap(), frame);
        }
    }

    #[tokio::test]
    async fn oversized_frames_are_refused() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&(u32::MAX).to_le_bytes()).await.unwrap();
        a.write_all(&0u32.to_le_bytes()).await.unwrap();
        assert!(read_frame(&mut b).await.is_err());
    }
}
