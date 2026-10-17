//! A Yandex track → the same song on YouTube Music.
//!
//! A faithful port of `ui/js/yandex-import.js` (`matchTrack`) and the helpers
//! it took from `ui/js/playlist-importer.js` (`toLatin`, `stringSimilarity`,
//! `findBestMatch`): the same queries, spelling folds, scores and thresholds.

use std::future::Future;

use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup, Script};
use volna_ytm::{AlbumSummary, ArtistSummary, Item, SearchFilter, Track, YtmClient};

use super::client;

/// Candidates further off than this in length are another recording.
const DURATION_SLACK_S: f64 = 15.0;
/// Length tolerance when a non-Latin, non-Cyrillic title is matched by length.
const EXACT_LENGTH_S: f64 = 2.0;
/// Stricter than the CSV import: Yandex gives exact titles, and a loose match
/// puts a stranger's song into the likes.
const MIN_TITLE_SCORE: f64 = 0.7;
const MIN_ARTIST_SCORE: f64 = 0.7;
const MIN_SCORE: f64 = 0.55;
pub const MIN_ALBUM_SCORE: f64 = 0.6;
pub const MIN_ARTIST_NAME_SCORE: f64 = 0.85;

/// The YouTube Music search, as the matcher needs it. A failed search counts
/// as no results, as in the old UI.
pub trait Catalog: Send + Sync {
    fn find(&self, query: &str, filter: SearchFilter) -> impl Future<Output = Vec<Item>> + Send;
}

impl Catalog for YtmClient {
    async fn find(&self, query: &str, filter: SearchFilter) -> Vec<Item> {
        match YtmClient::search(self, query, filter).await {
            Ok(page) => page.items,
            Err(e) => {
                tracing::warn!("yandex import: search {filter:?} {query:?}: {e}");
                Vec::new()
            }
        }
    }
}

// ---- spelling ----------------------------------------------------------------

/// Cyrillic (Russian and Uzbek) to Latin. YouTube Music often lists Russian and
/// Uzbek releases transliterated: «Вотсап» is "Votsap", «Ахрор» is "Axror".
fn cyrillic_to_latin(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "e",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "j",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "h",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "sh",
        'ъ' => "",
        'ы' => "y",
        'ь' => "",
        'э' => "e",
        'ю' => "ju",
        'я' => "ja",
        'ў' => "o",
        'қ' => "q",
        'ғ' => "g",
        'ҳ' => "h",
        'і' => "i",
        'ї' => "ji",
        'є' => "je",
        _ => return None,
    })
}

/// `toLatin`: Cyrillic letters (U+0400–U+04FF) spelled in Latin, keeping case.
pub fn to_latin(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if !('\u{0400}'..='\u{04ff}').contains(&ch) {
            out.push(ch);
            continue;
        }
        let lower = single_lower(ch);
        match cyrillic_to_latin(lower) {
            None => out.push(ch),
            Some(latin) if ch == lower => out.push_str(latin),
            Some(latin) => {
                let mut cs = latin.chars();
                if let Some(first) = cs.next() {
                    out.extend(first.to_uppercase());
                    out.push_str(cs.as_str());
                }
            }
        }
    }
    out
}

/// The lowercase of one character when it is one character (all of U+0400–U+04FF).
fn single_lower(ch: char) -> char {
    let mut l = ch.to_lowercase();
    match (l.next(), l.next()) {
        (Some(c), None) => c,
        _ => ch,
    }
}

/// Replaces literal alternatives left to right, the first that fits winning,
/// as a global JS regex `a|b` does.
fn replace_alts(s: &str, alts: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    'outer: while let Some(ch) = rest.chars().next() {
        for (from, to) in alts {
            if let Some(after) = rest.strip_prefix(from) {
                out.push_str(to);
                rest = after;
                continue 'outer;
            }
        }
        out.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    out
}

fn general_category(c: char) -> GeneralCategory {
    CodePointMapData::<GeneralCategory>::new().get(c)
}

/// `\p{L}` or `\p{N}`.
fn is_letter_or_number(c: char) -> bool {
    let gc = general_category(c);
    GeneralCategoryGroup::Letter.contains(gc) || GeneralCategoryGroup::Number.contains(gc)
}

