//! Parsers against real InnerTube responses captured on 2026-10-16
//! (WEB_REMIX 1.20261013.15.00, hl=en, logged out). The fixtures had
//! tracking/menu noise stripped with jq; structure is untouched.

use serde_json::Value;
use volna_ytm::{AlbumKind, Item, SearchFilter, TrackKind, parsers};

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&raw).expect("fixture is JSON")
}

fn tracks(items: &[Item]) -> Vec<&volna_ytm::Track> {
    items.iter().filter_map(Item::as_track).collect()
}

#[test]
fn search_all_cyrillic_query() {
    let page = parsers::parse_search(&fixture("search_all"), SearchFilter::All);

    let Some(Item::Artist(top)) = &page.top_result else {
        panic!("top result should be the artist, got {:?}", page.top_result);
    };
    assert_eq!(top.id, "UC7IxSgtALkMNWram-ZGQyaA");
    assert_eq!(top.name, "Skryptonite");
    assert!(page.continuation.is_none());

    // The card's own songs omit the artist; it is filled from the card.
    let first = page.items[0].as_track().unwrap();
    assert_eq!(first.title, "Привычка");
    assert_eq!(first.video_id, "afFEuD8Ki8E");
    assert_eq!(first.artists[0].name, "Skryptonite");
    assert_eq!(first.duration_s, Some(197));
    assert!(first.explicit);

    // Every type shows up, podcasts and profiles are dropped.
    let has = |f: fn(&Item) -> bool| page.items.iter().any(f);
    assert!(has(|i| matches!(i, Item::Album(_))));
    assert!(has(|i| matches!(i, Item::Artist(_))));
    assert!(has(|i| matches!(i, Item::Playlist(_))));
    assert!(has(
        |i| matches!(i, Item::Track(t) if t.kind == TrackKind::Video)
    ));
    assert!(!page.items.iter().any(|i| match i {
        Item::Track(t) => t.title.contains("ИНТЕРВЬЮ"),
        _ => false,
    }));

    let single = page
        .items
        .iter()
        .filter_map(Item::as_album)
        .find(|a| a.title == "не расслабляйся")
        .unwrap();
    assert_eq!(single.kind, AlbumKind::Single);
    assert_eq!(single.year, Some(2024));
    assert_eq!(single.browse_id, "MPREb_zQjimfwWP2o");
    assert_eq!(
        single.playlist_id.as_deref(),
        Some("OLAK5uy_mvSE91a4AU3IHKLirR-qTGgfHFHTCh8P0")
    );

    let ytm_playlist = page
        .items
        .iter()
        .filter_map(Item::as_playlist)
        .find(|p| p.title == "Presenting Баста")
        .unwrap();
    assert_eq!(
        ytm_playlist.id,
        "RDCLAK5uy_kLaw49GbGpGx5yro0YPLicBbbqIHgK51w"
    );
    assert_eq!(ytm_playlist.track_count, Some(30));
    assert_eq!(ytm_playlist.author.as_ref().unwrap().name, "YouTube Music");

    // Names are trimmed.
    assert!(
        tracks(&page.items)
            .iter()
            .flat_map(|t| &t.artists)
            .all(|a| a.name == a.name.trim())
    );
}

#[test]
fn search_songs_uzbek_query_with_continuation() {
    let page = parsers::parse_search(&fixture("search_songs"), SearchFilter::Songs);
    assert_eq!(page.items.len(), 20);
    assert!(page.continuation.is_some());
    let t = page.items[1].as_track().unwrap();
    assert_eq!(t.title, "Ishon");
    assert_eq!(t.video_id, "TVwp8oqme6I");
    assert_eq!(t.artists[0].id.as_deref(), Some("UCo6BC71FZJiDPwMWdjfAzvg"));
    assert_eq!(t.album.as_ref().unwrap().name, "Qora ko'zlaring");
    assert_eq!(
        t.album.as_ref().unwrap().id.as_deref(),
        Some("MPREb_9CC1cAMRQuj")
    );
    assert_eq!(t.duration_s, Some(205));
    assert_eq!(t.kind, TrackKind::Song);
    assert!(!t.thumbnails.is_empty());

    let more =
        parsers::parse_search_continuation(&fixture("search_songs_cont"), SearchFilter::Songs);
    assert_eq!(more.items.len(), 20);
    assert!(more.continuation.is_some());
    // Unlinked featured artist is kept with no id.
    let feat = more.items[0].as_track().unwrap();
    assert_eq!(feat.artists.len(), 2);
    assert_eq!(feat.artists[1].id, None);
}

