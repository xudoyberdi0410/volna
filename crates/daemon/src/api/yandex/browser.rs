//! Desktop only: the token from the Yandex session of the browser yt-dlp takes
//! cookies from (Settings → «Сеть»). The mobile core has no yt-dlp and signs
//! in through a webview instead; everything else in `api::yandex` is shared.

use std::path::Path;
use std::sync::Arc;

use volna_streams::YtDlp;

use super::client::token_from_cookies;
use super::{TokenSource, YandexState};
use crate::api::library::Library;
use crate::state::AppState;

/// The daemon's Yandex state: the browser session through the app's yt-dlp
/// (with the cookies source from Settings → «Сеть»), the jar dumped to
/// `cache_dir`; matches go into `library`.
pub fn desktop_state(app: &AppState, cache_dir: &Path, library: Arc<Library>) -> Arc<YandexState> {
    let browser: Option<TokenSource> = app.streams.clone().map(|streams| {
        let dir = cache_dir.to_owned();
        let source: TokenSource = Arc::new(move || {
            let (streams, dir) = (streams.clone(), dir.clone());
            Box::pin(async move { browser_token(streams.backend(), &dir).await })
        });
        source
    });
    Arc::new(YandexState::new(library, browser))
}

/// A token, or `None` (no browser set, not signed in there, cookies unreadable).
/// The cookie jar is written to `scratch_dir` and removed right after reading.
pub async fn browser_token(ytdlp: &YtDlp, scratch_dir: &Path) -> Option<String> {
    std::fs::create_dir_all(scratch_dir).ok()?;
    let path = scratch_dir.join(format!("yandex-cookies-{}.txt", std::process::id()));
    let dumped = ytdlp.dump_cookies(&path).await;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    // The jar holds every site's session; keep it on disk no longer than needed.
    let _ = std::fs::remove_file(&path);
    match dumped {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            tracing::warn!("yandex: cannot read the browser cookies: {e}");
            return None;
        }
    }
    token_from_cookies(&text).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::yandex::client::{Client, export};

    /// The whole library through the real browser session:
    /// `VOLNA_TEST_COOKIES=chrome cargo test -p volna-daemon yandex_live -- --ignored --nocapture`.
    /// Prints counts only, never cookies or the token.
    #[tokio::test]
    #[ignore]
    async fn yandex_live_export_from_browser() {
        let browser = std::env::var("VOLNA_TEST_COOKIES").unwrap_or_else(|_| "chrome".into());
        let ytdlp = YtDlp::locate(None).expect("yt-dlp");
        ytdlp
            .set_cookies(volna_streams::Cookies::Browser(browser))
            .unwrap();
        let token = browser_token(&ytdlp, &std::env::temp_dir())
            .await
            .expect("signed in to Yandex in the browser");
        let client = Client::new(token).unwrap();
        let library = export(&client, &|stage, detail| {
            println!("{stage} {}", detail.unwrap_or_default())
        })
        .await
        .unwrap();
        let in_playlists: usize = library.playlists.iter().map(|p| p.tracks.len()).sum();
        println!(
            "likes {}, playlists {} ({in_playlists} tracks), albums {}, artists {}",
            library.liked_tracks.len(),
            library.playlists.len(),
            library.albums.len(),
            library.artists.len()
        );
        assert!(!library.liked_tracks.is_empty());
        assert!(
            library
                .liked_tracks
                .iter()
                .all(|t| !t.track.artists.is_empty())
        );
    }

    /// Browser token → library → the first 25 likes, 2 playlists, 5 albums and
    /// 5 artists matched on YouTube Music (through `VOLNA_TEST_PROXY`) and saved
    /// into an in-memory library:
    /// `VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome cargo test -p volna-daemon yandex_live_import -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn yandex_live_import() {
        use crate::api::yandex::import::{match_library, save, tests::MemorySink};
        let browser = std::env::var("VOLNA_TEST_COOKIES").unwrap_or_else(|_| "chrome".into());
        let ytdlp = YtDlp::locate(None).expect("yt-dlp");
        ytdlp
            .set_cookies(volna_streams::Cookies::Browser(browser))
            .unwrap();
        let token = browser_token(&ytdlp, &std::env::temp_dir())
            .await
            .expect("signed in to Yandex in the browser");
        let client = Client::new(token).unwrap();
        let mut library = export(&client, &|_, _| {}).await.unwrap();
        library.liked_tracks.truncate(25);
        library.playlists.truncate(2);
        for p in &mut library.playlists {
            p.tracks.truncate(10);
        }
        library.albums.truncate(5);
        library.artists.truncate(5);

        let ytm = crate::api::yandex::matching::tests::live_ytm();
        let started = std::time::Instant::now();
        let matched = match_library(&ytm, &library, &|_| {}).await;
        println!(
            "matched {} of {} tracks in {:?}; albums {}/{}, artists {}/{}",
            matched.tracks.len(),
            matched.total,
            started.elapsed(),
            matched.albums.len(),
            library.albums.len(),
            matched.artists.len(),
            library.artists.len()
        );
        for m in &matched.misses {
            println!("miss: {} — {}", m.artist, m.title);
        }
        let sink = MemorySink::default();
        let summary = save(&sink, &library, &matched).unwrap();
        println!("{summary:?}");
        assert!(
            matched.tracks.len() * 10 >= matched.total * 7,
            "under 70% matched"
        );
    }
}
