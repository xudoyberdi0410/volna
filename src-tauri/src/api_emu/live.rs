//! End-to-end checks of the emulator against the real YouTube Music, through
//! the same `protocol::handle` entry the webview uses.
//!
//! `cargo test -p volna api_live -- --ignored --nocapture --test-threads=1`

use http::{Request, StatusCode, header};
use serde_json::Value;

use crate::protocol::handle;
use crate::protocol::tests::test_state;
use crate::state::AppState;

async fn get(state: &AppState, uri: &str) -> Value {
    let res = handle(state, Request::get(uri).body(Vec::new()).unwrap()).await;
    let status = res.status();
    let body = String::from_utf8_lossy(res.body()).into_owned();
    assert_eq!(status, StatusCode::OK, "{uri}: {body}");
    assert_eq!(res.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("{uri}: {e}: {body}"))
}

fn arr<'a>(v: &'a Value, what: &str) -> &'a Vec<Value> {
    v.as_array()
        .unwrap_or_else(|| panic!("{what} is not an array: {v}"))
}

fn assert_track(t: &Value) {
    assert!(t["id"].as_str().is_some_and(|id| id.len() == 11), "{t}");
    assert!(t["title"].as_str().is_some_and(|s| !s.is_empty()), "{t}");
    assert_eq!(t["type"], "track");
    assert!(!arr(&t["artists"], "artists").is_empty());
    assert!(t["album"].is_object(), "album is required: {t}");
    assert!(t["album"].get("cover").is_some(), "{t}");
}

