//! `StreamService` and `LoopbackServer` against a local fake googlevideo server.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use volna_streams::{
    Backend, Network, Quality, ResolvedStream, Resolver, StreamError, StreamRequest, StreamService,
    VideoId,
};

const TOTAL: usize = 10_000;
const CAP: u64 = 4096;

fn byte_at(i: usize) -> u8 {
    (i % 251) as u8
}

/// Serves `TOTAL` deterministic bytes with Range support. URLs whose query
/// contains `n=0` get a 403, like an expired googlevideo link; with `mid=1`,
/// `n=1` also expires for ranges from byte 5000 (expiry mid-playback).
async fn fake_upstream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let first = req.lines().next().unwrap_or_default().to_owned();
                if first.contains("n=0") {
                    let _ = sock
                        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                let expired_mid = first.contains("mid=1") && first.contains("n=1&");
                let range = req.lines().find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                });
                let (start, end) = match range {
                    Some(r) => {
                        let (a, b) = r.trim().split_once('-').unwrap();
                        let a: usize = a.parse().unwrap();
                        let b: usize = if b.is_empty() {
                            TOTAL - 1
                        } else {
                            b.parse().unwrap()
                        };
                        (a, b.min(TOTAL - 1))
                    }
                    None => (0, TOTAL - 1),
                };
                if expired_mid && start >= 5000 {
                    let _ = sock
                        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }
                if start >= TOTAL {
                    let head = format!(
                        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{TOTAL}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    return;
                }
                let body: Vec<u8> = (start..=end).map(byte_at).collect();
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/webm\r\nContent-Range: bytes {start}-{end}/{TOTAL}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    format!("http://{addr}")
}

struct FakeBackend {
    base: String,
    calls: AtomicUsize,
    /// Whether `filesize` is reported (yt-dlp usually knows it).
    known_size: bool,
    /// Make the second URL expire half way through the file.
    expire_mid: bool,
}

impl Backend for FakeBackend {
    async fn resolve(
        &self,
        id: &VideoId,
        _quality: Quality,
        _proxy: Option<&str>,
    ) -> volna_streams::Result<ResolvedStream> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ResolvedStream {
            url: format!(
                "{}/videoplayback?id={id}&n={n}&mid={}",
                self.base, self.expire_mid as u8
            ),
            format_id: "251".into(),
            mime: "audio/webm".into(),
            codec: Some("opus".into()),
            bitrate_kbps: Some(128.0),
            filesize: self.known_size.then_some(TOTAL as u64),
            expires_at: SystemTime::now() + Duration::from_secs(6 * 3600),
            user_agent: Some("volna-test".into()),
        })
    }
}

struct LocalNet;

impl Network for LocalNet {
    fn client(&self) -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }
    fn proxy_url(&self) -> Option<String> {
        None
    }
}

async fn service(known_size: bool) -> StreamService<FakeBackend, LocalNet> {
    service_with(known_size, false).await
}

async fn service_with(known_size: bool, expire_mid: bool) -> StreamService<FakeBackend, LocalNet> {
    let backend = FakeBackend {
        base: fake_upstream().await,
        calls: AtomicUsize::new(0),
        known_size,
        expire_mid,
    };
    StreamService::with_resolver(Resolver::new(backend), Arc::new(LocalNet), CAP)
}

fn req(range: Option<&str>) -> StreamRequest {
    StreamRequest {
        id: VideoId::parse("dQw4w9WgXcQ").unwrap(),
        quality: Quality::High,
        range: range.map(str::to_owned),
        head: false,
    }
}

fn expected(start: usize, end: usize) -> Vec<u8> {
    (start..=end).map(byte_at).collect()
}

#[tokio::test]
async fn expired_url_is_re_resolved_once_then_ranges_work() {
    let svc = service(true).await;

    // First resolve hands out n=0 (403) → service re-resolves to n=1.
    let res = svc.serve(&req(Some("bytes=100-199"))).await.unwrap();
    assert_eq!(svc.backend().calls.load(Ordering::SeqCst), 2);
    assert_eq!(res.status, 206);
    assert_eq!(res.content_type, "audio/webm");
    assert_eq!(res.content_range, format!("bytes 100-199/{TOTAL}"));
    assert_eq!(res.content_length, 100);
    assert_eq!(&res.body[..], &expected(100, 199)[..]);

    // Open range is capped, and the cached URL is reused.
    let res = svc.serve(&req(Some("bytes=0-"))).await.unwrap();
    assert_eq!(res.content_range, format!("bytes 0-{}/{TOTAL}", CAP - 1));
    assert_eq!(res.body.len() as u64, CAP);
    assert_eq!(svc.backend().calls.load(Ordering::SeqCst), 2);

    // A seek near the end gets the tail only.
    let res = svc.serve(&req(Some("bytes=9000-"))).await.unwrap();
    assert_eq!(res.content_range, format!("bytes 9000-9999/{TOTAL}"));
    assert_eq!(&res.body[..], &expected(9000, 9999)[..]);

    // No Range header: still a capped 206 from the start.
    let res = svc.serve(&req(None)).await.unwrap();
    assert_eq!(res.status, 206);
    assert_eq!(res.body.len() as u64, CAP);

    let err = svc.serve(&req(Some("bytes=20000-"))).await.unwrap_err();
    assert!(matches!(err, StreamError::RangeNotSatisfiable { total } if total == TOTAL as u64));
    assert_eq!(err.http_status(), 416);
}