#[test]
fn search_other_filters() {
    let albums = parsers::parse_search(&fixture("search_albums"), SearchFilter::Albums);
    assert_eq!(albums.items.len(), 20);
    let duet = albums
        .items
        .iter()
        .filter_map(Item::as_album)
        .find(|a| a.title == "Im Shab")
        .unwrap();
    assert_eq!(duet.artists.len(), 2);
    assert_eq!(duet.kind, AlbumKind::Single);

    let artists = parsers::parse_search(&fixture("search_artists"), SearchFilter::Artists);
    let a = artists.items[0].as_artist().unwrap();
    assert_eq!(a.name, "Shahzoda");
    assert_eq!(a.audience.as_deref(), Some("7.75M monthly audience"));

    let pls = parsers::parse_search(&fixture("search_playlists"), SearchFilter::Playlists);
    assert_eq!(pls.items.len(), 20);
    let p = pls.items[0].as_playlist().unwrap();
    assert!(p.id.starts_with("PL"), "{}", p.id);
    assert!(p.author.is_some());

    let vids = parsers::parse_search(&fixture("search_videos"), SearchFilter::Videos);
    assert_eq!(vids.items.len(), 20);
    assert!(
        tracks(&vids.items)
            .iter()
            .all(|t| t.kind != TrackKind::Song)
    );
    assert!(tracks(&vids.items).iter().all(|t| t.duration_s.is_some()));
}

#[test]
fn filter_drops_foreign_types() {
    // An "all" page parsed as a songs search keeps only songs.
    let page = parsers::parse_search(&fixture("search_all"), SearchFilter::Songs);
    assert!(!page.items.is_empty());
    assert!(
        page.items
            .iter()
            .all(|i| matches!(i, Item::Track(t) if t.kind == TrackKind::Song))
    );
    assert!(page.top_result.is_some());
}

#[test]
fn suggestions() {
    let s = parsers::parse_suggestions(&fixture("suggestions"));
    assert_eq!(s.queries[0], "скриптонит");
    assert_eq!(s.queries.len(), 6);
    assert!(matches!(&s.items[0], Item::Artist(a) if a.name == "Skryptonite"));
    assert_eq!(s.items.len(), 4);
}

#[test]
fn watch_single_track_and_tabs() {
    let q = parsers::parse_watch(&fixture("next_track"));
    let t = &q.tracks[0];
    assert_eq!(t.video_id, "TVwp8oqme6I");
    assert_eq!(t.title, "Ishon");
    assert_eq!(t.artists[0].name, "Shahzoda");
    assert_eq!(
        t.album.as_ref().unwrap().id.as_deref(),
        Some("MPREb_9CC1cAMRQuj")
    );
    assert_eq!(t.year, Some(2011));
    assert_eq!(t.duration_s, Some(205));
    assert!(t.thumbnails.len() >= 4);
    // The lyrics tab is marked unselectable for this song: no lyrics.
    assert_eq!(q.lyrics_browse_id, None);
    // "Related" is the 4th tab now (after the new "Comments" tab).
    assert_eq!(q.related_browse_id.as_deref(), Some("MPTRt_9CC1cAMRQuj-5"));
}

#[test]
fn radio_and_continuation() {
    let q = parsers::parse_watch(&fixture("next_radio"));
    assert_eq!(q.playlist_id.as_deref(), Some("RDAMVMTVwp8oqme6I"));
    assert_eq!(q.tracks.len(), 50);
    assert_eq!(q.tracks[0].video_id, "TVwp8oqme6I");
    assert!(q.continuation.is_some());
    assert!(q.tracks.iter().all(|t| !t.artists.is_empty()));

    let more = parsers::parse_watch_continuation(&fixture("next_radio_cont"));
    assert_eq!(more.tracks.len(), 49);
    assert!(more.continuation.is_some());
}