/// `foldForMatch`: one spelling for the usual transliteration variants
/// (kh/x/h, y/j, zh/j, diacritics), so «Алихонов», "Alixonov" and "Alikhonov"
/// compare equal.
pub fn fold_for_match(s: &str) -> String {
    let latin = to_latin(&s.to_lowercase());
    let nfd = DecomposingNormalizerBorrowed::new_nfd().normalize(&latin);
    let stripped: String = nfd
        .chars()
        .filter(|&c| !GeneralCategoryGroup::Mark.contains(general_category(c)))
        .collect();
    let s = replace_alts(&stripped, &[("shch", "sh"), ("sch", "sh")]);
    let s = replace_alts(&s, &[("kh", "h"), ("x", "h")]);
    replace_alts(&s, &[("zh", "j"), ("y", "j")])
}

fn normalize_for_match(s: &str) -> String {
    fold_for_match(s)
        .chars()
        .filter(|&c| is_letter_or_number(c))
        .collect()
}

fn tokenize(s: &str) -> Vec<String> {
    fold_for_match(s)
        .split(|c: char| !is_letter_or_number(c))
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `stringSimilarity`: 1 for the same folded spelling, 0.85 when one contains
/// the other, else the larger of the shared-word share and the edit similarity.
pub fn string_similarity(a: &str, b: &str) -> f64 {
    let na = normalize_for_match(a);
    let nb = normalize_for_match(b);
    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }
    if na == nb {
        return 1.0;
    }
    if na.contains(&nb) || nb.contains(&na) {
        return 0.85;
    }
    let mut ta = tokenize(a);
    let mut tb = tokenize(b);
    if ta.is_empty() || tb.is_empty() {
        return 0.0;
    }
    ta.sort();
    ta.dedup();
    tb.sort();
    tb.dedup();
    let common = ta.iter().filter(|t| tb.binary_search(t).is_ok()).count();
    let words = common as f64 / ta.len().max(tb.len()) as f64;
    words.max(edit_similarity(&na, &nb))
}

/// 1 - Levenshtein distance / length, for spellings a letter or two apart
/// ("berezka" and «берёзка»). Short strings differ too easily to count.
fn edit_similarity(a: &str, b: &str) -> f64 {
    // JS counts UTF-16 units.
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    if a.len().min(b.len()) < 6 || a.len().max(b.len()) > 80 {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(sub);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()) as f64
}

/// `splitArtists`: on `,`, `&`, ` and `, ` feat. `, ` feat `, ` featuring `
/// (any case).
fn split_artists(s: &str) -> Vec<String> {
    const SEPS: [&str; 6] = [",", "&", " and ", " feat. ", " feat ", " featuring "];
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < bytes.len() {
        let sep = SEPS.iter().find(|sep| {
            bytes
                .get(i..i + sep.len())
                .is_some_and(|w| w.eq_ignore_ascii_case(sep.as_bytes()))
        });
        match sep {
            Some(sep) => {
                parts.push(&s[start..i]);
                i += sep.len();
                start = i;
            }
            None => i += 1,
        }
    }
    parts.push(&s[start..]);
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
        .collect()
}

// ---- scoring -------------------------------------------------------------------

/// A search result as scored: a track, or one reading of a video's title.
#[derive(Debug, Clone)]
struct Candidate<'a> {
    title: String,
    artists: Vec<String>,
    album: Option<&'a str>,
    source: &'a Track,
}

impl<'a> Candidate<'a> {
    fn of(t: &'a Track) -> Self {
        Self {
            title: t.title.clone(),
            artists: t
                .artists
                .iter()
                .map(|a| a.name.clone())
                .filter(|n| !n.is_empty())
                .collect(),
            album: t.album.as_ref().map(|a| a.name.as_str()),
            source: t,
        }
    }
}

/// Length in seconds when known (0 is unknown, as in the old API shape).
fn duration_s(t: &Track) -> Option<f64> {
    t.duration_s.filter(|&d| d > 0).map(f64::from)
}

