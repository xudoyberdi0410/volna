//! `catalog_*`: YouTube Music catalogue for the native UI. Results are the
//! [`volna_ytm`] models serialised as is (snake_case field names, `Item`
//! tagged by `type`).
//!
//! | command                 | args                                | result              |
//! |-------------------------|-------------------------------------|---------------------|
//! | `catalog_search`        | `query`, `filter?` (`SearchFilter`) | `SearchPage`        |
//! | `catalog_search_more`   | `continuation`, `filter?`           | `SearchPage`        |
//! | `catalog_suggestions`   | `input`                             | `SearchSuggestions` |
//! | `catalog_album`         | `id` (`MPREb_…` or `OLAK5uy_…`)     | `Album`             |
//! | `catalog_artist`        | `id` (`UC…`)                        | `Artist`            |
//! | `catalog_playlist`      | `id` (with or without `VL`)         | `Playlist`          |
//! | `catalog_playlist_more` | `continuation`                      | `TrackPage`         |
//! | `catalog_track`         | `videoId`                           | `TrackDetails`      |
//! | `catalog_related`       | `videoId`                           | `Related`           |
//! | `catalog_home`          | `minSections?` (default 6)          | `Section[]`         |
//! | `catalog_moods`         | —                                   | `MoodCategory[]`    |
//! | `catalog_mood`          | `params` (from `Mood`)              | `Section[]`         |

use serde_json::Value;
use volna_ytm::{SearchFilter, YtmClient};

use crate::dispatch::{arg, opt, out};

/// Home feed sections loaded when `minSections` is not given.
const DEFAULT_HOME_SECTIONS: usize = 6;

/// `"<kind>: <message>"`, see [`crate::api`].
fn err(e: volna_ytm::Error) -> String {
    match e {
        volna_ytm::Error::NotFound(what) => format!("not_found: {what}"),
        other => format!("upstream: {other}"),
    }
}

/// A YouTube id: `[A-Za-z0-9_-]`, 1..=128 chars (keeps junk out of browse ids).
fn id(a: &Value, key: &str) -> Result<String, String> {
    let v: String = arg(a, key).map_err(|e| format!("bad_args: {e}"))?;
    let ok = !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(v)
    } else {
        Err(format!("bad_args: invalid `{key}` {v:?}"))
    }
}

/// A non-empty string argument.
fn text(a: &Value, key: &str) -> Result<String, String> {
    let v: String = arg(a, key).map_err(|e| format!("bad_args: {e}"))?;
    if v.trim().is_empty() {
        Err(format!("bad_args: empty `{key}`"))
    } else {
        Ok(v)
    }
}

fn filter(a: &Value) -> Result<SearchFilter, String> {
    Ok(opt(a, "filter")
        .map_err(|e| format!("bad_args: {e}"))?
        .unwrap_or_default())
}

/// Runs a `catalog_*` command; `None` when `cmd` is not one.
pub async fn call(ytm: &YtmClient, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    if !cmd.starts_with("catalog_") {
        return None;
    }
    Some(run(ytm, cmd, a).await)
}

