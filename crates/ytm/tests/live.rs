//! Live checks against music.youtube.com. Ignored by default:
//! `cargo test -p volna-ytm --test live -- --ignored --test-threads=2`
//!
//! Assertions stay loose (catalogues and charts change); they check that
//! each endpoint answers and that the parser finds the expected shape.

use volna_ytm::{AlbumKind, Item, SearchFilter, TrackKind, YtmClient};

const ISHON: &str = "TVwp8oqme6I"; // Shahzoda — Ishon
const PRIVYCHKA: &str = "afFEuD8Ki8E"; // Скриптонит — Привычка (has lyrics)
const SKRIPTONIT: &str = "UC7IxSgtALkMNWram-ZGQyaA";
const SHAHZODA: &str = "UCo6BC71FZJiDPwMWdjfAzvg";

fn client() -> YtmClient {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    YtmClient::new(http)
}

#[tokio::test]
#[ignore = "live network"]
async fn bootstrap_reads_current_client_version() {
    let v = client().client_version().await.unwrap();
    assert!(v.starts_with("1.20"), "{v}");
}

#[tokio::test]
#[ignore = "live network"]
async fn search_all_russian() {
    let page = client()
        .search("Скриптонит", SearchFilter::All)
        .await
        .unwrap();
    let top = page.top_result.expect("top result");
    assert!(
        matches!(&top, Item::Artist(a) if a.id == SKRIPTONIT),
        "{top:?}"
    );
    assert!(page.items.len() > 10);
}

#[tokio::test]
#[ignore = "live network"]
async fn search_songs_uzbek_and_continue() {
    let ytm = client();
    let page = ytm.search("Shahzoda", SearchFilter::Songs).await.unwrap();
    assert!(page.items.len() >= 15);
    let t = page.items[0].as_track().unwrap();
    assert_eq!(t.kind, TrackKind::Song);
    assert!(t.duration_s.is_some());
    assert!(
        page.items
            .iter()
            .filter_map(Item::as_track)
            .any(|t| t.artists.iter().any(|a| a.id.as_deref() == Some(SHAHZODA)))
    );
    let more = ytm
        .search_more(&page.continuation.unwrap(), SearchFilter::Songs)
        .await
        .unwrap();
    assert!(!more.items.is_empty());
}

#[tokio::test]
#[ignore = "live network"]
async fn search_each_filter() {
    let ytm = client();
    for (q, f) in [
        ("Shahzoda", SearchFilter::Albums),
        ("Скриптонит", SearchFilter::Artists),
        ("узбекские песни", SearchFilter::Playlists),
        ("Скриптонит", SearchFilter::Videos),
    ] {
        let page = ytm.search(q, f).await.unwrap();
        assert!(!page.items.is_empty(), "{q} {f:?}");
        let ok = page.items.iter().all(|i| match f {
            SearchFilter::Albums => i.as_album().is_some(),
            SearchFilter::Artists => i.as_artist().is_some(),
            SearchFilter::Playlists => i.as_playlist().is_some(),
            SearchFilter::Videos => i.as_track().is_some_and(|t| t.kind != TrackKind::Song),
            _ => true,
        });
        assert!(ok, "{q} {f:?}");
    }
}

#[tokio::test]
#[ignore = "live network"]
async fn suggestions_cyrillic() {
    let s = client().search_suggestions("скрип").await.unwrap();
    assert!(
        s.queries.iter().any(|q| q.contains("скриптонит")),
        "{:?}",
        s.queries
    );
}

#[tokio::test]
#[ignore = "live network"]
async fn track_details() {
    let d = client().track(ISHON).await.unwrap();
    assert_eq!(d.track.title, "Ishon");
    assert_eq!(d.track.artists[0].name, "Shahzoda");
    assert!(d.track.album.is_some());
    assert_eq!(d.track.duration_s, Some(205));
    assert!(d.related_browse_id.is_some());
}