#[tokio::test]
#[ignore = "network: music.youtube.com"]
async fn api_live_search_album_artist_radio() {
    let state = test_state();
    let q = "volna://api/search/?q=%D0%A1%D0%BA%D1%80%D0%B8%D0%BF%D1%82%D0%BE%D0%BD%D0%B8%D1%82";

    // Combined search fills every bucket the UI may ask for.
    let search = get(&state, q).await;
    let data = &search["data"];
    for key in ["tracks", "artists", "albums", "playlists", "videos"] {
        assert!(data[key]["items"].is_array(), "bucket {key}");
    }
    let tracks = arr(&data["tracks"]["items"], "tracks");
    let artists = arr(&data["artists"]["items"], "artists");
    let albums = arr(&data["albums"]["items"], "albums");
    println!(
        "search: {} tracks, {} artists, {} albums, {} playlists",
        tracks.len(),
        artists.len(),
        albums.len(),
        arr(&data["playlists"]["items"], "playlists").len()
    );
    assert!(!tracks.is_empty() && !artists.is_empty() && !albums.is_empty());
    tracks.iter().for_each(assert_track);
    assert!(tracks.iter().all(|t| t["duration"].as_u64() > Some(0)));
    assert!(
        artists[0]["picture"].as_str().is_some(),
        "artist search items carry a picture"
    );

    // First album: every track filled, numberOfTracks matches.
    let album_id = albums[0]["id"].as_str().unwrap();
    assert!(album_id.starts_with("MPREb_"), "{album_id}");
    let album = get(&state, &format!("volna://api/album/?id={album_id}")).await;
    let a = &album["data"];
    let items = arr(&a["items"], "album items");
    println!("album {album_id}: {} — {} tracks", a["title"], items.len());
    assert_eq!(a["numberOfTracks"].as_u64(), Some(items.len() as u64));
    assert!(a["artist"]["name"].as_str().is_some());
    assert!(a["cover"].as_str().is_some_and(|c| c.contains("img")));
    for i in items {
        assert_track(&i["item"]);
        assert_eq!(i["item"]["album"]["id"], album_id);
    }
    let page2 = get(
        &state,
        &format!(
            "volna://api/album/?id={album_id}&offset={}&limit=500",
            items.len()
        ),
    )
    .await;
    assert!(arr(&page2["data"]["items"], "page 2").is_empty());

    // First artist: profile, discography owned by the artist, top tracks.
    let artist_id = artists[0]["id"].as_str().unwrap();
    assert!(artist_id.starts_with("UC"), "{artist_id}");
    let artist = get(&state, &format!("volna://api/artist/?id={artist_id}")).await;
    assert_eq!(artist["artist"]["id"], artist_id);
    let discography = arr(&artist["albums"]["items"], "albums");
    let top = arr(&artist["tracks"], "top tracks");
    println!(
        "artist {}: {} releases, {} top tracks, mix {}",
        artist["artist"]["name"],
        discography.len(),
        top.len(),
        artist["artist"]["mixes"]["ARTIST_MIX"]
    );
    assert!(!discography.is_empty() && !top.is_empty());
    assert!(discography.iter().all(|a| a["artist"]["id"] == artist_id));
    for t in top {
        assert_track(t);
        assert!(
            arr(&t["artists"], "artists")
                .iter()
                .any(|a| a["id"] == artist_id)
        );
    }
    let paged = get(
        &state,
        &format!("volna://api/artist/?f={artist_id}&skip_tracks=true&offset=0&limit=15"),
    )
    .await;
    assert!(!arr(&paged["tracks"], "paged top tracks").is_empty());
    let similar = get(
        &state,
        &format!("volna://api/artist/similar/?id={artist_id}"),
    )
    .await;
    println!(
        "similar artists: {}",
        arr(&similar["artists"], "similar").len()
    );

    // Radio of the first track, as a mix and as recommendations.
    let seed = tracks[0]["id"].as_str().unwrap();
    let mix = get(&state, &format!("volna://api/mix/?id=RDAMVM{seed}")).await;
    let mix_items = arr(&mix["items"], "mix items");
    println!("mix: {} — {} tracks", mix["mix"]["title"], mix_items.len());
    assert_eq!(mix["mix"]["mixType"], "TRACK_MIX");
    assert!(mix_items.len() >= 20);
    assert_eq!(mix_items[0]["id"], seed, "a radio starts with its seed");
    mix_items.iter().for_each(assert_track);

    let recs = get(&state, &format!("volna://api/recommendations/?id={seed}")).await;
    let rec_items = arr(&recs["data"]["items"], "recommendations");
    assert!(!rec_items.is_empty());
    assert!(
        rec_items.iter().all(|e| e["item"]["id"] != seed),
        "seed excluded"
    );

    let similar_albums = get(&state, &format!("volna://api/album/similar/?id={album_id}")).await;
    println!(
        "similar albums: {}",
        arr(&similar_albums["albums"], "albums").len()
    );

    let info = get(&state, &format!("volna://api/info/?id={seed}")).await;
    assert_eq!(info["data"]["id"], seed);
}

#[tokio::test]
#[ignore = "network: music.youtube.com + googlevideo, needs yt-dlp"]
async fn api_live_search_then_stream_range() {
    let state = test_state();
    assert!(state.streams.is_some(), "yt-dlp not found");
    let search = get(
        &state,
        "volna://api/search/?s=%D0%A1%D0%BA%D1%80%D0%B8%D0%BF%D1%82%D0%BE%D0%BD%D0%B8%D1%82",
    )
    .await;
    let ids: Vec<String> = arr(&search["data"]["tracks"]["items"], "tracks")
        .iter()
        .filter_map(|t| t["id"].as_str().map(str::to_owned))
        .take(5)
        .collect();
    assert!(!ids.is_empty());

    // From a datacenter exit YouTube asks yt-dlp to "confirm you're not a
    // bot" for many videos; take the first one that resolves.
    let mut served = None;
    for id in &ids {
        let req = Request::get(format!("volna://stream/{id}"))
            .header(header::RANGE, "bytes=0-65535")
            .body(Vec::new())
            .unwrap();
        let res = handle(&state, req).await;
        if res.status() == StatusCode::PARTIAL_CONTENT {
            served = Some((id.clone(), res));
            break;
        }
        println!(
            "stream {id}: {} {}",
            res.status(),
            String::from_utf8_lossy(res.body())
        );
    }
    let (id, res) = served.expect("none of the first search results could be streamed");
    assert_eq!(res.body().len(), 65536);
    let ct = res.headers()[header::CONTENT_TYPE].to_str().unwrap();
    assert!(ct.starts_with("audio/"), "{ct}");
    println!(
        "stream {id}: {ct}, {}",
        res.headers()[header::CONTENT_RANGE].to_str().unwrap()
    );
}

