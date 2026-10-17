//! Live tests against YouTube; they need yt-dlp and network access.
//! Run with `cargo test -p volna-streams --test live -- --ignored --nocapture`.
#![cfg(feature = "ytdlp")]

use std::sync::Arc;
use std::time::Instant;

use volna_net::{NetManager, NetOptions};
use volna_streams::{ImageCache, Quality, StreamRequest, StreamService, VideoId, YtDlp};

/// "Never Gonna Give You Up": stable, available almost everywhere.
const VIDEO: &str = "dQw4w9WgXcQ";

fn net() -> Arc<NetManager> {
    Arc::new(NetManager::new(NetOptions::default()).unwrap())
}

#[tokio::test]
#[ignore = "network + yt-dlp"]
async fn resolves_real_track_and_fetches_64k_range() {
    let ytdlp = YtDlp::locate(None).expect("yt-dlp on PATH or $VOLNA_YTDLP");
    println!(
        "yt-dlp {} at {}",
        ytdlp.version().await.unwrap(),
        ytdlp.binary().display()
    );
    let svc = StreamService::new(ytdlp, net());
    let id = VideoId::parse(VIDEO).unwrap();

    let t = Instant::now();
    let stream = svc.resolve(&id, Quality::High).await.unwrap();
    println!(
        "resolved in {:?}: itag {} {} {:?} {:?} kbps, {:?} bytes, expires {:?}",
        t.elapsed(),
        stream.format_id,
        stream.mime,
        stream.codec,
        stream.bitrate_kbps,
        stream.filesize,
        stream.expires_at
    );
    assert!(stream.url.contains("googlevideo.com"));
    assert!(stream.mime == "audio/webm" || stream.mime == "audio/mp4");

    let t = Instant::now();
    let again = svc.resolve(&id, Quality::High).await.unwrap();
    assert_eq!(again.url, stream.url, "second resolve must come from cache");
    assert!(t.elapsed().as_millis() < 50);

    let mut req = StreamRequest {
        id: id.clone(),
        quality: Quality::High,
        range: Some("bytes=0-65535".into()),
        head: false,
    };
    let t = Instant::now();
    let res = svc.serve(&req).await.unwrap();
    println!("first 64 KiB in {:?}: {}", t.elapsed(), res.content_range);
    assert_eq!(res.status, 206);
    assert_eq!(res.body.len(), 65536);
    if res.content_type == "audio/webm" {
        assert_eq!(&res.body[..4], &[0x1A, 0x45, 0xDF, 0xA3], "EBML magic");
    }

    // Seek into the middle.
    let mid = res.total / 2;
    req.range = Some(format!("bytes={mid}-{}", mid + 65535));
    let res = svc.serve(&req).await.unwrap();
    assert_eq!(
        res.content_range,
        format!("bytes {mid}-{}/{}", mid + 65535, res.total)
    );
    assert_eq!(res.body.len(), 65536);
}

#[tokio::test]
#[ignore = "network"]
async fn proxies_and_caches_a_real_cover() {
    let dir = std::env::temp_dir().join(format!("volna-img-live-{}", std::process::id()));
    let cache = ImageCache::new(net(), &dir);
    let url = format!("https://i.ytimg.com/vi/{VIDEO}/hqdefault.jpg");
    let first = cache.get(&url, None).await.unwrap();
    assert_eq!(first.content_type, "image/jpeg");
    assert!(!first.from_cache);
    let second = cache.get(&url, None).await.unwrap();
    assert!(second.from_cache);
    assert_eq!(first.bytes, second.bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
#[ignore = "network + yt-dlp"]
async fn loopback_server_streams_a_real_track() {
    let svc = StreamService::new(YtDlp::locate(None).unwrap(), net());
    let server = volna_streams::LoopbackServer::bind().unwrap();
    let url = format!("{}/{VIDEO}", server.stream_base());
    tokio::spawn(server.serve(svc));
    let http = reqwest::Client::builder().no_proxy().build().unwrap();

    let res = http
        .get(&url)
        .header("Range", "bytes=0-65535")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    let cr = res.headers()["content-range"].to_str().unwrap().to_owned();
    let total: usize = cr.rsplit('/').next().unwrap().parse().unwrap();
    assert_eq!(res.bytes().await.unwrap().len(), 65536);

    // The whole track in one streamed response, as <audio> would read it.
    let t = Instant::now();
    let res = http.get(&url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.bytes().await.unwrap();
    println!("full track {} bytes in {:?}", body.len(), t.elapsed());
    assert_eq!(body.len(), total);
}
