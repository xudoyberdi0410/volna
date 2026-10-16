//! Tiny helpers for walking InnerTube JSON without panicking.
//!
//! Paths are JSON pointers (`/contents/0/musicShelfRenderer`), which
//! `serde_json` resolves for both object keys and array indices.

use serde_json::Value;

use crate::models::Thumbnail;

static EMPTY: Vec<Value> = Vec::new();

pub fn at<'a>(v: &'a Value, ptr: &str) -> Option<&'a Value> {
    v.pointer(ptr)
}

pub fn str_at<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr)?.as_str()
}

pub fn string_at(v: &Value, ptr: &str) -> Option<String> {
    str_at(v, ptr).map(str::to_owned)
}

/// Array at `ptr`, or an empty slice when missing / not an array.
pub fn arr_at<'a>(v: &'a Value, ptr: &str) -> &'a [Value] {
    v.pointer(ptr)
        .and_then(Value::as_array)
        .map_or(EMPTY.as_slice(), Vec::as_slice)
}

/// Concatenated text of a `{runs:[…]}` or `{simpleText:…}` object.
pub fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_owned());
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        return Some(s.to_owned());
    }
    let runs = v.get("runs")?.as_array()?;
    let s: String = runs
        .iter()
        .filter_map(|r| r.get("text").and_then(Value::as_str))
        .collect();
    Some(s)
}

/// Like [`text`] at a pointer, with empty strings mapped to `None`.
pub fn text_at(v: &Value, ptr: &str) -> Option<String> {
    v.pointer(ptr)
        .and_then(text)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Browse endpoint of a run / renderer: `(browseId, pageType)`.
pub fn browse_endpoint(v: &Value) -> Option<(&str, Option<&str>)> {
    let be = v.pointer("/navigationEndpoint/browseEndpoint")?;
    let id = be.get("browseId")?.as_str()?;
    let page_type = str_at(
        be,
        "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
    );
    Some((id, page_type))
}

/// Parses any of the thumbnail shapes YouTube Music uses on `v`.
pub fn thumbnails(v: &Value) -> Vec<Thumbnail> {
    const PATHS: [&str; 6] = [
        "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/croppedSquareThumbnailRenderer/thumbnail/thumbnails",
        "/foregroundThumbnail/musicThumbnailRenderer/thumbnail/thumbnails",
        "/thumbnail/thumbnails",
        "/thumbnails",
    ];
    PATHS
        .iter()
        .map(|p| arr_at(v, p))
        .find(|a| !a.is_empty())
        .map(|a| a.iter().filter_map(thumbnail).collect())
        .unwrap_or_default()
}

fn thumbnail(v: &Value) -> Option<Thumbnail> {
    let url = v.get("url")?.as_str()?;
    let dim = |k| {
        v.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0)
    };
    Some(Thumbnail {
        url: url.to_owned(),
        width: dim("width"),
        height: dim("height"),
    })
}

/// Whether any badge in `badges` is the explicit-content badge.
pub fn has_explicit_badge(v: &Value) -> bool {
    ["badges", "subtitleBadges"].iter().any(|k| {
        arr_at(v, &format!("/{k}")).iter().any(|b| {
            str_at(b, "/musicInlineBadgeRenderer/icon/iconType") == Some("MUSIC_EXPLICIT_BADGE")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_joins_runs_and_simple_text() {
        assert_eq!(
            text(&json!({"runs":[{"text":"a"},{"text":" • "},{"text":"b"}]})).as_deref(),
            Some("a • b")
        );
        assert_eq!(text(&json!({"simpleText":"x"})).as_deref(), Some("x"));
        assert_eq!(text(&json!({"other":1})), None);
    }

    #[test]
    fn missing_paths_are_harmless() {
        let v = json!({"a":[{"b":"c"}]});
        assert_eq!(str_at(&v, "/a/0/b"), Some("c"));
        assert_eq!(str_at(&v, "/a/5/b"), None);
        assert!(arr_at(&v, "/nope").is_empty());
        assert!(thumbnails(&v).is_empty());
    }

    #[test]
    fn thumbnails_tolerate_missing_dimensions() {
        let v = json!({"thumbnail":{"thumbnails":[{"url":"u1","width":60,"height":60},{"url":"u2"},{"nourl":1}]}});
        let t = thumbnails(&v);
        assert_eq!(t.len(), 2);
        assert_eq!(t[1].width, 0);
    }
}
