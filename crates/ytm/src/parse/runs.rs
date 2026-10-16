//! Decoding of subtitle "runs" such as
//! `Song • Скриптонит • Дом с нормальными явлениями • 3:17 • 57M plays`.
//!
//! Linked runs are classified by their browse endpoint (language-independent);
//! unlinked runs by shape (duration, year, counters). The type label in front
//! ("Song", "Album", …) is matched against the English UI strings, which is
//! why the client pins `hl=en`.

use serde_json::Value;

use super::json::{browse_endpoint, str_at};
use crate::models::{AlbumKind, AlbumRef, ArtistRef};

/// Type label that YouTube puts in front of subtitle runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Song,
    Video,
    Album,
    Single,
    Ep,
    Playlist,
    Artist,
    Episode,
    Podcast,
    Profile,
    Station,
}

impl Label {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "song" => Self::Song,
            "video" => Self::Video,
            "album" => Self::Album,
            "single" => Self::Single,
            "ep" => Self::Ep,
            "playlist" => Self::Playlist,
            "artist" => Self::Artist,
            "episode" => Self::Episode,
            "podcast" => Self::Podcast,
            "profile" => Self::Profile,
            "station" => Self::Station,
            _ => return None,
        })
    }

    pub fn album_kind(self) -> Option<AlbumKind> {
        match self {
            Self::Album => Some(AlbumKind::Album),
            Self::Single => Some(AlbumKind::Single),
            Self::Ep => Some(AlbumKind::Ep),
            _ => None,
        }
    }
}

/// Everything that could be read from a list of runs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RunInfo {
    pub label: Option<Label>,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration_s: Option<u32>,
    pub year: Option<u16>,
    /// Counters such as "57M plays", "226 subscribers", "30 songs".
    pub stats: Vec<String>,
}

impl RunInfo {
    /// The first counter that ends with one of `words` ("songs", "tracks").
    pub fn stat_with(&self, words: &[&str]) -> Option<&str> {
        self.stats.iter().map(String::as_str).find(|s| {
            let lower = s.to_lowercase();
            words.iter().any(|w| lower.contains(w))
        })
    }

    /// "57M plays" / "1.3K views", whichever is shown.
    pub fn views(&self) -> Option<String> {
        self.stat_with(&["plays", "views"]).map(str::to_owned)
    }
}

pub fn parse_runs(runs: &[Value]) -> RunInfo {
    let mut info = RunInfo::default();
    let mut runs = runs;

    // A leading unlinked type label followed by a separator (or alone).
    if let Some(first) = runs.first()
        && first.get("navigationEndpoint").is_none()
        && let Some(label) = str_at(first, "/text").and_then(Label::parse)
        && runs.get(1).is_none_or(is_separator)
    {
        info.label = Some(label);
        runs = &runs[1..];
    }

    for run in runs {
        let Some(text) = str_at(run, "/text") else {
            continue;
        };
        if is_separator(run) {
            continue;
        }
        if let Some((id, page_type)) = browse_endpoint(run) {
            match classify_link(id, page_type) {
                Link::Artist => info.artists.push(ArtistRef {
                    id: Some(id.to_owned()),
                    name: text.trim().to_owned(),
                }),
                Link::Album => {
                    info.album.get_or_insert(AlbumRef {
                        id: Some(id.to_owned()),
                        name: text.trim().to_owned(),
                    });
                }
                Link::Other => {}
            }
            continue;
        }
        let t = text.trim();
        if let Some(d) = parse_duration(t) {
            info.duration_s = Some(d);
        } else if let Some(y) = parse_year(t) {
            info.year = Some(y);
        } else if is_stat(t) {
            info.stats.push(t.to_owned());
        } else {
            // Unlinked name: an artist without a channel ("Various Artists").
            info.artists.push(ArtistRef {
                id: None,
                name: t.to_owned(),
            });
        }
    }
    info
}

enum Link {
    Artist,
    Album,
    Other,
}

fn classify_link(id: &str, page_type: Option<&str>) -> Link {
    match page_type {
        Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL") => Link::Artist,
        Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK") => Link::Album,
        Some(_) => Link::Other,
        None if id.starts_with("UC") => Link::Artist,
        None if id.starts_with("MPRE") => Link::Album,
        None => Link::Other,
    }
}

