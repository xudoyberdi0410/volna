//! The track as the services see it, and the artist rules of the old UI.

use serde::{Deserialize, Serialize};

pub const UNKNOWN_ARTIST: &str = "Unknown Artist";

/// What a scrobble needs; the daemon builds it from player or library tracks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    pub title: String,
    /// In credit order; the first is the main artist.
    pub artists: Vec<String>,
    pub album: Option<String>,
    /// Seconds.
    pub duration: Option<f64>,
    pub track_number: Option<u32>,
    /// YouTube video id, for logs.
    pub video_id: Option<String>,
}

/// A finished play: the track and when it started (unix seconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Play {
    pub track: Track,
    pub started_at: i64,
}

impl Track {
    /// The first named artist, as `_getScrobbleArtist` in `lastfm.js`.
    pub fn first_artist(&self) -> &str {
        self.artists
            .iter()
            .map(|a| a.trim())
            .find(|a| !a.is_empty())
            .unwrap_or(UNKNOWN_ARTIST)
    }

    /// The first artist without "feat. X", "& Y", "x Z" (Libre.fm, Maloja).
    pub fn main_artist(&self) -> String {
        main_artist(self.first_artist(), false)
    }

    /// As [`main_artist`](Self::main_artist), with ListenBrainz's looser
    /// `feat\.?\s*` rule.
    pub fn main_artist_lb(&self) -> String {
        main_artist(self.first_artist(), true)
    }

    /// Whole seconds, when known.
    pub fn duration_s(&self) -> Option<u64> {
        self.duration
            .filter(|d| d.is_finite() && *d >= 1.0)
            .map(|d| d.floor() as u64)
    }
}

/// Port of the split regex of the UI:
/// `/\s*[&]\s*|\s+feat\.?\s+|\s+ft\.?\s+|\s+featuring\s+|\s+with\s+|\s+x\s+/i`
/// (ListenBrainz: `feat\.?\s*`, `ft\.?\s*`), keeping what is before the first
/// match.
pub fn main_artist(name: &str, loose: bool) -> String {
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let cut = (0..chars.len()).find(|&i| {
        let (_, c) = chars[i];
        if c == '&' {
            return true;
        }
        // The other separators need whitespace before them.
        if i == 0 || !chars[i - 1].1.is_whitespace() || c.is_whitespace() {
            return false;
        }
        let rest = &name[chars[i].0..];
        for (word, dot, loose_tail) in [
            ("feat", true, loose),
            ("ft", true, loose),
            ("featuring", false, false),
            ("with", false, false),
            ("x", false, false),
        ] {
            let Some(head) = rest.get(..word.len()) else {
                continue;
            };
            if !head.eq_ignore_ascii_case(word) {
                continue;
            }
            let mut tail = &rest[word.len()..];
            if dot && tail.starts_with('.') {
                tail = &tail[1..];
            }
            if loose_tail || tail.starts_with(char::is_whitespace) {
                return true;
            }
        }
        false
    });
    let head = match cut {
        Some(i) => &name[..chars[i].0],
        None => name,
    };
    let head = head.trim();
    if head.is_empty() {
        UNKNOWN_ARTIST.to_owned()
    } else {
        head.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_ui() {
        let strict = |s| main_artist(s, false);
        assert_eq!(strict("Simon & Garfunkel"), "Simon");
        assert_eq!(strict("Drake feat. Rihanna"), "Drake");
        assert_eq!(strict("Drake Feat Rihanna"), "Drake");
        assert_eq!(strict("A ft. B"), "A");
        assert_eq!(strict("A featuring B"), "A");
        assert_eq!(strict("A with B"), "A");
        assert_eq!(strict("A x B"), "A");
        assert_eq!(strict("Xzibit"), "Xzibit");
        assert_eq!(strict("Max Richter"), "Max Richter");
        assert_eq!(strict("Featherweight"), "Featherweight");
        assert_eq!(strict("A feat.B"), "A feat.B", "strict wants a space after");
        assert_eq!(main_artist("A feat.B", true), "A");
        assert_eq!(main_artist("A featB", true), "A");
        assert_eq!(strict("Скриптонит & 104"), "Скриптонит");
        assert_eq!(strict("& Co"), UNKNOWN_ARTIST);
        assert_eq!(strict(""), UNKNOWN_ARTIST);
    }

    #[test]
    fn first_artist_skips_blanks() {
        let t = Track {
            artists: vec![" ".into(), "B".into()],
            ..Default::default()
        };
        assert_eq!(t.first_artist(), "B");
        assert_eq!(Track::default().first_artist(), UNKNOWN_ARTIST);
    }
}
