//! Which lyrics providers work today, against the real services.
//!
//! `cargo test -p volna-daemon lyrics_live -- --ignored --nocapture --test-threads=1`
//!
//! Set `VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081` to go out like the app
//! (YouTube Music is geo-blocked in some countries).

use volna_lyrics::{Cache, Key, LyricsService, Provider, TrackQuery};
use volna_ytm::{SearchFilter, YtmClient};

use super::{Args, lyrics_get, lyrics_sources};

fn http() -> reqwest::Client {
    let mut b = reqwest::Client::builder();
    if let Ok(p) = std::env::var("VOLNA_TEST_PROXY") {
        b = b.proxy(reqwest::Proxy::all(p).expect("VOLNA_TEST_PROXY"));
    }
    b.build().unwrap()
}

/// Well-known tracks: (search, title, artist, album, seconds, ISRC).
const TRACKS: [(&str, &str, &str, &str, u64, &str); 3] = [
    (
        "The Weeknd Blinding Lights",
        "Blinding Lights",
        "The Weeknd",
        "After Hours",
        200,
        "USUG11904206",
    ),
    (
        "Queen Bohemian Rhapsody",
        "Bohemian Rhapsody",
        "Queen",
        "A Night At The Opera",
        355,
        "GBUM71029604",
    ),
    (
        "Ed Sheeran Shape of You",
        "Shape of You",
        "Ed Sheeran",
        "÷",
        234,
        "GBAHS1600463",
    ),
];

async fn video_id(ytm: &YtmClient, search: &str) -> Option<String> {
    let page = ytm.search(search, SearchFilter::Songs).await.ok()?;
    page.items
        .iter()
        .find_map(|i| i.as_track())
        .map(|t| t.video_id.clone())
}

fn describe(l: &volna_lyrics::Lyrics) -> String {
    format!("{} ({:?}, {} lines)", l.source, l.synced, l.lines.len())
}

#[tokio::test]
#[ignore = "network: lyrics providers and music.youtube.com"]
async fn lyrics_live_providers() {
    let http = http();
    let ytm = YtmClient::new(http.clone());
    let svc = LyricsService::new(Cache::in_memory().unwrap(), Some(ytm.clone()));
    let mut working: Vec<Provider> = Vec::new();

    for (search, title, artist, album, secs, isrc) in TRACKS {
        let vid = video_id(&ytm, search).await;
        println!("\n== {artist} — {title} (videoId {vid:?})");
        let q = TrackQuery {
            title: title.into(),
            artist: artist.into(),
            album: Some(album.into()),
            duration_ms: Some(secs * 1000),
            isrc: Some(isrc.into()),
            video_id: vid.clone(),
        };
        for p in Provider::ALL {
            let t0 = std::time::Instant::now();
            let r = tokio::time::timeout(p.timeout(), svc.fetcher().provider(&http, &q, p)).await;
            let ms = t0.elapsed().as_millis();
            let status = match r {
                Err(_) => "timeout".to_owned(),
                Ok(Err(e)) => format!("error: {e}"),
                Ok(Ok(found)) if found.is_empty() => "nothing".to_owned(),
                Ok(Ok(found)) => {
                    if !working.contains(&p) {
                        working.push(p);
                    }
                    found.iter().map(describe).collect::<Vec<_>>().join(", ")
                }
            };
            println!("  {:<14} {ms:>6} ms  {status}", p.name());
        }

        if let Some(vid) = vid {
            // Through the daemon handlers: YTM lookup of the metadata.
            let a = Args {
                video_id: vid.clone(),
                ..Default::default()
            };
            let best = lyrics_get(&svc, &http, &ytm, &a).await.unwrap();
            println!("  lyrics_get    → {:?}", best.as_ref().map(describe));
            let sources = lyrics_sources(&svc, &http, &ytm, &a).await.unwrap();
            println!(
                "  lyrics_sources → {:?}",
                sources
                    .iter()
                    .map(|s| (&s.source, s.synced))
                    .collect::<Vec<_>>()
            );
            if best.is_some() {
                assert!(
                    !sources.is_empty(),
                    "all providers found less than the chain"
                );
            }
            // Cached now: no query needed.
            let key = Key {
                video_id: &vid,
                isrc: None,
            };
            let cached = svc
                .get(&http, key, || async { panic!("cache miss") })
                .await
                .unwrap();
            assert_eq!(cached.is_some(), best.is_some());
        }
    }
    println!(
        "\nworking today: {:?}",
        working.iter().map(|p| p.name()).collect::<Vec<_>>()
    );
    assert!(!working.is_empty(), "no lyrics provider answered");
}
