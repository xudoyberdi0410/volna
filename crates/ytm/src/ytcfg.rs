//! Extracting the InnerTube config (`ytcfg.set({...})`) from the
//! music.youtube.com HTML shell.

use serde_json::{Map, Value};

/// The parts of ytcfg the client needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtCfg {
    pub client_version: String,
    pub visitor_data: Option<String>,
    pub api_key: Option<String>,
    pub gl: Option<String>,
    pub hl: Option<String>,
}

/// Parses every `ytcfg.set({...})` object in `html` and merges them.
/// Falls back to a plain string search when the objects are not valid JSON
/// (some of them use single quotes).
pub fn parse(html: &str) -> Option<YtCfg> {
    let mut merged = Map::new();
    let mut rest = html;
    while let Some(pos) = rest.find("ytcfg.set(") {
        let after = &rest[pos + "ytcfg.set(".len()..];
        if let Some(obj) = balanced_object(after)
            && let Ok(Value::Object(m)) = serde_json::from_str::<Value>(obj)
        {
            merged.extend(m);
        }
        rest = after;
    }

    let get = |key: &str| -> Option<String> {
        merged
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| raw_string_value(html, key))
            .filter(|s| !s.is_empty())
    };

    Some(YtCfg {
        client_version: get("INNERTUBE_CLIENT_VERSION")
            .or_else(|| get("INNERTUBE_CONTEXT_CLIENT_VERSION"))?,
        visitor_data: get("VISITOR_DATA"),
        api_key: get("INNERTUBE_API_KEY"),
        gl: get("GL").or_else(|| get("INNERTUBE_CONTEXT_GL")),
        hl: get("HL").or_else(|| get("INNERTUBE_CONTEXT_HL")),
    })
}

/// The `{...}` prefix of `s`, matching braces outside of string literals.
fn balanced_object(s: &str) -> Option<&str> {
    let s = s.trim_start();
    if !s.starts_with('{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_str: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if let Some(q) = in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// `"KEY":"value"` anywhere in the page.
fn raw_string_value(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = html.find(&needle)? + needle.len();
    let end = html[start..].find('"')? + start;
    Some(html[start..end].to_owned())
}

/// A plausible `WEB_REMIX` client version for today's date, used only when
/// the HTML shell could not be read: `1.YYYYMMDD.01.00`.
pub fn fallback_client_version(unix_secs: u64) -> String {
    let (y, m, d) = civil_from_days((unix_secs / 86_400) as i64);
    format!("1.{y:04}{m:02}{d:02}.01.00")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_shell() {
        let html = include_str!("../tests/fixtures/ytcfg.html");
        let cfg = parse(html).expect("ytcfg");
        assert_eq!(cfg.client_version, "1.20261013.15.00");
        assert!(cfg.visitor_data.as_deref().unwrap().starts_with("Cgt"));
        assert_eq!(
            cfg.api_key.as_deref(),
            Some("AIzaSyC9XL3ZjWddXya6X74dJoCTL-WEYFDNX30")
        );
        assert_eq!(cfg.hl.as_deref(), Some("en"));
    }

    #[test]
    fn falls_back_to_raw_search() {
        let html = r#"<script>ytcfg.set({'X': 1, "INNERTUBE_CLIENT_VERSION":"1.2.3"})</script>"#;
        assert_eq!(parse(html).unwrap().client_version, "1.2.3");
        assert!(parse("<html>consent page</html>").is_none());
    }

    #[test]
    fn braces_inside_strings() {
        assert_eq!(
            balanced_object(r#"{"a":"}{","b":{}} tail"#),
            Some(r#"{"a":"}{","b":{}}"#)
        );
        assert_eq!(balanced_object("nope"), None);
    }

    #[test]
    fn fallback_version_uses_utc_date() {
        // 2026-10-16T00:00:00Z
        assert_eq!(fallback_client_version(1_792_108_800), "1.20261016.01.00");
        assert_eq!(fallback_client_version(0), "1.19700101.01.00");
        // 2024-02-29
        assert_eq!(fallback_client_version(1_709_164_800), "1.20240229.01.00");
    }
}
