//! The import itself: every Yandex track looked up on YouTube Music (a few at a
//! time), then the matches saved to the local library: likes, playlists,
//! albums, artists. Running it again adds only what is new. Port of `runImport`
//! and `savePlaylists` in `ui/js/yandex-import.js`.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use volna_ytm::{AlbumSummary, ArtistSummary, Track};

use super::client::{self, Library};
use super::matching::{Catalog, match_album, match_artist, match_track};

/// Requests in flight while matching.
const PARALLEL: usize = 4;
/// Marks the playlists an import created, so the next one updates them.
pub const IMPORTED_FROM: &str = "yandex";

/// Where the matches go: the local library (`api/library.rs`). Calls are
/// short and synchronous (SQLite); every write must be idempotent as noted.
pub trait LibrarySink: Send + Sync {
    /// Adds a liked track unless it is liked already; `added_at_ms` (Unix ms)
    /// orders the likes. True when it was new.
    fn like_track(&self, track: &Track, added_at_ms: i64) -> anyhow::Result<bool>;
    /// Adds a liked album unless already there. True when it was new.
    fn like_album(&self, album: &AlbumSummary) -> anyhow::Result<bool>;
    /// Adds a liked artist unless already there. True when it was new.
    fn like_artist(&self, artist: &ArtistSummary) -> anyhow::Result<bool>;
    /// The user playlist an earlier import from `source` made under `title`.
    fn imported_playlist(&self, source: &str, title: &str) -> anyhow::Result<Option<String>>;
    /// A new empty user playlist marked as imported from `source`; its id.
    fn create_playlist(
        &self,
        title: &str,
        description: &str,
        source: &str,
    ) -> anyhow::Result<String>;
    /// Appends the tracks the playlist does not have yet, in order.
    fn add_to_playlist(&self, playlist: &str, tracks: &[Track]) -> anyhow::Result<()>;
    /// Video ids of the playlist, in order.
    fn playlist_track_ids(&self, playlist: &str) -> anyhow::Result<Vec<String>>;
    /// Puts the playlist's tracks in this order (a permutation of its ids).
    fn reorder_playlist(&self, playlist: &str, video_ids: &[String]) -> anyhow::Result<()>;
}

/// What to import; everything by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub likes: bool,
    pub playlists: bool,
    pub albums: bool,
    pub artists: bool,
    /// Try the browser's Yandex session when no token is held (desktop).
    pub browser: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            likes: true,
            playlists: true,
            albums: true,
            artists: true,
            browser: true,
        }
    }
}

impl Options {
    pub fn parts(&self) -> client::Parts {
        client::Parts {
            likes: self.likes,
            playlists: self.playlists,
            albums: self.albums,
            artists: self.artists,
        }
    }
}

/// `yandex_import_progress` payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    /// `browser`, `account`, `likes`, `playlists`, `albums` (reading Yandex),
    /// then `tracks`, `album_match`, `artist_match`, `saving`.
    pub stage: String,
    /// A playlist title while reading Yandex.
    pub detail: Option<String>,
    /// Lookups finished / to do (tracks + albums + artists); 0/0 before matching.
    pub done: usize,
    pub total: usize,
    /// Tracks found so far.
    pub matched: usize,
    /// "Artist — Title" (or an album / artist name) just looked up.
    pub current: Option<String>,
}

impl Progress {
    pub fn stage(stage: &str, detail: Option<String>) -> Self {
        Self {
            stage: stage.to_owned(),
            detail,
            ..Self::default()
        }
    }
}

/// What the import added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// New likes (already liked ones are not counted).
    pub liked_tracks: usize,
    /// Playlists created (updated ones are not counted).
    pub playlists: usize,
    pub albums: usize,
    pub artists: usize,
    /// Distinct Yandex tracks found on YouTube Music, of `total`.
    pub matched: usize,
    pub total: usize,
}

/// A Yandex track not found on YouTube Music.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Miss {
    pub title: String,
    pub artist: String,
}

/// Matching results, before anything is saved.
#[derive(Debug, Default)]
pub struct Matched {
    /// Yandex track id → YouTube Music track.
    pub tracks: HashMap<String, Track>,
    pub total: usize,
    pub misses: Vec<Miss>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
}

fn artist_line(t: &client::Track) -> String {
    t.artists.join(", ")
}