#[test]
fn album_page() {
    let a = parsers::parse_album(&fixture("album"), "MPREb_9CC1cAMRQuj").unwrap();
    assert_eq!(a.title, "Qora ko'zlaring");
    assert_eq!(a.kind, AlbumKind::Album);
    assert_eq!(a.year, Some(2011));
    assert_eq!(a.artists[0].name, "Shahzoda");
    assert_eq!(
        a.playlist_id.as_deref(),
        Some("OLAK5uy_l3kAfpHbiAsam0TwQnsdjuFCqebkpm-c0")
    );
    assert_eq!(a.track_count, Some(13));
    assert_eq!(a.tracks.len(), 13);
    assert!(a.duration_s > 45 * 60);
    let first = &a.tracks[0];
    assert_eq!(first.video_id, "mUwV6ezpEx4");
    assert_eq!(first.duration_s, Some(225));
    // Album rows carry no cover or album link; both are filled in.
    assert_eq!(
        first.album.as_ref().unwrap().id.as_deref(),
        Some("MPREb_9CC1cAMRQuj")
    );
    assert_eq!(first.thumbnails, a.thumbnails);
    assert!(!a.related.is_empty());
}

#[test]
fn album_rows_linked_to_official_videos_are_songs() {
    // Captured 2026-10-17: every row of this album links an OMV video id.
    let a = parsers::parse_album(&fixture("album_omv"), "MPREb_K8qWMWVqXGi").unwrap();
    assert_eq!(a.title, "Random Access Memories");
    assert_eq!(a.tracks.len(), 13);
    assert_eq!(a.tracks[0].video_id, "IluRBvnYMoY");
    assert!(a.tracks.iter().all(|t| t.kind == TrackKind::Song));
}

#[test]
fn album_id_behind_olak_playlist() {
    let id = parsers::album_id_from_playlist_page(&fixture("album_olak_playlist"));
    assert_eq!(id.as_deref(), Some("MPREb_9CC1cAMRQuj"));
    // The header-less page still parses as a playlist named after the album.
    let p = parsers::parse_playlist(&fixture("album_olak_playlist"), "VLOLAK5uy_x").unwrap();
    assert_eq!(p.id, "OLAK5uy_x");
    assert_eq!(p.title, "Qora ko'zlaring");
    assert_eq!(p.tracks.len(), 13);
}

#[test]
fn artist_pages() {
    let a =
        parsers::parse_artist(&fixture("artist_skriptonit"), "UC7IxSgtALkMNWram-ZGQyaA").unwrap();
    assert_eq!(a.name, "Skryptonite");
    assert_eq!(a.subscribers.as_deref(), Some("622K subscribers"));
    assert_eq!(
        a.radio_playlist_id.as_deref(),
        Some("RDEMDG7BqAYjmBpYql-exAx6fw")
    );
    assert_eq!(
        a.shuffle_playlist_id.as_deref(),
        Some("RDAODG7BqAYjmBpYql-exAx6fw")
    );
    assert_eq!(
        a.top_songs_playlist_id.as_deref(),
        Some("OLAK5uy_l5GV8Ec9wGYxD3KizqlLj0QBbvE5b9GdM")
    );
    assert_eq!(a.top_songs.len(), 5);
    assert_eq!(a.top_songs[0].title, "Ламбада");
    assert_eq!(a.top_songs[0].artists.len(), 2);
    assert!(a.top_songs[0].album.is_some());
    assert!(!a.albums.is_empty() && !a.singles.is_empty());
    assert!(a.albums.iter().all(|x| x.browse_id.starts_with("MPRE")));
    assert!(a.singles.iter().any(|x| x.kind == AlbumKind::Single));
    assert!(a.videos.iter().all(|v| v.kind != TrackKind::Song));
    assert!(!a.playlists.is_empty());
    assert_eq!(a.related.len(), 10);
    assert!(a.related.iter().all(|r| r.id.starts_with("UC")));

    let s = parsers::parse_artist(&fixture("artist_shahzoda"), "UCo6BC71FZJiDPwMWdjfAzvg").unwrap();
    assert!(s.description.unwrap().starts_with("Zilola"));
    assert!(!s.related.is_empty());
}

#[test]
fn playlists_and_paging() {
    let p = parsers::parse_playlist(&fixture("playlist"), "VLPLG5iFiCeDoFtIJEMA7O6PCnRvJ738WDaH")
        .unwrap();
    assert_eq!(p.id, "PLG5iFiCeDoFtIJEMA7O6PCnRvJ738WDaH");
    assert_eq!(p.author.as_ref().unwrap().name, "No_Name");
    assert_eq!(
        p.author.as_ref().unwrap().id.as_deref(),
        Some("UCcgHCHR9WqoK3x9socJT5Ug")
    );
    assert_eq!(p.track_count, Some(43));
    assert_eq!(p.tracks.len(), 42);
    assert!(p.continuation.is_none());

    let big = parsers::parse_playlist(
        &fixture("playlist_big"),
        "PLOHoVaTp8R7c-vblRA0F_hOPCSMSwqu3z",
    )
    .unwrap();
    assert_eq!(big.track_count, Some(140));
    assert_eq!(big.tracks.len(), 100);
    assert!(big.continuation.is_some());
    let rest = parsers::parse_playlist_continuation(&fixture("playlist_cont"));
    assert_eq!(rest.tracks.len(), 40);
    assert!(rest.continuation.is_none());

    let ytm = parsers::parse_playlist(
        &fixture("playlist_ytm"),
        "RDCLAK5uy_kLaw49GbGpGx5yro0YPLicBbbqIHgK51w",
    )
    .unwrap();
    assert_eq!(ytm.title, "Presenting Баста");
    assert_eq!(ytm.author.unwrap().name, "YouTube Music");
    assert!(ytm.description.unwrap().starts_with("The most played"));
    assert_eq!(ytm.tracks.len(), 28);
}

