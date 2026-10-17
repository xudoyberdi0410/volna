//! Live tests for the RustyPipe backend (phase 5 spike), against real YouTube.
//!
//! ```sh
//! VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 \
//!   cargo test --manifest-path crates/streams-rustypipe/Cargo.toml \
//!   --test rustypipe_live -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `VOLNA_TEST_PROXY` is the exit every request (player API and googlevideo)
//! goes through; unset means direct. `VOLNA_TEST_COOKIES_TXT` (a Netscape
//! cookies.txt) adds a logged-in run of the client matrix (RustyPipe only
//! uses cookies for the web clients). `VOLNA_TEST_CLIENTS` and
//! `VOLNA_TEST_PAUSE_MS` narrow and pace the matrix.

use std::time::{Duration, Instant};

use rustypipe::client::{ClientType, RustyPipe};
use rustypipe::model::{AudioCodec, AudioStream};

/// YTM tracks: official videos (OMV) and "Topic" art tracks (ATV), several
/// of them licensed per region.
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

const CLIENTS: &[ClientType] = &[
    ClientType::Desktop,
    ClientType::DesktopMusic,
    ClientType::Mobile,
    ClientType::Tv,
    ClientType::Android,
    ClientType::AndroidVr,
    ClientType::Ios,
];

/// `VOLNA_TEST_CLIENTS=AndroidVr,Ios` limits the matrix to these clients.
fn client_selected(client: ClientType) -> bool {
    match std::env::var("VOLNA_TEST_CLIENTS") {
        Ok(list) => list
            .split(',')
            .any(|c| c.trim().eq_ignore_ascii_case(&format!("{client:?}"))),
        Err(_) => true,
    }
}

/// `VOLNA_TEST_PAUSE_MS` between player requests (default 1500).
fn pause() -> Duration {
    let ms = std::env::var("VOLNA_TEST_PAUSE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1500);
    Duration::from_millis(ms)
}

fn proxy() -> Option<String> {
    std::env::var("VOLNA_TEST_PROXY")
        .ok()
        .filter(|p| !p.is_empty())
}

fn http_builder() -> reqwest::ClientBuilder {
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(30));
    if let Some(p) = proxy() {
        b = b.proxy(reqwest::Proxy::all(p).unwrap());
    }
    b
}

fn rustypipe() -> RustyPipe {
    let dir = std::env::temp_dir().join("volna-rustypipe-live");
    std::fs::create_dir_all(&dir).unwrap();
    RustyPipe::builder()
        .storage_dir(dir)
        .no_botguard()
        .build_with_client(http_builder())
        .unwrap()
}

/// GET `bytes=0-65535` from googlevideo: status and bytes actually read.
async fn probe(http: &reqwest::Client, s: &AudioStream) -> (u16, usize) {
    let Some(url) = &s.url else {
        return (0, 0); // SABR-only: no plain URL
    };
    match http.get(url).header("Range", "bytes=0-65535").send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let n = r.bytes().await.map(|b| b.len()).unwrap_or(0);
            (status, n)
        }
        Err(e) => {
            println!("      request failed: {e}");
            (0, 0)
        }
    }
}

fn describe(s: &AudioStream) -> String {
    format!(
        "{}:{:?}@{}k{}",
        s.itag,
        s.codec,
        s.average_bitrate.max(s.bitrate) / 1000,
        if s.url.is_some() { "" } else { "(no url)" }
    )
}

async fn exit_ip(http: &reqwest::Client) -> String {
    match http.get("https://api.ipify.org").send().await {
        Ok(r) => r.text().await.unwrap_or_default(),
        Err(e) => format!("? ({e})"),
    }
}

/// The spike matrix: every client × every video, without PO token. A cell
/// is OK when googlevideo answers the range with 206 and real bytes.
#[tokio::test]
#[ignore = "network"]
async fn rustypipe_live_clients() {
    let http = http_builder().build().unwrap();
    println!("exit IP {} (proxy {:?})", exit_ip(&http).await, proxy());
    let rp = rustypipe();
    if let Ok(path) = std::env::var("VOLNA_TEST_COOKIES_TXT") {
        let txt = std::fs::read_to_string(path).unwrap();
        rp.user_auth_set_cookie_txt(&txt).await.unwrap();
        println!("logged in with cookies");
    }
    run_matrix(&rp, &http, false).await;
    if std::env::var("VOLNA_TEST_COOKIES_TXT").is_ok() {
        println!("\n=== same, authenticated ===");
        run_matrix(&rp, &http, true).await;
    }
}

