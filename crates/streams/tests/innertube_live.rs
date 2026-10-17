//! Live tests for the InnerTube backend with Deno as the JavaScript runtime.
//!
//! ```sh
//! VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome \
//!   cargo test -p volna-streams --features innertube-deno --test innertube_live \
//!   -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `VOLNA_TEST_PROXY`: the exit for every request (InnerTube, BotGuard and
//! googlevideo); unset means direct. Cookies come from
//! `VOLNA_TEST_COOKIES=<browser>` (exported with yt-dlp, as the app does) or
//! `VOLNA_TEST_COOKIES_TXT=<cookies.txt>`. `VOLNA_TEST_PAUSE_MS` paces the
//! player requests (default 1000). Needs `deno` (or `$VOLNA_DENO`); the
//! first run downloads jsdom into `target/tmp/deno`.
#![cfg(feature = "innertube-deno")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use volna_streams::innertube::{CookieJar, DenoRuntime, InnerTube, InnerTubeOptions};
use volna_streams::{Codec, Quality, VideoId};

/// The RustyPipe spike's set: official videos (OMV) and "Topic" art tracks
/// (ATV), several of them licensed per region.
const VIDEOS: &[(&str, &str)] = &[
    ("dQw4w9WgXcQ", "Rick Astley - Never Gonna Give You Up (OMV)"),
    ("fJ9rUzIMcZQ", "Queen - Bohemian Rhapsody (OMV)"),
    ("kJQP7kiw5Fk", "Luis Fonsi - Despacito (OMV, Vevo)"),
    ("hTWKbfoikeg", "Nirvana - Smells Like Teen Spirit (OMV)"),
    ("xtxjm7ciwmc", "Kino - Gruppa krovi (ATV)"),
    ("eN_YiZ4uBe8", "Zemfira - Iskala (ATV)"),
    ("m9SMT5ipbxk", "YOASOBI - Idol (ATV)"),
    ("4D7u5KF7SP8", "Daft Punk - Get Lucky (ATV)"),
    ("kiw_VxQ2oXo", "Rammstein - Du hast (ATV)"),
    ("3YgtjHZyCIQ", "Taylor Swift - Anti-Hero (ATV)"),
];

fn proxy() -> Option<String> {
    std::env::var("VOLNA_TEST_PROXY")
        .ok()
        .filter(|p| !p.is_empty())
}

fn pause() -> Duration {
    let ms = std::env::var("VOLNA_TEST_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    Duration::from_millis(ms)
}

fn tmp() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn runtime() -> Arc<DenoRuntime> {
    let deno = DenoRuntime::locate().expect("deno on PATH or $VOLNA_DENO");
    Arc::new(DenoRuntime::new(
        deno,
        Some(tmp().join("deno")),
        tmp().join("deno-host"),
    ))
}

fn http() -> reqwest::Client {
    let b = reqwest::Client::builder().timeout(Duration::from_secs(30));
    match proxy() {
        Some(p) => b.proxy(reqwest::Proxy::all(p).unwrap()),
        None => b.no_proxy(),
    }
    .build()
    .unwrap()
}

/// The test's browser session, read the way the app reads it (yt-dlp export).
async fn cookies() -> Option<CookieJar> {
    if let Ok(path) = std::env::var("VOLNA_TEST_COOKIES_TXT") {
        return Some(CookieJar::from_netscape(
            &std::fs::read_to_string(path).unwrap(),
        ));
    }
    let browser = std::env::var("VOLNA_TEST_COOKIES")
        .ok()
        .filter(|b| !b.is_empty())?;
    export_cookies(&browser).await
}

#[cfg(feature = "ytdlp")]
async fn export_cookies(browser: &str) -> Option<CookieJar> {
    use volna_streams::{Cookies, YtDlp};
    let ytdlp = YtDlp::locate(None).expect("yt-dlp exports the browser cookies");
    ytdlp.set_cookies(Cookies::Browser(browser.into())).unwrap();
    let file = tmp().join(format!("cookies-{}.txt", std::process::id()));
    ytdlp.dump_cookies(&file).await.unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let _ = std::fs::remove_file(&file);
    let jar = CookieJar::from_netscape(&text);
    println!(
        "cookies: {} from {browser}, logged in: {}",
        jar.len(),
        jar.is_logged_in()
    );
    Some(jar)
}

#[cfg(not(feature = "ytdlp"))]
async fn export_cookies(_browser: &str) -> Option<CookieJar> {
    panic!("VOLNA_TEST_COOKIES needs the ytdlp feature; use VOLNA_TEST_COOKIES_TXT")
}

async fn exit_ip(http: &reqwest::Client) -> String {
    match http.get("https://api.ipify.org").send().await {
        Ok(r) => r.text().await.unwrap_or_default(),
        Err(e) => format!("? ({e})"),
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "volna_streams=debug".into()),
        )
        .with_test_writer()
        .try_init();
}