fn is_separator(run: &Value) -> bool {
    run.get("navigationEndpoint").is_none()
        && str_at(run, "/text").is_some_and(|t| matches!(t.trim(), "" | "•" | "·" | "&" | ","))
}

/// `"3:17"` → 197, `"1:02:03"` → 3723.
pub fn parse_duration(s: &str) -> Option<u32> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut total: u32 = 0;
    for p in parts {
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(p.parse().ok()?)?;
    }
    Some(total)
}

fn parse_year(s: &str) -> Option<u16> {
    if s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()) {
        let y: u16 = s.parse().ok()?;
        (1900..=2200).contains(&y).then_some(y)
    } else {
        None
    }
}

/// Counter text like "57M plays" or "2,3 млн просмотров".
fn is_stat(s: &str) -> bool {
    const WORDS: [&str; 12] = [
        "play",
        "view",
        "subscriber",
        "song",
        "track",
        "audience",
        "listener",
        "episode",
        "video",
        "like",
        "follower",
        "save",
    ];
    if !s.starts_with(|c: char| c.is_ascii_digit()) || !s.contains(char::is_whitespace) {
        return false;
    }
    let lower = s.to_lowercase();
    // Pinned to English, but tolerate other locales: no Latin letters at all
    // means it cannot be a Latin artist name like "50 Cent".
    WORDS.iter().any(|w| lower.contains(w)) || !lower.chars().any(|c| c.is_ascii_alphabetic())
}

/// Leading integer of "13 songs", "1,234 tracks".
pub fn leading_count(s: &str) -> Option<u32> {
    let digits: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.' || *c == '\u{a0}')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn link(text: &str, id: &str, pt: &str) -> Value {
        json!({"text": text, "navigationEndpoint": {"browseEndpoint": {"browseId": id,
            "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": pt}}}}})
    }
    fn plain(t: &str) -> Value {
        json!({ "text": t })
    }

    #[test]
    fn song_runs_with_label() {
        let runs = vec![
            plain("Song"),
            plain(" • "),
            link("Скриптонит", "UC7", "MUSIC_PAGE_TYPE_ARTIST"),
            plain(" & "),
            link("T-Fest", "UCW", "MUSIC_PAGE_TYPE_ARTIST"),
            plain(" • "),
            link("Дом", "MPREb_x", "MUSIC_PAGE_TYPE_ALBUM"),
            plain(" • "),
            plain("3:17"),
            plain(" • "),
            plain("57M plays"),
        ];
        let i = parse_runs(&runs);
        assert_eq!(i.label, Some(Label::Song));
        assert_eq!(i.artists.len(), 2);
        assert_eq!(i.artists[1].name, "T-Fest");
        assert_eq!(i.album.as_ref().unwrap().id.as_deref(), Some("MPREb_x"));
        assert_eq!(i.duration_s, Some(197));
        assert_eq!(i.views().as_deref(), Some("57M plays"));
    }

    #[test]
    fn unlinked_artist_and_year() {
        let runs = vec![plain("Various Artists"), plain(" • "), plain("2011")];
        let i = parse_runs(&runs);
        assert_eq!(i.label, None);
        assert_eq!(i.artists[0].id, None);
        assert_eq!(i.year, Some(2011));
    }

    #[test]
    fn label_alone_and_name_like_number() {
        assert_eq!(parse_runs(&[plain("Podcast")]).label, Some(Label::Podcast));
        let i = parse_runs(&[plain("50 Cent")]);
        assert_eq!(i.artists[0].name, "50 Cent");
        assert!(is_stat("2,3 млн просмотров"));
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("0:59"), Some(59));
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("2,343"), None);
        assert_eq!(parse_duration("abc"), None);
        assert_eq!(parse_duration("12"), None);
    }

    #[test]
    fn counts() {
        assert_eq!(leading_count("13 songs"), Some(13));
        assert_eq!(leading_count("1,234 tracks"), Some(1234));
        assert_eq!(leading_count("songs"), None);
    }
}
