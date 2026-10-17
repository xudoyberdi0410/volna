//! What the UI gets: one source's lyrics as lines with optional word timing.

use serde::{Deserialize, Serialize};

/// How precisely a [`Lyrics`] is timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Synced {
    /// Plain text: every time is 0.
    None,
    /// Each line has a start and an end.
    Line,
    /// Lines carry per-word (syllable) timing in [`Line::words`].
    Word,
}

/// A timed syllable or word. `text` keeps its trailing space when the next
/// word is a separate word, so concatenating `words` gives the line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<Word>>,
    /// Background vocals (TTML `ttm:role="x-bg"`), word-timed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Vec<Word>>,
}

impl Line {
    /// An untimed line.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            start_ms: 0,
            end_ms: 0,
            text: text.into(),
            words: None,
            background: None,
        }
    }

    fn timed(&self) -> bool {
        self.start_ms > 0 || self.end_ms > 0
    }
}

/// Lyrics from one provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Display label of the provider ("lrc.red", "Apple", "LRCLIB", …).
    pub source: String,
    pub synced: Synced,
    pub lines: Vec<Line>,
    /// The whole text, for unsynced lyrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub songwriters: Option<String>,
}

impl Lyrics {
    /// Builds from parsed lines, deriving [`Synced`] and `plain`.
    pub fn new(source: impl Into<String>, lines: Vec<Line>) -> Self {
        let synced = if lines
            .iter()
            .any(|l| l.words.as_ref().is_some_and(|w| !w.is_empty()))
        {
            Synced::Word
        } else if lines.iter().any(Line::timed) {
            Synced::Line
        } else {
            Synced::None
        };
        let plain = (synced == Synced::None).then(|| {
            lines
                .iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        });
        Self {
            source: source.into(),
            synced,
            lines,
            plain,
            songwriters: None,
        }
    }

    /// Untimed lyrics from text; blank lines and lines starting with
    /// `skip_prefix` (Genius section headers like `[Chorus]`) are dropped.
    pub fn from_plain(source: impl Into<String>, text: &str, skip_prefix: Option<char>) -> Self {
        let lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .filter(|l| skip_prefix.is_none_or(|p| !l.starts_with(p)))
            .map(Line::plain)
            .collect();
        Self::new(source, lines)
    }

    pub fn with_songwriters(mut self, songwriters: Option<String>) -> Self {
        self.songwriters = songwriters.filter(|s| !s.is_empty());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// What the providers search by.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackQuery {
    pub title: String,
    /// Artists joined with ", " (as the old UI sent them).
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub isrc: Option<String>,
    /// YouTube Music video id, for the YTM fallback.
    #[serde(default)]
    pub video_id: Option<String>,
}

impl TrackQuery {
    pub(crate) fn album(&self) -> Option<&str> {
        self.album
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    pub(crate) fn isrc(&self) -> Option<&str> {
        self.isrc
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    /// Duration in whole seconds, as every provider wants it.
    pub(crate) fn duration_s(&self) -> Option<u64> {
        self.duration_ms
            .filter(|&d| d > 0)
            .map(|d| (d as f64 / 1000.0).round() as u64)
    }

    pub(crate) fn has_title_artist(&self) -> bool {
        !self.title.trim().is_empty() && !self.artist.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_level_is_derived() {
        let plain = Lyrics::from_plain("Genius", "[Verse]\nHello\n\nWorld\n", Some('['));
        assert_eq!(plain.synced, Synced::None);
        assert_eq!(plain.lines.len(), 2);
        assert_eq!(plain.plain.as_deref(), Some("Hello\nWorld"));

        let mut line = Line::plain("x");
        line.start_ms = 1000;
        let l = Lyrics::new("LRCLIB", vec![line.clone()]);
        assert_eq!(l.synced, Synced::Line);
        assert!(l.plain.is_none());

        line.words = Some(vec![Word {
            start_ms: 1000,
            end_ms: 1200,
            text: "x".into(),
        }]);
        assert_eq!(Lyrics::new("lrc.red", vec![line]).synced, Synced::Word);
    }

    #[test]
    fn serializes_camel_case() {
        let l = Lyrics::new("LRCLIB", vec![Line::plain("a")]);
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["synced"], "none");
        assert_eq!(v["lines"][0]["startMs"], 0);
        assert!(v["lines"][0].get("words").is_none());
    }
}
