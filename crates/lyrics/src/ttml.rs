//! TTML (Apple Music style) → [`Line`]s, ported from `AmLyrics.parseTTML`.
//!
//! Attributes are matched by local name, so `itunes:key`, `lrc:key` and the
//! `ttm:` metadata work whatever prefix or namespace a provider uses.

use roxmltree::{Document, Node};

use crate::model::{Line, Word};

/// Parsed TTML document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ttml {
    pub lines: Vec<Line>,
    pub songwriters: Option<String>,
}

/// A TTML clock value in ms: `1:02.345`, `62.345`, `1.5s`, `300ms`, `1m`, `1h`.
pub fn parse_time(value: Option<&str>, fallback: u64) -> u64 {
    let Some(v) = value.map(|v| v.trim().to_ascii_lowercase()) else {
        return fallback;
    };
    if v.is_empty() {
        return fallback;
    }
    let to_ms = |secs: f64| (secs * 1000.0).round().max(0.0) as u64;
    for (unit, mul) in [("ms", 0.001), ("h", 3600.0), ("m", 60.0), ("s", 1.0)] {
        if let Some(num) = v.strip_suffix(unit)
            && let Ok(n) = num.parse::<f64>()
        {
            return to_ms(n * mul);
        }
    }
    let parts: Option<Vec<f64>> = v.split(':').map(|p| p.parse::<f64>().ok()).collect();
    match parts.as_deref() {
        Some([s]) => to_ms(*s),
        Some([m, s]) => to_ms(m * 60.0 + s),
        Some([h, m, s]) => to_ms(h * 3600.0 + m * 60.0 + s),
        _ => fallback,
    }
}

fn attr<'a>(n: Node<'a, '_>, local: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == local)
        .map(|a| a.value())
}

fn is_elem(n: Node, local: &str) -> bool {
    n.is_element() && n.tag_name().name() == local
}

fn is_bg(n: Node) -> bool {
    n.is_element() && attr(n, "role") == Some("x-bg")
}