/// Each Yandex track once, however many playlists it is in; likes first.
pub fn unique_tracks(library: &Library) -> Vec<&client::Track> {
    let mut seen = HashSet::new();
    library
        .liked_tracks
        .iter()
        .map(|l| &l.track)
        .chain(library.playlists.iter().flat_map(|p| &p.tracks))
        .filter(|t| seen.insert(t.id.as_str()))
        .collect()
}

/// Runs `f` over `items` with [`PARALLEL`] in flight; results in input order.
/// `on_done(done, item)` after each.
async fn pool<'a, T: Sync, R: Send, F, Fut>(
    items: &'a [T],
    f: F,
    on_done: impl Fn(usize, &'a T) + Sync,
) -> Vec<Option<R>>
where
    F: Fn(&'a T) -> Fut + Sync,
    Fut: Future<Output = Option<R>>,
{
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let worker = || async {
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(i) else { return };
            let r = f(item).await;
            out.lock().unwrap_or_else(|e| e.into_inner())[i] = r;
            on_done(done.fetch_add(1, Ordering::Relaxed) + 1, item);
        }
    };
    const _: () = assert!(PARALLEL == 4);
    tokio::join!(worker(), worker(), worker(), worker());
    out.into_inner().unwrap_or_else(|e| e.into_inner())
}

/// Looks every track, album and artist of `library` up on YouTube Music.
pub async fn match_library<C: Catalog>(
    catalog: &C,
    library: &Library,
    progress: &(dyn Fn(Progress) + Sync),
) -> Matched {
    let unique = unique_tracks(library);
    let total = unique.len() + library.albums.len() + library.artists.len();
    let matched_count = AtomicUsize::new(0);
    let report = |stage: &str, done: usize, current: String| {
        progress(Progress {
            stage: stage.to_owned(),
            detail: None,
            done,
            total,
            matched: matched_count.load(Ordering::Relaxed),
            current: Some(current),
        })
    };
    progress(Progress {
        stage: "tracks".into(),
        total,
        ..Progress::default()
    });

    let found = pool(
        &unique,
        |t| async {
            let m = match_track(catalog, t).await;
            if m.is_some() {
                matched_count.fetch_add(1, Ordering::Relaxed);
            }
            m
        },
        |done, t| report("tracks", done, format!("{} — {}", artist_line(t), t.full_title())),
    )
    .await;
    let mut out = Matched {
        total: unique.len(),
        ..Matched::default()
    };
    for (t, m) in unique.iter().zip(found) {
        match m {
            Some(m) => {
                out.tracks.insert(t.id.clone(), m);
            }
            None => out.misses.push(Miss {
                title: t.full_title(),
                artist: artist_line(t),
            }),
        }
    }

    let base = unique.len();
    out.albums = pool(
        &library.albums,
        |a| match_album(catalog, a),
        |done, a| report("album_match", base + done, a.title.clone()),
    )
    .await
    .into_iter()
    .flatten()
    .collect();
    let base = base + library.albums.len();
    out.artists = pool(
        &library.artists,
        |name| match_artist(catalog, name),
        |done, name| report("artist_match", base + done, name.clone()),
    )
    .await
    .into_iter()
    .flatten()
    .collect();
    out
}

/// Saves the matches; the counts of what was new.
pub fn save(
    sink: &dyn LibrarySink,
    library: &Library,
    matched: &Matched,
) -> anyhow::Result<Summary> {
    let mut summary = Summary {
        matched: matched.tracks.len(),
        total: matched.total,
        ..Summary::default()
    };
    // Yandex lists likes newest first; the time keeps that order here.
    let now = now_ms();
    for (i, liked) in library.liked_tracks.iter().enumerate() {
        let Some(track) = matched.tracks.get(&liked.track.id) else {
            continue;
        };
        let at = liked
            .liked_at
            .as_deref()
            .and_then(parse_iso_ms)
            .unwrap_or(now - i as i64);
        if sink.like_track(track, at)? {
            summary.liked_tracks += 1;
        }
    }
    for album in &matched.albums {
        if sink.like_album(album)? {
            summary.albums += 1;
        }
    }
    for artist in &matched.artists {
        if sink.like_artist(artist)? {
            summary.artists += 1;
        }
    }
    summary.playlists = save_playlists(sink, library, &matched.tracks)?;
    Ok(summary)
}