struct Scores {
    score: f64,
    title: f64,
    artist: f64,
}

fn score_track(c: &Candidate, title: &str, artist: &str, album: Option<&str>) -> Scores {
    let title_score = string_similarity(&c.title, title);
    let mut artist_score: f64 = 0.0;
    for ta in split_artists(artist) {
        for ia in &c.artists {
            artist_score = artist_score.max(string_similarity(ia, &ta));
        }
    }
    if !c.artists.is_empty() {
        artist_score = artist_score.max(string_similarity(&c.artists.join(" "), artist));
    }
    let album_score = match album.filter(|a| !a.is_empty()) {
        Some(a) => string_similarity(c.album.unwrap_or(""), a),
        None => 0.0,
    };
    Scores {
        score: title_score * 0.5 + artist_score * 0.4 + album_score * 0.1,
        title: title_score,
        artist: artist_score,
    }
}

/// `findBestMatch` with the strict import options.
fn find_best_match<'a>(
    items: &[Candidate<'a>],
    artist: &str,
    album: Option<&str>,
    title: &str,
) -> Option<&'a Track> {
    if title.is_empty() {
        return None;
    }
    let mut best = None;
    let mut best_score = -1.0;
    for item in items {
        let s = score_track(item, title, artist, album);
        if s.title < MIN_TITLE_SCORE {
            continue;
        }
        if !artist.is_empty() && s.artist < MIN_ARTIST_SCORE {
            continue;
        }
        if s.score > best_score {
            best_score = s.score;
            best = Some(item.source);
        }
    }
    best.filter(|_| best_score >= MIN_SCORE)
}

// ---- videos --------------------------------------------------------------------

const NOISE_WORDS: [&str; 9] = [
    "official",
    "music video",
    "audio",
    "lyric",
    "mood video",
    "hd",
    "mv",
    "клип",
    "премьера",
];

/// Case-insensitive prefix: the length in bytes of `s` that matched `word`.
fn starts_with_ci(s: &str, word: &str) -> Option<usize> {
    let mut len = 0;
    let mut chars = s.chars();
    for w in word.chars() {
        let c = chars.next()?;
        if single_lower(c) != w {
            return None;
        }
        len += c.len_utf8();
    }
    Some(len)
}

/// The end of `VIDEO_NOISE` matched at `i`, if any:
/// `\s*[([](official|music video|...)[^)\]]*[)\]]` or `\s+\|.*$`.
fn noise_at(s: &str, i: usize) -> Option<usize> {
    let rest = &s[i..];
    let trimmed = rest.trim_start_matches(char::is_whitespace);
    let ws = rest.len() - trimmed.len();
    if let Some(inner) = trimmed.strip_prefix(['(', '['])
        && let Some(word) = NOISE_WORDS.iter().find_map(|w| starts_with_ci(inner, w))
        && let Some(close) = inner[word..].find([')', ']'])
    {
        return Some(s.len() - inner.len() + word + close + 1);
    }
    (ws > 0 && trimmed.starts_with('|')).then_some(s.len())
}