/// GET 64 KiB at `offset`: status and the number of bytes read.
async fn probe(http: &reqwest::Client, url: &str, ua: Option<&str>, offset: u64) -> (u16, usize) {
    let range = format!("bytes={offset}-{}", offset + 65535);
    let mut req = http.get(url).header("Range", range);
    if let Some(ua) = ua {
        req = req.header("User-Agent", ua);
    }
    match req.send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            (status, r.bytes().await.map(|b| b.len()).unwrap_or(0))
        }
        Err(e) => {
            println!("    request failed: {e}");
            (0, 0)
        }
    }
}

struct Pass {
    name: &'static str,
    cookies: Option<CookieJar>,
    po_token: bool,
}

/// Resolves every track in Opus and AAC and reads its first 64 KiB from
/// googlevideo on the same exit. Prints a row per track and pass.
async fn run_pass(rt: Arc<DenoRuntime>, pass: Pass) -> (usize, usize) {
    let backend = InnerTube::new(
        rt,
        InnerTubeOptions {
            cache_dir: Some(tmp().join("players")),
            po_token: pass.po_token,
            ..Default::default()
        },
    );
    backend.set_cookies(pass.cookies);
    let http = http();
    let proxy = proxy();
    println!("\n== {} ==", pass.name);
    let (mut ok, mut total) = (0, 0);
    for (id, title) in VIDEOS {
        let vid = VideoId::parse(id).unwrap();
        let mut cells = Vec::new();
        for codec in [Codec::PreferOpus, Codec::AacOnly] {
            total += 1;
            let t = Instant::now();
            match backend
                .resolve(&vid, Quality::High.with_codec(codec), proxy.as_deref())
                .await
            {
                Ok(s) => {
                    let resolve_ms = t.elapsed().as_millis();
                    let ua = s.user_agent.as_deref();
                    let (status, n) = probe(&http, &s.url, ua, 0).await;
                    // Past the first MiBs, which googlevideo serves even
                    // without a PO token.
                    let mid = s.filesize.unwrap_or(0) / 2;
                    let (mid_status, mid_n) = probe(&http, &s.url, ua, mid).await;
                    let good = status == 206 && n == 65536 && mid_status == 206 && mid_n == 65536;
                    ok += usize::from(good);
                    let ip = url::Url::parse(&s.url)
                        .ok()
                        .and_then(|u| {
                            u.query_pairs()
                                .find(|(k, _)| k == "ip")
                                .map(|(_, v)| v.into_owned())
                        })
                        .unwrap_or_default();
                    cells.push(format!(
                        "{} {}@{:.0}k start {status}/{n}B mid({mid}) {mid_status}/{mid_n}B resolve {resolve_ms}ms ip={ip}",
                        if good { "OK" } else { "FAIL" },
                        s.format_id,
                        s.bitrate_kbps.unwrap_or(0.0),
                    ));
                }
                Err(e) => cells.push(format!("ERR {e}")),
            }
        }
        println!(
            "{id} {title}\n    opus: {}\n    aac:  {}",
            cells[0], cells[1]
        );
        tokio::time::sleep(pause()).await;
    }
    println!("{}: {ok}/{total} played", pass.name);
    (ok, total)
}

