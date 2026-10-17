//! Desktop only: the token from the Yandex session of the browser yt-dlp takes
//! cookies from (Settings → «Сеть»). The mobile core has no yt-dlp and signs
//! in through a webview instead; everything else in `api::yandex` is shared.

use std::path::Path;

use volna_streams::YtDlp;

use super::client::token_from_cookies;

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
}
