//! Source ordering, ported from `AmLyrics.getRankForCollected` and
//! `mergeAndSortSources`: word-synced first (Apple, Bini, Unison, QQ,
//! Musixmatch, LRCLIB), then line-synced in the same order, then plain.

use crate::model::Lyrics;

fn has_word_sync(l: &Lyrics) -> bool {
    l.lines
        .iter()
        .any(|line| line.words.as_ref().is_some_and(|w| w.len() > 1))
}

fn is_unsynced(l: &Lyrics) -> bool {
    !l.lines.is_empty() && l.lines.iter().all(|x| x.start_ms == 0 && x.end_ms == 0)
}

/// Lower is better.
pub fn rank(l: &Lyrics) -> u32 {
    let lower = l.source.to_lowercase();
    if lower == "lrc.red" {
        return 0;
    }
    let qq = lower.contains("qq") || lower.contains("lyricsplus");
    let order = [
        lower.contains("apple"),
        lower.contains("bini"),
        lower.contains("unison"),
        qq,
        lower.contains("musixmatch"),
        lower.contains("lrclib"),
    ];
    let pos = order.iter().position(|&m| m);
    let tier = |base: u32, others: u32| pos.map_or(others, |p| base + p as u32);
    if has_word_sync(l) {
        tier(1, 7)
    } else if !is_unsynced(l) {
        tier(8, 14)
    } else if pos.is_some() {
        tier(15, 0)
    } else if lower.contains("genius") {
        21
    } else if lower.contains("youtube") {
        22
    } else {
        30
    }
}

/// `AmLyrics.getDisplaySourceLabel`.
pub fn display_label(source: &str) -> String {
    if source.to_lowercase().contains("lyricsplus") {
        "QQ".to_owned()
    } else {
        source.to_owned()
    }
}

/// Key two labels of the same source share (`AmLyrics.getSourceKey`).
pub fn source_key(source: &str) -> String {
    let lower = source.trim().to_lowercase();
    if lower.contains("lyricsplus") || lower == "qq" {
        return "qq".to_owned();
    }
    lower.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One result per display label (first wins), best first.
pub fn merge_and_sort(collected: Vec<Lyrics>) -> Vec<Lyrics> {
    let mut out: Vec<Lyrics> = Vec::new();
    for mut l in collected {
        l.source = display_label(&l.source);
        if !l.is_empty() && !out.iter().any(|o| o.source == l.source) {
            out.push(l);
        }
    }
    out.sort_by_key(rank);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Line, Word};

    fn timed(source: &str, words: usize) -> Lyrics {
        let w = (0..words)
            .map(|i| Word {
                start_ms: 1000 + i as u64,
                end_ms: 1001 + i as u64,
                text: "a ".into(),
            })
            .collect::<Vec<_>>();
        let mut line = Line::plain("a");
        line.start_ms = 1000;
        line.end_ms = 2000;
        line.words = (!w.is_empty()).then_some(w);
        Lyrics::new(source, vec![line])
    }

    #[test]
    fn order_matches_am_lyrics() {
        let plain = |s: &str| Lyrics::from_plain(s, "a", None);
        let sorted = merge_and_sort(vec![
            plain("Genius"),
            timed("LRCLIB", 0),
            timed("Musixmatch", 3),
            plain("LRCLIB (unsynced)"),
            timed("LyricsPlus (KPoe)", 0),
            timed("qApple", 3),
            timed("BiniLyrics", 3),
            timed("lrc.red", 0),
            plain("YouTube Music"),
        ]);
        let labels: Vec<&str> = sorted.iter().map(|l| l.source.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "lrc.red",
                "qApple",
                "BiniLyrics",
                "Musixmatch",
                "QQ",
                "LRCLIB",
                "LRCLIB (unsynced)",
                "Genius",
                "YouTube Music",
            ]
        );
    }

    #[test]
    fn duplicates_and_empties_dropped() {
        let sorted = merge_and_sort(vec![
            timed("Apple", 2),
            timed("Apple", 0),
            Lyrics::new("Unison", vec![]),
        ]);
        assert_eq!(sorted.len(), 1);
        assert_eq!(source_key("LyricsPlus (KPoe)"), "qq");
        assert_eq!(source_key(" LRCLIB  (unsynced) "), "lrclib (unsynced)");
    }
}