#[tokio::test]
#[ignore = "network: music.youtube.com"]
async fn api_live_home_explore_lyrics_playlist() {
    let state = test_state();
    let home = get(&state, "volna://api/home/").await;
    let sections = arr(&home["sections"], "home sections");
    println!("home: {} sections", sections.len());
    assert!(!sections.is_empty());

    let explore = get(&state, "volna://api/explore/").await;
    let cats = arr(&explore["categories"], "categories");
    assert!(!cats.is_empty());
    let genre_id = cats[0]["genres"][0]["id"].as_str().unwrap();
    let genre = get(
        &state,
        &format!(
            "volna://api/explore/genre/?id={}",
            url::form_urlencoded::byte_serialize(genre_id.as_bytes()).collect::<String>()
        ),
    )
    .await;
    let genre_sections = arr(&genre["sections"], "genre sections");
    println!(
        "genre {}: {} sections",
        cats[0]["genres"][0]["name"],
        genre_sections.len()
    );
    assert!(!genre_sections.is_empty());

    // A playlist from a genre page pages through `offset`.
    let pl_id = genre_sections
        .iter()
        .flat_map(|s| arr(&s["items"], "items"))
        .find_map(|i| i["uuid"].as_str())
        .expect("a playlist in the genre page")
        .to_owned();
    let pl = get(&state, &format!("volna://api/playlist/?id={pl_id}")).await;
    let n = pl["playlist"]["numberOfTracks"].as_u64().unwrap();
    let first = arr(&pl["items"], "playlist items");
    println!("playlist {pl_id}: {} / {n}", first.len());
    assert!(!first.is_empty());
    first.iter().for_each(|i| assert_track(&i["item"]));
    let rest = get(
        &state,
        &format!("volna://api/playlist/?id={pl_id}&offset={}", first.len()),
    )
    .await;
    let loaded = (first.len() + arr(&rest["items"], "rest").len()) as u64;
    assert!(loaded >= n.min(200), "{loaded} of {n}");

    // A song (not a music video) with Musixmatch lyrics on YouTube Music.
    let search = get(
        &state,
        "volna://api/search/?s=rick%20astley%20never%20gonna%20give%20you%20up",
    )
    .await;
    let song = search["data"]["tracks"]["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let lyrics = get(&state, &format!("volna://api/lyrics/?id={song}")).await;
    let text = lyrics["data"]["text"].as_str().unwrap().to_lowercase();
    assert!(text.contains("give you up"), "{text}");
    println!(
        "lyrics of {song}: {} chars from {}",
        text.len(),
        lyrics["data"]["source"]
    );

    let suggest = get(
        &state,
        "volna://api/search/suggest/?q=%D1%81%D0%BA%D1%80%D0%B8%D0%BF",
    )
    .await;
    assert!(!arr(&suggest["data"], "suggestions").is_empty());
}

#[tokio::test]
#[ignore = "network: lrclib.net"]
async fn api_live_lyrics_proxy() {
    let state = test_state();
    let u = "https://lrclib.net/api/search?q=never%20gonna%20give%20you%20up";
    let uri = format!(
        "volna://api/proxy/?u={}",
        url::form_urlencoded::byte_serialize(u.as_bytes()).collect::<String>()
    );
    let v = get(&state, &uri).await;
    assert!(!arr(&v, "lrclib results").is_empty());
}
