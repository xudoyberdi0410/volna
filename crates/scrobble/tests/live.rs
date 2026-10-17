//! Live checks against the real services; skipped by default.
//!
//! ```sh
//! cargo test -p volna-scrobble --test live -- --ignored --nocapture
//! # optional: VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081
//! #           VOLNA_TEST_LISTENBRAINZ_TOKEN=…  (sends a "playing now")
//! #           VOLNA_TEST_MALOJA_URL=… VOLNA_TEST_MALOJA_KEY=…  (sends a scrobble)
//! ```

use volna_scrobble::audioscrobbler::Endpoint;
use volna_scrobble::listenbrainz::ListenBrainz;
use volna_scrobble::maloja::Maloja;
use volna_scrobble::{Http, ReqwestHttp, Scrobbler, Track, now_s};

fn http() -> ReqwestHttp {
    let proxy = std::env::var("VOLNA_TEST_PROXY").ok();
    ReqwestHttp::new(move || {
        let mut b = reqwest::Client::builder();
        if let Some(p) = &proxy {
            b = b.proxy(reqwest::Proxy::all(p).expect("proxy url"));
        }
        b.build().expect("client")
    })
}

fn track() -> Track {
    Track {
        title: "Bohemian Rhapsody".into(),
        artists: vec!["Queen".into()],
        album: Some("A Night at the Opera".into()),
        duration: Some(355.0),
        track_number: None,
        video_id: Some("fJ9rUzIMcZQ".into()),
    }
}

/// The desktop auth's first step works with the app's default key.
#[tokio::test]
#[ignore]
async fn lastfm_token() {
    let h = http();
    let e = Endpoint::lastfm(None);
    let token = e.get_token(&h as &dyn Http).await.expect("auth.getToken");
    println!("open {}", e.auth_page(&token));
    assert!(!token.is_empty());
}

#[tokio::test]
#[ignore]
async fn librefm_token() {
    let h = http();
    let e = Endpoint::librefm();
    let token = e.get_token(&h as &dyn Http).await.expect("auth.getToken");
    println!("open {}", e.auth_page(&token));
}

#[tokio::test]
#[ignore]
async fn listenbrainz_lookup_and_now_playing() {
    let Ok(token) = std::env::var("VOLNA_TEST_LISTENBRAINZ_TOKEN") else {
        eprintln!("VOLNA_TEST_LISTENBRAINZ_TOKEN not set; skipping");
        return;
    };
    let h = http();
    let lb = ListenBrainz::new(None, &token);
    let mbids = lb.lookup(&h, &track()).await;
    println!("mbids: {mbids:?}");
    lb.now_playing(&h, &track()).await.expect("playing_now");
}

#[tokio::test]
#[ignore]
async fn maloja_scrobble() {
    let (Ok(url), Ok(key)) = (
        std::env::var("VOLNA_TEST_MALOJA_URL"),
        std::env::var("VOLNA_TEST_MALOJA_KEY"),
    ) else {
        eprintln!("VOLNA_TEST_MALOJA_URL / _KEY not set; skipping");
        return;
    };
    let h = http();
    Maloja::new(&url, &key)
        .scrobble(&h, &track(), now_s() - 400)
        .await
        .expect("newscrobble");
}