#[tokio::test]
#[ignore = "live network"]
async fn radio_with_continuation() {
    let ytm = client();
    let q = ytm.radio(ISHON).await.unwrap();
    assert_eq!(q.tracks[0].video_id, ISHON);
    assert!(q.tracks.len() >= 20);
    let more = ytm.queue_more(&q.continuation.unwrap()).await.unwrap();
    assert!(more.tracks.len() >= 10);
    assert!(more.continuation.is_some());
}

#[tokio::test]
#[ignore = "live network"]
async fn watch_album_playlist() {
    let q = client()
        .watch_playlist("OLAK5uy_l3kAfpHbiAsam0TwQnsdjuFCqebkpm-c0")
        .await
        .unwrap();
    assert!(q.tracks.len() >= 10, "{}", q.tracks.len());
}

#[tokio::test]
#[ignore = "live network"]
async fn album_by_browse_id_and_by_olak() {
    let ytm = client();
    let a = ytm.album("MPREb_9CC1cAMRQuj").await.unwrap();
    assert_eq!(a.title, "Qora ko'zlaring");
    assert_eq!(a.kind, AlbumKind::Album);
    assert_eq!(a.tracks.len(), 13);
    let b = ytm
        .album("OLAK5uy_l3kAfpHbiAsam0TwQnsdjuFCqebkpm-c0")
        .await
        .unwrap();
    assert_eq!(b.browse_id, "MPREb_9CC1cAMRQuj");
}

#[tokio::test]
#[ignore = "live network"]
async fn artist_page() {
    let a = client().artist(SKRIPTONIT).await.unwrap();
    assert_eq!(a.name, "Skryptonite");
    assert!(!a.top_songs.is_empty());
    assert!(!a.albums.is_empty());
    assert!(!a.singles.is_empty());
    assert!(!a.related.is_empty());
    assert!(a.top_songs_playlist_id.is_some());
}

#[tokio::test]
#[ignore = "live network"]
async fn playlist_paging() {
    let ytm = client();
    let first = ytm
        .playlist("PLOHoVaTp8R7c-vblRA0F_hOPCSMSwqu3z")
        .await
        .unwrap();
    assert!(first.tracks.len() >= 50);
    assert!(first.continuation.is_some());
    let all = ytm
        .playlist_all("PLOHoVaTp8R7c-vblRA0F_hOPCSMSwqu3z", Some(500))
        .await
        .unwrap();
    assert!(all.tracks.len() > first.tracks.len());
    let ids: std::collections::HashSet<_> = all.tracks.iter().map(|t| &t.video_id).collect();
    assert!(ids.len() > first.tracks.len());
}

#[tokio::test]
#[ignore = "live network"]
async fn related_tab() {
    let r = client().related(ISHON).await.unwrap();
    assert!(!r.tracks.is_empty());
    assert!(!r.artists.is_empty());
}

#[tokio::test]
#[ignore = "live network"]
async fn lyrics_present_and_absent() {
    let ytm = client();
    let l = ytm.lyrics(PRIVYCHKA).await.unwrap().expect("lyrics");
    assert!(l.text.lines().count() > 10);
    assert!(l.source.is_some());
    assert!(ytm.lyrics(ISHON).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "live network"]
async fn moods_and_a_mood() {
    let ytm = client();
    let cats = ytm.moods_and_genres().await.unwrap();
    assert!(cats.len() >= 2);
    let mood = &cats[0].moods[0];
    let sections = ytm.mood_playlists(&mood.params).await.unwrap();
    assert!(!sections.is_empty(), "{}", mood.title);
    assert!(
        sections
            .iter()
            .any(|s| s.items.iter().any(|i| i.as_playlist().is_some()))
    );
}

#[tokio::test]
#[ignore = "live network"]
async fn home_feed() {
    let sections = client().home(6).await.unwrap();
    assert!(sections.len() >= 4, "{}", sections.len());
}

#[tokio::test]
#[ignore = "live network"]
async fn session_survives_client_swap() {
    let ytm = client();
    ytm.search("Shahzoda", SearchFilter::Artists).await.unwrap();
    ytm.set_http_client(reqwest::Client::new()).await;
    let p = ytm.search("Shahzoda", SearchFilter::Artists).await.unwrap();
    assert!(!p.items.is_empty());
}