#[tokio::test]
#[ignore = "network + deno"]
async fn innertube_live() {
    init_tracing();
    let http = http();
    println!("exit IP: {}", exit_ip(&http).await);
    let rt = runtime();
    let jar = cookies().await;

    let main = run_pass(
        rt.clone(),
        Pass {
            name: if jar.is_some() {
                "cookies + PO token"
            } else {
                "no cookies + PO token"
            },
            cookies: jar.clone(),
            po_token: true,
        },
    )
    .await;
    // `VOLNA_TEST_MAIN_ONLY=1` skips the comparison passes.
    if jar.is_some() && std::env::var_os("VOLNA_TEST_MAIN_ONLY").is_none() {
        run_pass(
            rt.clone(),
            Pass {
                name: "no cookies + PO token",
                cookies: None,
                po_token: true,
            },
        )
        .await;
        run_pass(
            rt.clone(),
            Pass {
                name: "cookies, no PO token",
                cookies: jar,
                po_token: false,
            },
        )
        .await;
    }
    // Region-licensed tracks may be unavailable on the exit: allow two misses.
    assert!(main.0 + 4 >= main.1, "only {}/{} played", main.0, main.1);
}

/// The app path: `NetManager` (with the test proxy) → `StreamService` with
/// [`InnerTube`] → `LoopbackServer`, read like GStreamer reads it, in Opus
/// and in AAC (iOS), with a seek into the middle.
#[tokio::test]
#[ignore = "network + deno"]
async fn innertube_live_plays_through_loopback() {
    use volna_net::{NetManager, NetOptions, ProxyConfig};
    use volna_streams::{LoopbackServer, StreamService};

    init_tracing();
    let net = Arc::new(NetManager::new(NetOptions::default()).unwrap());
    if let Some(p) = proxy() {
        net.apply(ProxyConfig::Url(p)).await.unwrap();
    }
    let backend = InnerTube::new(
        runtime(),
        InnerTubeOptions {
            cache_dir: Some(tmp().join("players")),
            ..Default::default()
        },
    );
    backend.set_cookies(cookies().await);
    let svc = StreamService::new(backend, net);
    let server = LoopbackServer::bind().unwrap();
    let base = server.stream_base();
    tokio::spawn(server.serve(svc.clone()));
    let local = reqwest::Client::builder().no_proxy().build().unwrap();

    let mut failed = Vec::new();
    for (id, title) in VIDEOS {
        for (codec, param, magic) in [
            (Codec::PreferOpus, "", &[0x1A, 0x45, 0xDF, 0xA3][..]), // EBML (WebM)
            (Codec::AacOnly, "?codec=aac", &b"ftyp"[..]),           // MP4, at offset 4
        ] {
            let t = Instant::now();
            let url = format!("{base}/{id}{param}");
            let res = local
                .get(&url)
                .header("Range", "bytes=0-65535")
                .send()
                .await
                .unwrap();
            let status = res.status().as_u16();
            let total: u64 = res
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next()?.parse().ok())
                .unwrap_or(0);
            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let head = res.bytes().await.unwrap_or_default();
            let first_ms = t.elapsed().as_millis();
            let magic_ok = if content_type == "audio/webm" {
                head.starts_with(magic)
            } else {
                head.get(4..8) == Some(&b"ftyp"[..])
            };
            let mid = total / 2;
            let seek = local
                .get(&url)
                .header("Range", format!("bytes={mid}-{}", mid + 65535))
                .send()
                .await
                .unwrap();
            let seek_status = seek.status().as_u16();
            let seek_len = seek.bytes().await.map(|b| b.len()).unwrap_or(0);
            let ok = status == 206
                && head.len() == 65536
                && magic_ok
                && seek_status == 206
                && seek_len == 65536;
            println!(
                "{id} {title} {codec:?}: {} {content_type} {total} B, first 64 KiB in {first_ms} ms, seek {seek_status}",
                if ok { "OK" } else { "FAIL" },
            );
            if !ok {
                let why = svc
                    .last_failure(&VideoId::parse(id).unwrap())
                    .map(|f| f.message)
                    .unwrap_or_default();
                failed.push(format!(
                    "{id} {codec:?} ({status}/{} {seek_status}/{seek_len}) {why}",
                    head.len()
                ));
            }
        }
        tokio::time::sleep(pause()).await;
    }
    assert!(failed.len() <= 4, "not played: {failed:#?}");
}