/// Playlists with at least one match; one already imported under the same
/// title gets the new tracks. Returns how many were created.
fn save_playlists(
    sink: &dyn LibrarySink,
    library: &Library,
    matched: &HashMap<String, Track>,
) -> anyhow::Result<usize> {
    let mut created = 0;
    for p in &library.playlists {
        let tracks: Vec<Track> = p
            .tracks
            .iter()
            .filter_map(|t| matched.get(&t.id).cloned())
            .collect();
        if tracks.is_empty() {
            continue;
        }
        if let Some(id) = sink.imported_playlist(IMPORTED_FROM, &p.title)? {
            sink.add_to_playlist(&id, &tracks)?;
            let current = sink.playlist_track_ids(&id)?;
            sink.reorder_playlist(&id, &imported_order(&tracks, current))?;
            continue;
        }
        let id = sink.create_playlist(
            &p.title,
            p.description.as_deref().unwrap_or(""),
            IMPORTED_FROM,
        )?;
        sink.add_to_playlist(&id, &tracks)?;
        created += 1;
    }
    Ok(created)
}

/// Tracks found this time go where Yandex has them, not at the end; tracks
/// added by hand in Volna stay after the imported ones, in their order.
fn imported_order(imported: &[Track], current: Vec<String>) -> Vec<String> {
    let mut rank: HashMap<&str, usize> = HashMap::new();
    for (i, t) in imported.iter().enumerate() {
        rank.entry(t.video_id.as_str()).or_insert(i);
    }
    let mut keyed: Vec<(usize, usize, String)> = current
        .into_iter()
        .enumerate()
        .map(|(i, id)| (rank.get(id.as_str()).copied().unwrap_or(imported.len()), i, id))
        .collect();
    keyed.sort_by_key(|&(r, i, _)| (r, i));
    keyed.into_iter().map(|(_, _, id)| id).collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `2024-01-01T12:34:56(.789)?(Z|±hh:mm)` → Unix ms (what `Date.parse` read).
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| -> Option<i64> {
        let part = s.get(a..b)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5)? != "-" || s.get(7..8)? != "-" || !matches!(s.get(10..11)?, "T" | " ") {
        return None;
    }
    let (h, mi, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    let mut rest = s.get(19..)?;
    let mut ms = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        let padded = format!("{:0<3}", &frac[..digits.min(3)]);
        ms = padded.parse().ok()?;
        rest = &frac[digits..];
    }
    let offset_min = match rest {
        "" | "Z" => 0,
        _ => {
            let sign = match rest.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let digits: String = rest[1..].chars().filter(char::is_ascii_digit).collect();
            if digits.len() != 4 {
                return None;
            }
            sign * (digits[..2].parse::<i64>().ok()? * 60 + digits[2..].parse::<i64>().ok()?)
        }
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1000 + ms)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::api::yandex::client::{LikedTrack, Playlist};
    use crate::api::yandex::matching::tests::{FakeCatalog, track, ytrack};
    use volna_ytm::{Item, SearchFilter};

    /// The library in memory, as `api/library.rs` will behave.
    #[derive(Default)]
    pub struct MemorySink {
        pub likes: Mutex<Vec<(String, i64)>>,
        pub albums: Mutex<Vec<String>>,
        pub artists: Mutex<Vec<String>>,
        /// (id, title, source, video ids)
        pub playlists: Mutex<Vec<(String, String, String, Vec<String>)>>,
    }

    impl LibrarySink for MemorySink {
        fn like_track(&self, t: &Track, at: i64) -> anyhow::Result<bool> {
            let mut l = self.likes.lock().unwrap();
            if l.iter().any(|(id, _)| *id == t.video_id) {
                return Ok(false);
            }
            l.push((t.video_id.clone(), at));
            Ok(true)
        }
        fn like_album(&self, a: &AlbumSummary) -> anyhow::Result<bool> {
            let mut l = self.albums.lock().unwrap();
            Ok(!l.contains(&a.browse_id) && {
                l.push(a.browse_id.clone());
                true
            })
        }
        fn like_artist(&self, a: &ArtistSummary) -> anyhow::Result<bool> {
            let mut l = self.artists.lock().unwrap();
            Ok(!l.contains(&a.id) && {
                l.push(a.id.clone());
                true
            })
        }
        fn imported_playlist(&self, source: &str, title: &str) -> anyhow::Result<Option<String>> {
            Ok(self
                .playlists
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.1 == title && p.2 == source)
                .map(|p| p.0.clone()))
        }
        fn create_playlist(&self, title: &str, _: &str, source: &str) -> anyhow::Result<String> {
            let mut l = self.playlists.lock().unwrap();
            let id = format!("p{}", l.len());
            l.push((id.clone(), title.into(), source.into(), vec![]));
            Ok(id)
        }
        fn add_to_playlist(&self, id: &str, tracks: &[Track]) -> anyhow::Result<()> {
            let mut l = self.playlists.lock().unwrap();
            let p = l.iter_mut().find(|p| p.0 == id).unwrap();
            for t in tracks {
                if !p.3.contains(&t.video_id) {
                    p.3.push(t.video_id.clone());
                }
            }
            Ok(())
        }
        fn playlist_track_ids(&self, id: &str) -> anyhow::Result<Vec<String>> {
            let l = self.playlists.lock().unwrap();
            Ok(l.iter().find(|p| p.0 == id).unwrap().3.clone())
        }
        fn reorder_playlist(&self, id: &str, ids: &[String]) -> anyhow::Result<()> {
            let mut l = self.playlists.lock().unwrap();
            l.iter_mut().find(|p| p.0 == id).unwrap().3 = ids.to_vec();
            Ok(())
        }
    }

    #[test]
    fn parses_yandex_timestamps() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2024-01-01T00:00:00+00:00"),
            Some(1_704_067_200_000)
        );
        assert_eq!(
            parse_iso_ms("2024-01-01T03:00:00.5+03:00"),
            Some(1_704_067_200_500)
        );
        assert_eq!(parse_iso_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
        assert_eq!(parse_iso_ms("yesterday"), None);
        assert_eq!(parse_iso_ms(""), None);
    }

    #[test]
    fn imported_tracks_lead_hand_added_ones_follow() {
        let imported = [track("b", "", &[], None), track("a", "", &[], None)];
        let current = ["x", "a", "y", "b"].map(String::from).to_vec();
        assert_eq!(imported_order(&imported, current), ["b", "a", "x", "y"]);
    }

    fn library() -> Library {
        let liked = |t: client::Track, at: Option<&str>| LikedTrack {
            track: t,
            liked_at: at.map(Into::into),
        };
        Library {
            login: None,
            liked_tracks: vec![
                liked(ytrack("One", None, &["A"], None), Some("2024-01-02T00:00:00Z")),
                liked(ytrack("Two", None, &["A"], None), None),
                liked(ytrack("Gone", None, &["A"], None), None),
            ],
            playlists: vec![Playlist {
                title: "Mix".into(),
                description: None,
                own: true,
                tracks: vec![
                    ytrack("Two", None, &["A"], None),
                    ytrack("Three", None, &["B"], None),
                ],
            }],
            albums: vec![],
            artists: vec![],
        }
    }

    fn catalog() -> FakeCatalog {
        let hit = |id: &str, title: &str, artist: &str| vec![Item::Track(track(id, title, &[artist], None))];
        FakeCatalog::default()
            .with(SearchFilter::Songs, "A One", hit("v1", "One", "A"))
            .with(SearchFilter::Songs, "A Two", hit("v2", "Two", "A"))
            .with(SearchFilter::Songs, "B Three", hit("v3", "Three", "B"))
    }

    #[tokio::test]
    async fn matches_and_saves_once() {
        let lib = library();
        let events = Mutex::new(Vec::new());
        let matched = match_library(&catalog(), &lib, &|p| events.lock().unwrap().push(p)).await;
        assert_eq!(matched.total, 4);
        assert_eq!(matched.tracks.len(), 3);
        assert_eq!(
            matched.misses,
            [Miss {
                title: "Gone".into(),
                artist: "A".into()
            }]
        );
        let events = events.into_inner().unwrap();
        let last = events.last().unwrap();
        assert_eq!((last.done, last.total, last.matched), (4, 4, 3));

        let sink = MemorySink::default();
        let summary = save(&sink, &lib, &matched).unwrap();
        assert_eq!(
            summary,
            Summary {
                liked_tracks: 2,
                playlists: 1,
                albums: 0,
                artists: 0,
                matched: 3,
                total: 4
            }
        );
        let likes = sink.likes.lock().unwrap().clone();
        assert_eq!(likes[0], ("v1".into(), 1_704_153_600_000));
        assert_eq!(likes[1].0, "v2");
        assert_eq!(sink.playlists.lock().unwrap()[0].3, ["v2", "v3"]);

        // Again: nothing new, the playlist is updated in place.
        let again = save(&sink, &lib, &matched).unwrap();
        assert_eq!((again.liked_tracks, again.playlists), (0, 0));
        assert_eq!(sink.playlists.lock().unwrap().len(), 1);
    }
}
