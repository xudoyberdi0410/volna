//! lyricsplus ("KPoe") JSON → [`Line`]s, ported from `AmLyrics.convertKPoeLyrics`.

use serde_json::Value;

use crate::model::{Line, Word};

/// `AmLyrics.toMilliseconds`: integers are ms, fractions are seconds.
fn to_ms(v: Option<&Value>, fallback: u64) -> u64 {
    let n = match v {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if n.is_finite() && n.fract() != 0.0 => (n * 1000.0).round().max(0.0) as u64,
        Some(n) if n.is_finite() => n.max(0.0) as u64,
        _ => fallback,
    }
}

/// The provider label of a payload (`metadata.source`, e.g. "qApple").
pub fn source_label(payload: &Value) -> String {
    let m = &payload["metadata"];
    m["source"]
        .as_str()
        .or_else(|| m["provider"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("LyricsPlus (KPoe)")
        .to_owned()
}

/// Lines of a lyricsplus payload; `None` when it has none.
pub fn convert(payload: &Value) -> Option<Vec<Line>> {
    let raw = payload["lyrics"]
        .as_array()
        .or_else(|| payload["data"]["lyrics"].as_array())
        .or_else(|| payload["data"].as_array())?;
    let line_type = matches!(payload["type"].as_str(), Some("Line" | "line"));

    let mut lines = Vec::new();
    for entry in raw.iter().filter(|e| e.is_object()) {
        let line_text = entry["text"].as_str().unwrap_or_default();
        let start = to_ms(entry.get("time"), 0);
        let duration = to_ms(entry.get("duration"), 0);
        let explicit_end = to_ms(entry.get("endTime"), 0);
        let end = if explicit_end > 0 {
            explicit_end
        } else {
            start + duration
        };

        let syllabus: Vec<&Value> = entry["syllabus"]
            .as_array()
            .or_else(|| entry["words"].as_array())
            .map(|a| a.iter().filter(|s| s.is_object()).collect())
            .unwrap_or_default();

        let mut main = Vec::new();
        let mut bg = Vec::new();
        if !line_type {
            for syl in &syllabus {
                let s = to_ms(syl.get("time"), start);
                let d = to_ms(syl.get("duration"), 0);
                let e = if d == 0 && syllabus.len() == 1 {
                    end
                } else {
                    s + d
                };
                let w = Word {
                    start_ms: s,
                    end_ms: e,
                    text: syl["text"].as_str().unwrap_or_default().to_owned(),
                };
                if syl["isBackground"].as_bool() == Some(true) {
                    bg.push(w);
                } else {
                    main.push(w);
                }
            }
        }

        let text = if !line_text.is_empty() {
            line_text.trim().to_owned()
        } else {
            main.iter()
                .map(|w| w.text.as_str())
                .collect::<String>()
                .trim()
                .to_owned()
        };
        if text.is_empty() && bg.is_empty() {
            continue;
        }
        lines.push(Line {
            start_ms: start,
            end_ms: end,
            text,
            words: (!main.is_empty()).then_some(main),
            background: (!bg.is_empty()).then_some(bg),
        });
    }
    (!lines.is_empty()).then_some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_payload() {
        let p: Value =
            serde_json::from_str(include_str!("../tests/fixtures/kpoe-word.json")).unwrap();
        assert_eq!(source_label(&p), "qApple");
        let lines = convert(&p).unwrap();
        assert_eq!(lines.len(), 2);
        let l0 = &lines[0];
        assert_eq!((l0.start_ms, l0.end_ms), (27_395, 28_960));
        assert_eq!(l0.text, "Hello bright world");
        let w = l0.words.as_ref().unwrap();
        assert_eq!(w[0].text, "Hello ");
        assert_eq!((w[0].start_ms, w[0].end_ms), (27_395, 27_549));
        let l1 = &lines[1];
        assert_eq!(l1.background.as_ref().unwrap()[0].text, "(oh)");
        assert_eq!(l1.words.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn line_payload_in_seconds() {
        let p = serde_json::json!({
            "type": "Line",
            "metadata": {"provider": "Musixmatch"},
            "data": [
                {"time": 1.5, "duration": 2.25, "text": "One",
                 "syllabus": [{"time": 1.5, "duration": 1.0, "text": "One"}]},
                {"time": 4.0, "endTime": 6.0, "text": "Two"},
                {"time": 7.0, "text": ""}
            ]
        });
        assert_eq!(source_label(&p), "Musixmatch");
        let lines = convert(&p).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0].start_ms, lines[0].end_ms), (1500, 3750));
        assert!(lines[0].words.is_none(), "Line type drops the syllabus");
        // 4.0 is an integer in JSON → read as ms, like AmLyrics.
        assert_eq!(lines[1].start_ms, 4);
    }

    #[test]
    fn empty_payloads() {
        assert!(convert(&serde_json::json!({"lyrics": []})).is_none());
        assert!(convert(&serde_json::json!({"error": {"status": 404}})).is_none());
    }
}