#[test]
fn home_feed() {
    let (sections, cont) = parsers::parse_feed(&fixture("home"));
    assert!(cont.is_some());
    assert_eq!(sections[0].title, "Quick picks");
    assert_eq!(sections[0].items.len(), 20);
    assert!(sections[0].items.iter().all(|i| i.as_track().is_some()));
    assert_eq!(
        sections[1].strapline.as_deref(),
        Some("THE BIGGEST HITS OF ALL TIMES!")
    );

    let (more, _) = parsers::parse_feed(&fixture("home_cont"));
    assert_eq!(more.len(), 3);
    assert!(
        more.iter()
            .any(|s| s.items.iter().any(|i| i.as_album().is_some()))
    );
}

#[test]
fn moods_and_mood_page() {
    let cats = parsers::parse_moods(&fixture("moods"));
    assert_eq!(cats.len(), 2);
    assert_eq!(cats[0].title, "Moods & moments");
    assert_eq!(cats[0].moods[0].title, "Chill");
    assert_eq!(cats[0].moods[0].params, "ggMPOg1uX1JOQWZFeDByc2Jm");
    assert_eq!(cats[1].moods.len(), 24);

    let (sections, _) = parsers::parse_feed(&fixture("mood_chill"));
    assert_eq!(sections.len(), 4);
    let p = sections[0].items[0].as_playlist().unwrap();
    assert_eq!(p.id, "RDCLAK5uy_nBE4bLuBHUXWZrF59ZrkPEToKt8M_I3Vc");
    assert_eq!(p.title, "Coffee Shop Blend");
}

#[test]
fn related_tab() {
    let r = parsers::parse_related(&fixture("related"));
    assert_eq!(r.tracks.len(), 20);
    assert_eq!(r.artists.len(), 10);
    assert!(!r.playlists.is_empty());
    assert!(!r.albums.is_empty());
    assert!(r.artist_description.unwrap().starts_with("Zilola"));
}

#[test]
fn lyrics() {
    let l = parsers::parse_lyrics(&fixture("lyrics_skriptonit")).unwrap();
    assert!(l.text.starts_with("Я в такой же майке"));
    assert!(l.text.contains('\n'));
    assert_eq!(l.source.as_deref(), Some("Musixmatch"));
    assert!(parsers::parse_lyrics(&fixture("lyrics_unavailable")).is_none());
}

/// Every parser must survive every other response (and junk) without
/// panicking — that is what "renderers vary" looks like in practice.
#[test]
fn parsers_never_panic_on_foreign_input() {
    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut inputs: Vec<Value> = vec![
        Value::Null,
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!({"contents": 5}),
        serde_json::json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{}]}}}),
    ];
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "json") {
            inputs.push(serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());
        }
    }
    for v in &inputs {
        for f in [
            SearchFilter::All,
            SearchFilter::Songs,
            SearchFilter::Playlists,
        ] {
            let _ = parsers::parse_search(v, f);
            let _ = parsers::parse_search_continuation(v, f);
        }
        let _ = parsers::parse_suggestions(v);
        let _ = parsers::parse_watch(v);
        let _ = parsers::parse_watch_continuation(v);
        let _ = parsers::parse_album(v, "x");
        let _ = parsers::album_id_from_playlist_page(v);
        let _ = parsers::parse_artist(v, "x");
        let _ = parsers::parse_playlist(v, "x");
        let _ = parsers::parse_playlist_continuation(v);
        let _ = parsers::parse_feed(v);
        let _ = parsers::parse_moods(v);
        let _ = parsers::parse_related(v);
        let _ = parsers::parse_lyrics(v);
    }
}