async fn run_matrix(rp: &RustyPipe, http: &reqwest::Client, auth: bool) {
    let mut ok_per_client = vec![0usize; CLIENTS.len()];
    for (id, title) in VIDEOS {
        println!("\n{id} {title}");
        for (ci, client) in CLIENTS.iter().enumerate() {
            if !client_selected(*client) {
                continue;
            }
            // Bursts of player requests get an IP flagged faster.
            tokio::time::sleep(pause()).await;
            let q = if auth {
                rp.query().authenticated()
            } else {
                rp.query().unauthenticated()
            };
            let t = Instant::now();
            let player = match q.player_from_client(*id, *client).await {
                Ok(p) => p,
                Err(e) => {
                    let msg = e.to_string();
                    println!("  {client:?}: ERR {}", &msg[..msg.len().min(160)]);
                    continue;
                }
            };
            let ms = t.elapsed().as_millis();
            let formats: Vec<String> = player.audio_streams.iter().map(describe).collect();
            let best_opus = player
                .audio_streams
                .iter()
                .filter(|s| s.codec == AudioCodec::Opus)
                .max_by_key(|s| s.bitrate);
            let m4a = player.audio_streams.iter().find(|s| s.itag == 140);
            let mut cells = Vec::new();
            for s in [best_opus, m4a].into_iter().flatten() {
                let (status, n) = probe(http, s).await;
                cells.push(format!("{}→{status}/{n}B", s.itag));
            }
            let ok = cells.iter().any(|c| c.ends_with("206/65536B"));
            if ok {
                ok_per_client[ci] += 1;
            }
            println!(
                "  {client:?}: {} in {ms} ms, valid {}s, sabr {}, formats [{}], probe [{}]",
                if ok { "OK" } else { "NO" },
                player.expires_in_seconds,
                player.abr_streaming_url.is_some(),
                formats.join(" "),
                cells.join(" ")
            );
        }
    }
    println!("\nplayable per client (of {}):", VIDEOS.len());
    for (client, n) in CLIENTS.iter().zip(ok_per_client) {
        println!("  {client:?}: {n}");
    }
}

/// The app path: `NetManager` (with the test proxy) → `StreamService` with
/// [`RustyPipeBackend`] → `LoopbackServer`, read like GStreamer reads it.
/// Plays every video in Opus and in AAC (iOS) and seeks into the middle.
#[tokio::test]
#[ignore = "network"]
async fn rustypipe_live_plays_through_loopback() {
    use std::sync::Arc;
    use volna_net::{NetManager, NetOptions, ProxyConfig};
    use volna_streams::{Codec, LoopbackServer, Quality, StreamService};
    use volna_streams_rustypipe::{RustyPipeBackend, RustyPipeOptions};

    let net = Arc::new(NetManager::new(NetOptions::default()).unwrap());
    if let Some(p) = proxy() {
        net.apply(ProxyConfig::Url(p)).await.unwrap();
    }
    let backend = RustyPipeBackend::new(RustyPipeOptions {
        storage_dir: Some(std::env::temp_dir().join("volna-rustypipe-live")),
        ..Default::default()
    });
    let svc = StreamService::new(backend, net);
    let server = LoopbackServer::bind().unwrap();
    let base = server.stream_base();
    tokio::spawn(server.serve(svc.clone()));
    let http = reqwest_app::Client::builder().no_proxy().build().unwrap();

    let mut failed = Vec::new();
    for (id, title) in VIDEOS {
        for (codec, param, magic) in [
            (Codec::PreferOpus, "", &[0x1A, 0x45, 0xDF, 0xA3][..]), // EBML (WebM)
            (Codec::AacOnly, "?codec=aac", &b"ftyp"[..]),           // MP4 at offset 4
        ] {
            let t = Instant::now();
            let vid = volna_streams::VideoId::parse(id).unwrap();
            let resolved = match svc.resolve(&vid, Quality::High.with_codec(codec)).await {
                Ok(s) => s,
                Err(e) => {
                    println!("{id} {title} {codec:?}: resolve failed: {e}");
                    failed.push(format!("{id} {codec:?}"));
                    continue;
                }
            };
            let url = format!("{base}/{id}{param}");
            let res = http
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
            let head = res.bytes().await.unwrap_or_default();
            let magic_ok = match codec {
                Codec::PreferOpus if resolved.mime == "audio/webm" => head.starts_with(magic),
                _ => head.get(4..8) == Some(magic),
            };
            // Seek to the middle, as a scrub would.
            let mid = total / 2;
            let seek = http
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
                "{id} {title} {codec:?}: {} itag {} {} {:?} {:?} kbps, {total} B, {:?}",
                if ok { "OK" } else { "FAIL" },
                resolved.format_id,
                resolved.mime,
                resolved.codec,
                resolved.bitrate_kbps.map(f64::round),
                t.elapsed()
            );
            if !ok {
                failed.push(format!(
                    "{id} {codec:?} ({status}/{} {seek_status}/{seek_len})",
                    head.len()
                ));
            }
        }
    }
    assert!(failed.is_empty(), "not played: {failed:?}");
}