fn text_of(n: Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

/// Timed spans among `parent`'s children. A whitespace text node after a
/// span means a word boundary: the span's text gets a trailing space.
/// Returns the words and whether every one had `begin` and `end`.
fn spans(parent: Node, line_begin: u64, line_end: u64) -> (Vec<Word>, bool) {
    let mut words: Vec<Word> = Vec::new();
    let mut timed = false;
    for child in parent.children() {
        if child.is_text() {
            let t = child.text().unwrap_or_default();
            if t.starts_with(char::is_whitespace)
                && let Some(last) = words.last_mut()
                && !last.text.ends_with(' ')
            {
                last.text.push(' ');
            }
            continue;
        }
        if !is_elem(child, "span") || is_bg(child) {
            continue;
        }
        let begin = attr(child, "begin");
        let end = attr(child, "end");
        timed |= begin.is_some() && end.is_some();
        let start_ms = parse_time(begin, line_begin);
        words.push(Word {
            start_ms,
            end_ms: parse_time(end, line_end).max(start_ms),
            text: text_of(child),
        });
    }
    (words, timed)
}

fn line(p: Node) -> Option<Line> {
    let begin_attr = attr(p, "begin");
    let begin = parse_time(begin_attr, 0);
    let end = parse_time(attr(p, "end"), begin);

    let (main, word_synced) = spans(p, begin, end);
    let background: Vec<Word> = p
        .children()
        .filter(|c| is_bg(*c))
        .flat_map(|bg| spans(bg, begin, end).0)
        .filter(|w| !w.text.trim().is_empty())
        .collect();

    let (text, words) = if main.is_empty() {
        let text: String = p.children().filter(|c| !is_bg(*c)).map(text_of).collect();
        (text, None)
    } else {
        let text: String = main.iter().map(|w| w.text.as_str()).collect();
        (text, word_synced.then_some(main))
    };
    let text = text.trim().to_owned();
    if text.is_empty() && background.is_empty() {
        return None;
    }

    let start_ms = match (&words, begin_attr) {
        (Some(w), None) => w.iter().map(|w| w.start_ms).min().unwrap_or(begin),
        _ => begin,
    };
    let end_ms = words
        .iter()
        .flatten()
        .chain(&background)
        .map(|w| w.end_ms)
        .fold(end.max(start_ms), u64::max);

    Some(Line {
        start_ms,
        end_ms,
        text,
        words,
        background: (!background.is_empty()).then_some(background),
    })
}

/// Parses a TTML document. `None` when it is not XML or has no lines.
pub fn parse(ttml: &str) -> Option<Ttml> {
    let doc = Document::parse(ttml)
        .inspect_err(|e| tracing::debug!("bad TTML: {e}"))
        .ok()?;
    let names: Vec<String> = doc
        .descendants()
        .filter(|n| is_elem(*n, "songwriter"))
        .map(|n| text_of(n).trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let mut lines: Vec<Line> = doc
        .descendants()
        .filter(|n| is_elem(*n, "p"))
        .filter_map(line)
        .collect();
    // Stable: lines sharing a start keep document order.
    lines.sort_by_key(|l| l.start_ms);
    if lines.is_empty() {
        return None;
    }
    Some(Ttml {
        lines,
        songwriters: (!names.is_empty()).then(|| names.join(", ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_values() {
        assert_eq!(parse_time(Some("27.395"), 0), 27_395);
        assert_eq!(parse_time(Some("2:22.475"), 0), 142_475);
        assert_eq!(parse_time(Some("1:00:01.5"), 0), 3_601_500);
        assert_eq!(parse_time(Some("1.5s"), 0), 1500);
        assert_eq!(parse_time(Some("300ms"), 0), 300);
        assert_eq!(parse_time(Some("2m"), 0), 120_000);
        assert_eq!(parse_time(Some("junk"), 7), 7);
        assert_eq!(parse_time(None, 9), 9);
    }

    #[test]
    fn word_synced_with_background() {
        let src = include_str!("../tests/fixtures/word.ttml");
        let t = parse(src).expect("parses");
        assert_eq!(t.songwriters.as_deref(), Some("Jane Writer, John Composer"));
        assert_eq!(t.lines.len(), 3);

        let l0 = &t.lines[0];
        assert_eq!((l0.start_ms, l0.end_ms), (1_000, 2_500));
        assert_eq!(l0.text, "Hello bright world");
        let w = l0.words.as_ref().unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].text, "Hello ");
        assert_eq!((w[0].start_ms, w[0].end_ms), (1_000, 1_400));
        assert_eq!(w[2].text, "world");

        // Syllables of one word stay glued: no whitespace between spans.
        let l1 = &t.lines[1];
        assert_eq!(l1.text, "Wonderful day");
        assert_eq!(l1.words.as_ref().unwrap()[0].text, "Won");
        let bg = l1.background.as_ref().unwrap();
        assert_eq!(bg.len(), 2);
        assert_eq!(bg[0].text, "(oh ");
        // The line ends with its background vocals.
        assert_eq!(l1.end_ms, 63_900);

        // An untimed `p` text with a `begin` only.
        let l2 = &t.lines[2];
        assert_eq!(l2.text, "Line only");
        assert!(l2.words.is_none());
        assert_eq!((l2.start_ms, l2.end_ms), (70_000, 72_000));
    }

    #[test]
    fn line_synced_and_sorted() {
        let src = include_str!("../tests/fixtures/line.ttml");
        let t = parse(src).expect("parses");
        let starts: Vec<u64> = t.lines.iter().map(|l| l.start_ms).collect();
        assert_eq!(starts, vec![5_000, 9_000, 12_500]);
        assert!(t.lines.iter().all(|l| l.words.is_none()));
        assert_eq!(t.lines[0].text, "First line");
        assert!(t.songwriters.is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("<html>").is_none());
        assert!(parse("<tt><body/></tt>").is_none());
    }
}