/// "(Official Music Video)", "[HD]", "| Music video" and the like removed.
fn strip_video_noise(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut i = 0;
    while i < title.len() {
        if let Some(end) = noise_at(title, i) {
            i = end;
            continue;
        }
        let ch = title[i..].chars().next().expect("in bounds");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// `^(.+?)\s+[-–—]\s+(.+)$`: "Artist - Song" split at the first dash.
fn split_dash(title: &str) -> Option<(&str, &str)> {
    let chars: Vec<(usize, char)> = title.char_indices().collect();
    for e in 1..chars.len() {
        if !chars[e].1.is_whitespace() {
            continue;
        }
        let mut w = e;
        while w < chars.len() && chars[w].1.is_whitespace() {
            w += 1;
        }
        if w >= chars.len() || !matches!(chars[w].1, '-' | '–' | '—') {
            continue;
        }
        let mut r = w + 1;
        if r >= chars.len() || !chars[r].1.is_whitespace() {
            continue;
        }
        while r < chars.len() && chars[r].1.is_whitespace() {
            r += 1;
        }
        if r < chars.len() {
            return Some((&title[..chars[e].0], &title[chars[r].0..]));
        }
    }
    None
}

/// A video as the song it carries. Uploads by other channels put the song into
/// the title ("DVN - Можно я с тобой"), sometimes the other way round, so both
/// readings are offered; each candidate points back at the video.
fn video_candidates(video: &Track) -> Vec<Candidate<'_>> {
    let base = Candidate::of(video);
    let title = strip_video_noise(&video.title).trim().to_owned();
    let mut out = vec![Candidate {
        title: title.clone(),
        ..base.clone()
    }];
    if let Some((left, right)) = split_dash(&title) {
        let with = |name: &str| {
            let mut a = vec![name.to_owned()];
            a.extend(base.artists.iter().cloned());
            a
        };
        out.push(Candidate {
            title: right.to_owned(),
            artists: with(left),
            ..base.clone()
        });
        out.push(Candidate {
            title: left.to_owned(),
            artists: with(right),
            ..base.clone()
        });
    }
    out
}

// ---- matching ------------------------------------------------------------------

fn is_latin_or_cyrillic(c: char) -> bool {
    let script = CodePointMapData::<Script>::new().get(c);
    script == Script::Latin || script == Script::Cyrillic
}

/// Yandex sometimes has a Japanese release title («フェイデッド» for "Faded"),
/// which no spelling rule maps back. For such titles the same artist and a
/// near-identical length are taken as the song.
fn same_artist_and_length<'a>(items: &[&'a Track], t: &client::Track) -> Option<&'a Track> {
    let ms = t.duration_ms.filter(|&d| d > 0)?;
    if t.title.is_empty() || t.title.chars().any(is_latin_or_cyrillic) {
        return None;
    }
    let length = ms as f64 / 1000.0;
    items.iter().copied().find(|item| {
        duration_s(item).is_some_and(|d| (d - length).abs() <= EXACT_LENGTH_S)
            && item.artists.iter().any(|a| {
                t.artists
                    .iter()
                    .any(|name| string_similarity(&a.name, name) >= 0.9)
            })
    })
}

/// The YouTube Music track for a Yandex track, or `None`.
pub async fn match_track<C: Catalog>(catalog: &C, t: &client::Track) -> Option<Track> {
    let artist = t.artists.join(", ");
    let length = t
        .duration_ms
        .filter(|&d| d > 0)
        .map(|ms| ms as f64 / 1000.0);
    let duration_ok = |item: &Track| match (length, duration_s(item)) {
        (Some(l), Some(d)) => (d - l).abs() <= DURATION_SLACK_S,
        _ => true,
    };
    let first = t.artists.first().map(String::as_str).unwrap_or("");
    let plain = format!("{first} {}", t.title).trim().to_owned();
    // YouTube Music may only know the release under its Latin spelling.
    let latin = to_latin(&plain);
    let mut queries = vec![plain.clone()];
    if latin != plain {
        queries.push(latin);
    }
    let full = t.full_title();
    let titles: Vec<&str> = if t.version.is_some() {
        vec![&full, &t.title]
    } else {
        vec![&t.title]
    };
    let mut song_queries = Vec::new();
    if t.version.is_some() {
        song_queries.push(format!("{first} {full}").trim().to_owned());
    }
    song_queries.extend(queries.iter().cloned());

    for query in &song_queries {
        let found = catalog.find(query, SearchFilter::Songs).await;
        let items: Vec<&Track> = found
            .iter()
            .filter_map(Item::as_track)
            .filter(|i| duration_ok(i))
            .collect();
        let candidates: Vec<Candidate> = items.iter().map(|i| Candidate::of(i)).collect();
        for title in &titles {
            if let Some(m) = find_best_match(&candidates, &artist, t.album.as_deref(), title) {
                return Some(m.clone());
            }
        }
        if let Some(m) = same_artist_and_length(&items, t) {
            return Some(m.clone());
        }
    }
    // Songs that exist on YouTube only as videos. Uploads differ in length
    // (intros, compilations), so only close lengths count here.
    for query in &queries {
        let found = catalog.find(query, SearchFilter::Videos).await;
        let candidates: Vec<Candidate> = found
            .iter()
            .filter_map(Item::as_track)
            .filter(|i| duration_ok(i))
            .flat_map(video_candidates)
            .collect();
        for title in &titles {
            if let Some(m) = find_best_match(&candidates, &artist, None, title) {
                return Some(m.clone());
            }
        }
    }
    None
}