async fn run(ytm: &YtmClient, cmd: &str, a: &Value) -> Result<Value, String> {
    match cmd {
        "catalog_search" => {
            let (q, f) = (text(a, "query")?, filter(a)?);
            out(ytm.search(&q, f).await.map_err(err))
        }
        "catalog_search_more" => {
            let (c, f) = (text(a, "continuation")?, filter(a)?);
            out(ytm.search_more(&c, f).await.map_err(err))
        }
        "catalog_suggestions" => out(ytm
            .search_suggestions(&text(a, "input")?)
            .await
            .map_err(err)),
        "catalog_album" => out(ytm.album(&id(a, "id")?).await.map_err(err)),
        "catalog_artist" => out(ytm.artist(&id(a, "id")?).await.map_err(err)),
        "catalog_playlist" => out(ytm.playlist(&id(a, "id")?).await.map_err(err)),
        "catalog_playlist_more" => out(ytm
            .playlist_more(&text(a, "continuation")?)
            .await
            .map_err(err)),
        "catalog_track" => out(ytm.track(&id(a, "videoId")?).await.map_err(err)),
        "catalog_related" => out(ytm.related(&id(a, "videoId")?).await.map_err(err)),
        "catalog_home" => {
            let n = opt::<usize>(a, "minSections")
                .map_err(|e| format!("bad_args: {e}"))?
                .unwrap_or(DEFAULT_HOME_SECTIONS);
            out(ytm.home(n.clamp(1, 50)).await.map_err(err))
        }
        "catalog_moods" => out(ytm.moods_and_genres().await.map_err(err)),
        "catalog_mood" => out(ytm.mood_playlists(&text(a, "params")?).await.map_err(err)),
        other => Err(format!("unknown command {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ytm() -> YtmClient {
        YtmClient::new(reqwest::Client::new())
    }

    #[tokio::test]
    async fn other_commands_pass_through() {
        assert!(call(&ytm(), "player_play", &json!({})).await.is_none());
        assert!(call(&ytm(), "wave_start", &json!({})).await.is_none());
    }

    #[tokio::test]
    async fn bad_args_fail_before_any_request() {
        let y = ytm();
        let cases = [
            ("catalog_album", json!({})),
            ("catalog_album", json!({"id": "../etc"})),
            ("catalog_artist", json!({"id": ""})),
            ("catalog_track", json!({"id": "dQw4w9WgXcQ"})), // key is videoId
            ("catalog_search", json!({"query": "  "})),
            (
                "catalog_search",
                json!({"query": "x", "filter": "podcasts"}),
            ),
            ("catalog_search_more", json!({"continuation": null})),
            ("catalog_playlist_more", json!({})),
            ("catalog_suggestions", json!({"input": 5})),
            ("catalog_home", json!({"minSections": "many"})),
        ];
        for (cmd, args) in cases {
            let e = call(&y, cmd, &args).await.unwrap().unwrap_err();
            assert!(e.starts_with("bad_args: "), "{cmd} {args}: {e}");
        }
        let e = call(&y, "catalog_nope", &json!({})).await.unwrap();
        assert!(e.unwrap_err().contains("unknown command"));
    }

    #[test]
    fn filters_use_snake_case() {
        assert_eq!(filter(&json!({})).unwrap(), SearchFilter::All);
        assert_eq!(
            filter(&json!({"filter": "playlists"})).unwrap(),
            SearchFilter::Playlists
        );
    }

    #[test]
    fn errors_have_a_kind() {
        assert_eq!(
            err(volna_ytm::Error::NotFound("album x".into())),
            "not_found: album x"
        );
        assert!(err(volna_ytm::Error::Bootstrap("x".into())).starts_with("upstream: "));
    }
}

/// The catalogue against the real YouTube Music, through the same
/// [`crate::dispatch`] path the window uses. Run like `api_live` (CLAUDE.md):
/// `VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 VOLNA_TEST_COOKIES=chrome
/// cargo test -p volna-daemon api_live -- --ignored --nocapture --test-threads=1`
#[cfg(test)]
mod live {
    use super::*;
    use serde_json::json;

    use crate::protocol::tests::test_state;

    async fn live_ytm() -> YtmClient {
        let state = test_state();
        if let Ok(proxy) = std::env::var("VOLNA_TEST_PROXY") {
            crate::network::switch(&state, volna_net::ProxyConfig::Url(proxy))
                .await
                .expect("VOLNA_TEST_PROXY");
        }
        state.api.ytm()
    }

    async fn ok(ytm: &YtmClient, cmd: &str, args: Value) -> Value {
        call(ytm, cmd, &args)
            .await
            .expect("a catalog command")
            .unwrap_or_else(|e| panic!("{cmd} {args}: {e}"))
    }

    fn arr<'a>(v: &'a Value, key: &str) -> &'a Vec<Value> {
        v[key]
            .as_array()
            .unwrap_or_else(|| panic!("`{key}` is not an array: {v}"))
    }

    fn assert_track(t: &Value) {
        assert!(
            t["video_id"].as_str().is_some_and(|id| id.len() == 11),
            "{t}"
        );
        assert!(t["title"].as_str().is_some_and(|s| !s.is_empty()), "{t}");
        assert!(t["thumbnails"].is_array(), "{t}");
    }

    #[tokio::test]
    #[ignore = "network: music.youtube.com"]
    async fn api_live_catalog_search_album_artist_playlist() {
        let ytm = live_ytm().await;
        let q = "Скриптонит";

        let all = ok(&ytm, "catalog_search", json!({ "query": q })).await;
        println!(
            "search all: top {} + {} items",
            all["top_result"]["type"],
            arr(&all, "items").len()
        );
        assert!(!arr(&all, "items").is_empty());

        let songs = ok(
            &ytm,
            "catalog_search",
            json!({"query": q, "filter": "songs"}),
        )
        .await;
        let items = arr(&songs, "items");
        assert!(!items.is_empty());
        items.iter().for_each(|i| {
            assert_eq!(i["type"], "track", "{i}");
            assert_track(i);
        });
        let cont = songs["continuation"].as_str().expect("songs page more");
        let more = ok(
            &ytm,
            "catalog_search_more",
            json!({"continuation": cont, "filter": "songs"}),
        )
        .await;
        println!(
            "songs: {} + {} more",
            items.len(),
            arr(&more, "items").len()
        );
        assert!(!arr(&more, "items").is_empty());

        let sugg = ok(&ytm, "catalog_suggestions", json!({"input": "скрипт"})).await;
        assert!(!arr(&sugg, "queries").is_empty(), "{sugg}");

        let albums = ok(
            &ytm,
            "catalog_search",
            json!({"query": q, "filter": "albums"}),
        )
        .await;
        let album_id = arr(&albums, "items")[0]["browse_id"]
            .as_str()
            .expect("album browse_id")
            .to_owned();
        let album = ok(&ytm, "catalog_album", json!({ "id": album_id })).await;
        println!(
            "album {}: {} tracks",
            album["title"],
            arr(&album, "tracks").len()
        );
        assert!(!arr(&album, "tracks").is_empty());
        arr(&album, "tracks").iter().for_each(assert_track);

        let artist_id = album["artists"][0]["id"]
            .as_str()
            .expect("album artist id")
            .to_owned();
        let artist = ok(&ytm, "catalog_artist", json!({ "id": artist_id })).await;
        println!(
            "artist {}: {} top songs, {} albums, {} singles",
            artist["name"],
            arr(&artist, "top_songs").len(),
            arr(&artist, "albums").len(),
            arr(&artist, "singles").len()
        );
        assert!(!arr(&artist, "top_songs").is_empty());

        let pls = ok(
            &ytm,
            "catalog_search",
            json!({"query": format!("{q} playlist"), "filter": "playlists"}),
        )
        .await;
        let pl_id = arr(&pls, "items")[0]["id"]
            .as_str()
            .expect("playlist id")
            .to_owned();
        let pl = ok(&ytm, "catalog_playlist", json!({ "id": pl_id })).await;
        println!(
            "playlist {}: {} tracks, more: {}",
            pl["title"],
            arr(&pl, "tracks").len(),
            pl["continuation"].is_string()
        );
        assert!(!arr(&pl, "tracks").is_empty());
        if let Some(c) = pl["continuation"].as_str() {
            let page = ok(&ytm, "catalog_playlist_more", json!({"continuation": c})).await;
            assert!(!arr(&page, "tracks").is_empty());
        }

        let vid = items[0]["video_id"].as_str().unwrap().to_owned();
        let track = ok(&ytm, "catalog_track", json!({ "videoId": vid })).await;
        assert_eq!(track["track"]["video_id"], vid.as_str());
        let related = ok(&ytm, "catalog_related", json!({ "videoId": vid })).await;
        println!("related: {} tracks", arr(&related, "tracks").len());
        assert!(!arr(&related, "tracks").is_empty());
    }
}
