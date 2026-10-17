//! LRC → [`Line`]s, ported from `AmLyrics.parseLrcSubtitles` and extended
//! with repeated tags (`[00:10.00][01:20.00]chorus`), `[offset:…]` and
//! enhanced word tags (`<00:10.50>word`).

use crate::model::{Line, Word};

/// How long the last line lasts (as `AmLyrics`).
const LAST_LINE_MS: u64 = 5000;

/// `mm:ss`, `mm:ss.xx` or `mm:ss.xxx` → ms.
fn parse_stamp(s: &str) -> Option<u64> {
    let (m, rest) = s.split_once(':')?;
    let (sec, frac) = rest.split_once('.').unwrap_or((rest, ""));
    if m.is_empty() || m.len() > 3 || sec.len() != 2 || frac.len() > 3 {
        return None;
    }
    let all_digits = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(m) || !all_digits(sec) || !all_digits(frac) {
        return None;
    }
    let m: u64 = m.parse().ok()?;
    let sec: u64 = sec.parse().ok()?;
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac.parse::<u64>().ok()?,
    };
    Some((m * 60 + sec) * 1000 + frac_ms)
}

/// Splits leading `[…]` tags off a line.
fn leading_tags(mut s: &str) -> (Vec<&str>, &str) {
    let mut tags = Vec::new();
    while let Some(rest) = s.strip_prefix('[') {
        let Some(end) = rest.find(']') else { break };
        tags.push(&rest[..end]);
        s = &rest[end + 1..];
    }
    (tags, s)
}

/// `<mm:ss.xx>` word tags: (start, text) pieces. Empty when the text has none.
fn word_pieces(text: &str) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut rest = text;
    let mut seen_tag = false;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('<')
            && let Some(end) = after.find('>')
            && let Some(t) = parse_stamp(&after[..end])
        {
            seen_tag = true;
            out.push((t, String::new()));
            rest = &after[end + 1..];
            continue;
        }
        let next = rest
            .char_indices()
            .skip(1)
            .find(|(_, c)| *c == '<')
            .map_or(rest.len(), |(i, _)| i);
        match out.last_mut() {
            Some((_, txt)) => txt.push_str(&rest[..next]),
            None => out.push((0, rest[..next].to_owned())),
        }
        rest = &rest[next..];
    }
    if !seen_tag {
        return Vec::new();
    }
    out
}

/// Parses LRC text. Lines without a time tag (headers, plain text) are skipped.
pub fn parse(lrc: &str) -> Vec<Line> {
    let mut offset: i64 = 0;
    let mut entries: Vec<(u64, &str)> = Vec::new();
    for raw in lrc.lines() {
        let raw = raw.trim_end_matches('\r').trim_start();
        let (tags, text) = leading_tags(raw);
        for tag in tags {
            if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
            } else if let Some(t) = parse_stamp(tag) {
                entries.push((t, text));
            }
        }
    }
    // LRC offset: positive means the lyrics come sooner.
    let shift = |t: u64| (t as i64 - offset).max(0) as u64;
    entries.sort_by_key(|(t, _)| *t);

    let mut lines = Vec::new();
    for (i, &(start, text)) in entries.iter().enumerate() {
        let end = entries
            .get(i + 1)
            .map_or(start + LAST_LINE_MS, |(next, _)| *next);
        let pieces = word_pieces(text);
        let words: Vec<Word> = pieces
            .iter()
            .enumerate()
            .filter(|(_, (_, t))| !t.is_empty())
            .map(|(j, (s, t))| Word {
                start_ms: shift(if *s == 0 { start } else { *s }),
                end_ms: shift(pieces.get(j + 1).map_or(end, |(n, _)| *n)),
                text: t.clone(),
            })
            .collect();
        let line_text = if pieces.is_empty() {
            text.trim().to_owned()
        } else {
            words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<String>()
                .trim()
                .to_owned()
        };
        if line_text.is_empty() {
            continue;
        }
        lines.push(Line {
            start_ms: shift(start),
            end_ms: shift(end),
            text: line_text,
            words: (!words.is_empty()).then_some(words),
            background: None,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps() {
        assert_eq!(parse_stamp("00:13.13"), Some(13_130));
        assert_eq!(parse_stamp("01:02.345"), Some(62_345));
        assert_eq!(parse_stamp("100:00"), Some(6_000_000));
        assert_eq!(parse_stamp("ti:Song"), None);
        assert_eq!(parse_stamp("1:2.00"), None);
    }

    #[test]
    fn basic_file() {
        let lines = parse(include_str!("../tests/fixtures/basic.lrc"));
        let got: Vec<(u64, u64, &str)> = lines
            .iter()
            .map(|l| (l.start_ms, l.end_ms, l.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (13_130, 16_560, "Hello"),
                (16_560, 27_160, "♪"),
                // The blank line at 27.16 ends "♪" and is dropped itself.
                (29_960, 32_710, "Second, with [brackets]"),
                (32_710, 37_710, "Last line"),
            ]
        );
        assert!(lines.iter().all(|l| l.words.is_none()));
    }

    #[test]
    fn repeated_tags_and_offset() {
        let lines = parse("[offset:+500]\n[00:10.00][00:30.00]Chorus\n[00:20.00]Verse\n");
        let got: Vec<(u64, &str)> = lines
            .iter()
            .map(|l| (l.start_ms, l.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![(9_500, "Chorus"), (19_500, "Verse"), (29_500, "Chorus")]
        );
    }

    #[test]
    fn enhanced_words() {
        let lines = parse("[00:01.00]<00:01.00>Hel<00:01.50>lo <00:02.00>there\n[00:03.00]next\n");
        let w = lines[0].words.as_ref().unwrap();
        assert_eq!(lines[0].text, "Hello there");
        assert_eq!(
            w.iter()
                .map(|w| (w.start_ms, w.end_ms, w.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (1000, 1500, "Hel"),
                (1500, 2000, "lo "),
                (2000, 3000, "there")
            ]
        );
        assert!(lines[1].words.is_none());
    }
}