/// `bestByName`: the result named most like `name`, at least `min_score`;
/// on a tie the later one wins.
fn best_by_name<'a, T>(
    items: impl IntoIterator<Item = &'a T>,
    name_of: impl Fn(&T) -> &str,
    name: &str,
    min_score: f64,
) -> Option<&'a T> {
    let mut best = None;
    let mut best_score = min_score;
    for item in items {
        let score = string_similarity(name_of(item), name);
        if score >= best_score {
            best = Some(item);
            best_score = score;
        }
    }
    best
}

/// The YouTube Music album for a liked Yandex album.
pub async fn match_album<C: Catalog>(catalog: &C, a: &client::Album) -> Option<AlbumSummary> {
    let first = a.artists.first().map(String::as_str).unwrap_or("");
    let found = catalog
        .find(&format!("{first} {}", a.title), SearchFilter::Albums)
        .await;
    best_by_name(
        found.iter().filter_map(Item::as_album),
        |x| &x.title,
        &a.title,
        MIN_ALBUM_SCORE,
    )
    .cloned()
}

/// The YouTube Music artist for a liked Yandex artist.
pub async fn match_artist<C: Catalog>(catalog: &C, name: &str) -> Option<ArtistSummary> {
    let found = catalog.find(name, SearchFilter::Artists).await;
    best_by_name(
        found.iter().filter_map(Item::as_artist),
        |x| &x.name,
        name,
        MIN_ARTIST_NAME_SCORE,
    )
    .cloned()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use volna_ytm::{AlbumKind, ArtistRef, TrackKind};

    pub fn track(id: &str, title: &str, artists: &[&str], dur: Option<u32>) -> Track {
        Track {
            video_id: id.into(),
            title: title.into(),
            artists: artists
                .iter()
                .map(|a| ArtistRef {
                    id: None,
                    name: (*a).into(),
                })
                .collect(),
            album: None,
            duration_s: dur,
            thumbnails: vec![],
            explicit: false,
            kind: TrackKind::Song,
            year: None,
            views: None,
            playable: true,
        }
    }

    pub fn ytrack(
        title: &str,
        version: Option<&str>,
        artists: &[&str],
        ms: Option<u64>,
    ) -> client::Track {
        client::Track {
            id: format!("y-{title}"),
            title: title.into(),
            version: version.map(Into::into),
            artists: artists.iter().map(|a| (*a).into()).collect(),
            album: None,
            duration_ms: ms,
        }
    }

    /// Canned results per (filter, query); records every query asked.
    #[derive(Default)]
    pub struct FakeCatalog {
        pub results: Vec<(SearchFilter, String, Vec<Item>)>,
        pub asked: std::sync::Mutex<Vec<(SearchFilter, String)>>,
    }

    impl FakeCatalog {
        pub fn with(mut self, filter: SearchFilter, query: &str, items: Vec<Item>) -> Self {
            self.results.push((filter, query.into(), items));
            self
        }
        fn asked(&self) -> Vec<(SearchFilter, String)> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Catalog for FakeCatalog {
        async fn find(&self, query: &str, filter: SearchFilter) -> Vec<Item> {
            self.asked.lock().unwrap().push((filter, query.into()));
            self.results
                .iter()
                .find(|(f, q, _)| *f == filter && q == query)
                .map(|(_, _, items)| items.clone())
                .unwrap_or_default()
        }
    }

    /// YouTube Music through `VOLNA_TEST_PROXY` (e.g. `socks5h://127.0.0.1:1081`), or direct.
    pub fn live_ytm() -> YtmClient {
        let mut http = reqwest::Client::builder();
        if let Ok(proxy) = std::env::var("VOLNA_TEST_PROXY") {
            http = http.proxy(reqwest::Proxy::all(proxy).expect("VOLNA_TEST_PROXY"));
        }
        YtmClient::new(http.build().unwrap())
    }

    /// Real searches for a few known songs:
    /// `VOLNA_TEST_PROXY=socks5h://127.0.0.1:1081 cargo test -p volna-core yandex_live_match -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn yandex_live_match() {
        let ytm = live_ytm();
        let cases = [
            ytrack("Привычка", None, &["Скриптонит"], Some(201_000)),
            ytrack("Faded", None, &["Alan Walker"], Some(212_000)),
            ytrack("Группа крови", None, &["КИНО"], Some(286_000)),
            ytrack("Shape of You", None, &["Ed Sheeran"], Some(233_000)),
            ytrack(
                "Despacito",
                Some("Remix"),
                &["Luis Fonsi", "Daddy Yankee", "Justin Bieber"],
                Some(228_000),
            ),
        ];
        let mut found = 0;
        for t in &cases {
            let m = match_track(&ytm, t).await;
            println!(
                "{} — {} → {:?}",
                t.artists.join(", "),
                t.full_title(),
                m.as_ref().map(|m| (
                    &m.video_id,
                    &m.title,
                    m.artists
                        .iter()
                        .map(|a| a.name.as_str())
                        .collect::<Vec<_>>()
                ))
            );
            found += usize::from(m.is_some());
        }
        assert!(found >= cases.len() - 1, "found {found} of {}", cases.len());
        let album = match_album(
            &ytm,
            &client::Album {
                title: "Дом с нормальными явлениями".into(),
                artists: vec!["Скриптонит".into()],
                year: None,
            },
        )
        .await;
        println!(
            "album → {:?}",
            album.as_ref().map(|a| (&a.browse_id, &a.title))
        );
        let artist = match_artist(&ytm, "Скриптонит").await;
        println!("artist → {:?}", artist.as_ref().map(|a| (&a.id, &a.name)));
        assert!(album.is_some() && artist.is_some());
    }

    #[test]
    fn to_latin_keeps_case_and_other_letters() {
        assert_eq!(to_latin("Вотсап"), "Votsap");
        assert_eq!(to_latin("Ахрор"), "Ahror");
        assert_eq!(to_latin("ЖЁЛТЫЙ"), "ZhELTYJ");
        assert_eq!(to_latin("Щука"), "Shuka");
        assert_eq!(to_latin("Ўзбек қўшиқ ғ ҳ"), "Ozbek qoshiq g h");
        assert_eq!(to_latin("Їжак єнот і ъ ь"), "Jizhak jenot i  ");
        assert_eq!(to_latin("Mix: Ахрор & Co"), "Mix: Ahror & Co");
        assert_eq!(to_latin("フェイデッド"), "フェイデッド");
    }

    #[test]
    fn folds_transliteration_variants() {
        assert_eq!(fold_for_match("Алихонов"), "alihonov");
        assert_eq!(fold_for_match("Alixonov"), "alihonov");
        assert_eq!(fold_for_match("Alikhonov"), "alihonov");
        assert_eq!(fold_for_match("Beyoncé"), "bejonce");
        assert_eq!(fold_for_match("Shchedrin"), "shedrin");
        assert_eq!(fold_for_match("Zhanna"), "janna");
        assert_eq!(fold_for_match("Щедрин"), "shedrin");
    }

    #[test]
    fn similarity_like_the_js() {
        assert_eq!(string_similarity("Алихонов", "Alikhonov"), 1.0);
        assert_eq!(string_similarity("Hello", "Hello World"), 0.85);
        assert_eq!(string_similarity("", "x"), 0.0);
        assert_eq!(string_similarity("!!!", "x"), 0.0);
        // Two of three words (0.67), but "redbluecat"/"redbluedog" are 3 edits
        // of 10 apart: the larger, 0.7, wins.
        let s = string_similarity("red blue cat", "red blue dog");
        assert!((s - 0.7).abs() < 1e-9, "{s}");
        // Words only: "a b" vs "a c" are too short for the edit score.
        let s = string_similarity("a b", "a c");
        assert!((s - 0.5).abs() < 1e-9, "{s}");
        // A letter apart: the edit score wins.
        let s = string_similarity("berezka", "берёзки");
        assert!((s - 6.0 / 7.0).abs() < 1e-9, "{s}");
        // Under 6 letters the edit score is 0.
        assert_eq!(edit_similarity("abcde", "abcdf"), 0.0);
    }

    /// Values printed by `stringSimilarity` in `ui/js/playlist-importer.js` (bun).
    #[test]
    fn similarity_table_from_the_js() {
        let table = [
            ("Алихонов", "Alikhonov", 1.0),
            ("Hello", "Hello World", 0.85),
            ("red blue cat", "red blue dog", 0.7),
            ("a b", "a c", 0.5),
            ("berezka", "берёзки", 0.8571428571428572),
            ("Beyoncé", "Bejonce", 1.0),
            ("Skrip", "Скриптонит", 0.85),
            ("Скриптонит", "Skriptonit", 1.0),
            ("Группа крови", "Gruppa krovi", 1.0),
            ("Ёлка", "Yolka", 0.0),
        ];
        for (a, b, want) in table {
            let got = string_similarity(a, b);
            assert!((got - want).abs() < 1e-12, "{a} | {b}: {got} != {want}");
        }
    }

    #[test]
    fn splits_artists_like_the_js() {
        assert_eq!(
            split_artists("A, B & C and D feat. E Feat F FEATURING G"),
            ["A", "B", "C", "D", "E", "F", "G"]
        );
        assert_eq!(split_artists("Andy, Grand"), ["Andy", "Grand"]);
        assert_eq!(split_artists(" , "), Vec::<String>::new());
    }

    #[test]
    fn strips_video_noise() {
        assert_eq!(
            strip_video_noise("Song (Official Music Video) [HD]"),
            "Song"
        );
        assert_eq!(strip_video_noise("Song (Клип 2020)"), "Song");
        assert_eq!(strip_video_noise("Song | Music video"), "Song");
        assert_eq!(strip_video_noise("Song (feat. X)"), "Song (feat. X)");
        assert_eq!(strip_video_noise("Song (Lyrics"), "Song (Lyrics");
        assert_eq!(strip_video_noise("Song|x"), "Song|x");
    }

    #[test]
    fn splits_a_dash_title() {
        assert_eq!(
            split_dash("DVN - Можно я с тобой"),
            Some(("DVN", "Можно я с тобой"))
        );
        assert_eq!(split_dash("A B — C - D"), Some(("A B", "C - D")));
        assert_eq!(split_dash("A-B"), None);
        assert_eq!(split_dash("- B"), None);
    }

    #[test]
    fn video_readings() {
        let v = track(
            "v",
            "DVN - Можно я с тобой (Official Video)",
            &["Uploader"],
            Some(200),
        );
        let c = video_candidates(&v);
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].title, "DVN - Можно я с тобой");
        assert_eq!(c[1].title, "Можно я с тобой");
        assert_eq!(c[1].artists, ["DVN", "Uploader"]);
        assert_eq!(c[2].title, "DVN");
        assert_eq!(c[2].artists, ["Можно я с тобой", "Uploader"]);
    }

    #[test]
    fn best_match_needs_title_and_artist() {
        let a = track("a", "Song", &["Other"], None);
        let b = track("b", "Song", &["Artist"], None);
        let c = track("c", "Song (Live)", &["Artist"], None);
        let items: Vec<Candidate> = [&a, &b, &c].into_iter().map(Candidate::of).collect();
        assert_eq!(
            find_best_match(&items, "Artist", None, "Song").map(|t| t.video_id.as_str()),
            Some("b")
        );
        assert!(find_best_match(&items, "Nobody", None, "Song").is_none());
        assert!(find_best_match(&items, "Artist", None, "Different").is_none());
    }

    #[tokio::test]
    async fn matches_a_cyrillic_track_through_its_latin_spelling() {
        let hit = track("x1", "Votsap", &["Ahror"], Some(181));
        let cat = FakeCatalog::default().with(
            SearchFilter::Songs,
            "Ahror Votsap",
            vec![Item::Track(hit)],
        );
        let t = ytrack("Вотсап", None, &["Ахрор"], Some(180_000));
        assert_eq!(match_track(&cat, &t).await.unwrap().video_id, "x1");
        assert_eq!(
            cat.asked(),
            [
                (SearchFilter::Songs, "Ахрор Вотсап".into()),
                (SearchFilter::Songs, "Ahror Votsap".into())
            ]
        );
    }

    #[tokio::test]
    async fn length_rules_out_other_recordings() {
        let long = track("long", "Song", &["Artist"], Some(400));
        let cat = FakeCatalog::default().with(
            SearchFilter::Songs,
            "Artist Song",
            vec![Item::Track(long)],
        );
        let t = ytrack("Song", None, &["Artist"], Some(200_000));
        assert!(match_track(&cat, &t).await.is_none());
        // Songs then videos, one query each (no Latin spelling to add).
        assert_eq!(
            cat.asked(),
            [
                (SearchFilter::Songs, "Artist Song".into()),
                (SearchFilter::Videos, "Artist Song".into())
            ]
        );
    }

    #[tokio::test]
    async fn version_is_searched_first() {
        let remix = track("r", "Song (Remix)", &["Artist"], Some(200));
        let cat = FakeCatalog::default().with(
            SearchFilter::Songs,
            "Artist Song (Remix)",
            vec![Item::Track(remix)],
        );
        let t = ytrack("Song", Some("Remix"), &["Artist"], Some(200_000));
        assert_eq!(match_track(&cat, &t).await.unwrap().video_id, "r");
    }

    #[tokio::test]
    async fn japanese_title_by_artist_and_length() {
        let hit = track("f", "Faded", &["Alan Walker"], Some(212));
        let cat = FakeCatalog::default().with(
            SearchFilter::Songs,
            "Alan Walker フェイデッド",
            vec![Item::Track(hit)],
        );
        let t = ytrack("フェイデッド", None, &["Alan Walker"], Some(212_500));
        assert_eq!(match_track(&cat, &t).await.unwrap().video_id, "f");
        let off = ytrack("フェイデッド", None, &["Alan Walker"], Some(220_000));
        assert!(match_track(&cat, &off).await.is_none());
    }

    #[tokio::test]
    async fn falls_back_to_a_video_upload() {
        let video = track(
            "v",
            "DVN - Можно я с тобой (Official Video)",
            &["Some Channel"],
            Some(195),
        );
        let cat = FakeCatalog::default().with(
            SearchFilter::Videos,
            "DVN Можно я с тобой",
            vec![Item::Track(video)],
        );
        let t = ytrack("Можно я с тобой", None, &["DVN"], Some(190_000));
        assert_eq!(match_track(&cat, &t).await.unwrap().video_id, "v");
    }

    #[tokio::test]
    async fn albums_and_artists_by_name() {
        let album = |id: &str, title: &str| {
            Item::Album(AlbumSummary {
                browse_id: id.into(),
                playlist_id: None,
                title: title.into(),
                kind: AlbumKind::Album,
                artists: vec![],
                year: None,
                thumbnails: vec![],
                explicit: false,
            })
        };
        let artist = |id: &str, name: &str| {
            Item::Artist(ArtistSummary {
                id: id.into(),
                name: name.into(),
                thumbnails: vec![],
                audience: None,
            })
        };
        let cat = FakeCatalog::default()
            .with(
                SearchFilter::Albums,
                "Artist Album",
                vec![
                    album("a1", "Album"),
                    album("a2", "Album"),
                    album("a3", "Other"),
                ],
            )
            .with(
                SearchFilter::Artists,
                "Скриптонит",
                vec![artist("s0", "Skrip"), artist("s1", "Скриптонит")],
            );
        let a = client::Album {
            title: "Album".into(),
            artists: vec!["Artist".into()],
            year: None,
        };
        // A tie goes to the later result, as in the JS.
        assert_eq!(match_album(&cat, &a).await.unwrap().browse_id, "a2");
        assert_eq!(match_artist(&cat, "Скриптонит").await.unwrap().id, "s1");
        assert!(match_artist(&cat, "Nobody").await.is_none());
    }
}