#[tokio::test]
async fn unknown_size_is_learned_from_upstream() {
    let svc = service(false).await;
    let res = svc.serve(&req(Some("bytes=-10"))).await.unwrap();
    assert_eq!(res.content_range, format!("bytes 9990-9999/{TOTAL}"));
    assert_eq!(&res.body[..], &expected(9990, 9999)[..]);

    let res = svc.serve(&req(Some("bytes=5000-"))).await.unwrap();
    assert_eq!(res.total, TOTAL as u64);
    assert_eq!(res.body.len() as u64, CAP);

    // Past the end: upstream's 416 is passed on with its total.
    let err = svc.serve(&req(Some("bytes=10000-"))).await.unwrap_err();
    assert!(matches!(err, StreamError::RangeNotSatisfiable { total } if total == TOTAL as u64));
}

#[tokio::test]
async fn head_with_known_size_skips_upstream_body() {
    let svc = service(true).await;
    let mut r = req(Some("bytes=0-"));
    r.head = true;
    let res = svc.serve(&r).await.unwrap();
    assert!(res.body.is_empty());
    assert_eq!(res.content_length, CAP);
    assert_eq!(res.total, TOTAL as u64);
}

async fn collect(open: volna_streams::OpenStream) -> Vec<u8> {
    use futures_util::StreamExt;
    let mut body = open.body.expect("GET has a body");
    let mut out = Vec::new();
    while let Some(chunk) = body.next().await {
        out.extend_from_slice(&chunk.unwrap());
    }
    out
}

#[tokio::test]
async fn open_streams_whole_file_over_several_windows() {
    // 1000-byte windows: the 10 000-byte file takes ten upstream requests.
    let svc = service_with(true, false).await.with_upstream_window(1000);
    let open = svc.open(&req(None)).await.unwrap();
    assert_eq!(open.status, 200);
    assert_eq!(open.content_range, None);
    assert_eq!(open.content_length, TOTAL as u64);
    assert_eq!(collect(open).await, expected(0, TOTAL - 1));

    let open = svc.open(&req(Some("bytes=2500-"))).await.unwrap();
    assert_eq!(open.status, 206);
    assert_eq!(open.content_range.as_deref(), Some("bytes 2500-9999/10000"));
    assert_eq!(collect(open).await, expected(2500, TOTAL - 1));
}

#[tokio::test]
async fn open_re_resolves_when_url_expires_mid_stream() {
    let svc = service_with(true, true).await.with_upstream_window(1000);
    let open = svc.open(&req(Some("bytes=0-"))).await.unwrap();
    assert_eq!(collect(open).await, expected(0, TOTAL - 1));
    // n=0 (expired at start), n=1 (expired at 5000), n=2.
    assert_eq!(svc.backend().calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn loopback_server_speaks_http_ranges() {
    let svc = service(true).await;
    let server = volna_streams::LoopbackServer::bind().unwrap();
    let base = server.stream_base();
    let origin = format!("http://{}", server.addr());
    tokio::spawn(server.serve(svc));
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("{base}/dQw4w9WgXcQ");

    let res = http
        .get(&url)
        .header("Range", "bytes=100-")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    let h = res.headers().clone();
    assert_eq!(h["content-type"], "audio/webm");
    assert_eq!(h["content-range"], "bytes 100-9999/10000");
    assert_eq!(h["accept-ranges"], "bytes");
    assert_eq!(h["access-control-allow-origin"], "*");
    assert_eq!(res.bytes().await.unwrap(), expected(100, TOTAL - 1));

    let res = http.get(&url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.bytes().await.unwrap().len(), TOTAL);

    let res = http
        .head(&url)
        .header("Range", "bytes=0-")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(res.headers()["content-length"], "10000");

    let res = http
        .get(&url)
        .header("Range", "bytes=10000-")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 416);
    assert_eq!(res.headers()["content-range"], "bytes */10000");

    let res = http.get(format!("{base}/bad")).send().await.unwrap();
    assert_eq!(res.status(), 400);

    // Wrong token: nothing to see.
    let wrong = format!("{origin}/0000/stream/dQw4w9WgXcQ");
    assert_eq!(http.get(wrong).send().await.unwrap().status(), 404);
}
