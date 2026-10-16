//! HTTP `Range` handling for `volna://stream/`.
//!
//! The Tauri custom-protocol API needs the whole response body up front, so
//! instead of streaming a whole track we answer every Range request with at
//! most `cap` bytes. Media elements handle a short `206` fine: they read the
//! `Content-Range`, play what they got and ask for the next range.

use crate::error::{Result, StreamError};

/// A single byte range as the client asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSpec {
    /// `bytes=start-` or `bytes=start-end` (end inclusive).
    From { start: u64, end: Option<u64> },
    /// `bytes=-n`: the last `n` bytes.
    Suffix(u64),
}

impl RangeSpec {
    /// Parses a `Range` header value. Anything malformed or not in bytes
    /// yields `None`: RFC 9110 says such a header is ignored, not rejected.
    /// For a multi-range request only the first range is served.
    pub fn parse(header: &str) -> Option<Self> {
        let spec = header.trim().strip_prefix("bytes=")?;
        let first = spec.split(',').next()?.trim();
        let (start, end) = first.split_once('-')?;
        let (start, end) = (start.trim(), end.trim());

        if start.is_empty() {
            return end.parse().ok().map(Self::Suffix);
        }
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() {
            None
        } else {
            let end: u64 = end.parse().ok()?;
            if end < start {
                return None;
            }
            Some(end)
        };
        Some(Self::From { start, end })
    }
}

/// The byte span actually served (both ends inclusive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u64,
    pub end: u64,
}

impl Span {
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    /// Always `false`: a span holds at least one byte by construction.
    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }

    /// Value for the `Range` header sent upstream.
    pub fn range_header(&self) -> String {
        format!("bytes={}-{}", self.start, self.end)
    }

    /// Value for the `Content-Range` response header.
    pub fn content_range(&self, total: u64) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, total)
    }
}

/// Decides which bytes to serve for `spec` (no header = from the start),
/// limited to `cap` bytes. `total` is the stream length when known; a suffix
/// range needs it, so callers must find it out first in that case.
pub fn plan(spec: Option<RangeSpec>, total: Option<u64>, cap: u64) -> Result<Span> {
    debug_assert!(cap > 0);
    let spec = spec.unwrap_or(RangeSpec::From {
        start: 0,
        end: None,
    });

    let (start, end) = match (spec, total) {
        (RangeSpec::From { start, end }, Some(total)) => {
            if start >= total {
                return Err(StreamError::RangeNotSatisfiable { total });
            }
            (start, end.unwrap_or(u64::MAX).min(total - 1))
        }
        (RangeSpec::From { start, end }, None) => (start, end.unwrap_or(u64::MAX)),
        (RangeSpec::Suffix(n), Some(total)) => {
            if n == 0 || total == 0 {
                return Err(StreamError::RangeNotSatisfiable { total });
            }
            (total - n.min(total), total - 1)
        }
        (RangeSpec::Suffix(_), None) => {
            unreachable!("callers learn the stream length before planning a suffix range")
        }
    };

    let end = end.min(start.saturating_add(cap - 1));
    Ok(Span { start, end })
}

/// Parses an upstream `Content-Range: bytes a-b/total` into `(span, total)`.
/// `total` is `None` for `*`.
pub fn parse_content_range(value: &str) -> Option<(Span, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let span = Span {
        start: start.trim().parse().ok()?,
        end: end.trim().parse().ok()?,
    };
    if span.end < span.start {
        return None;
    }
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse().ok()?),
    };
    Some((span, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    #[test]
    fn parses_common_forms() {
        assert_eq!(
            RangeSpec::parse("bytes=0-"),
            Some(RangeSpec::From {
                start: 0,
                end: None
            })
        );
        assert_eq!(
            RangeSpec::parse("bytes=100-199"),
            Some(RangeSpec::From {
                start: 100,
                end: Some(199)
            })
        );
        assert_eq!(RangeSpec::parse("bytes=-500"), Some(RangeSpec::Suffix(500)));
        assert_eq!(
            RangeSpec::parse(" bytes=5-9, 20-30"),
            Some(RangeSpec::From {
                start: 5,
                end: Some(9)
            })
        );
    }

    #[test]
    fn ignores_malformed_headers() {
        for bad in [
            "",
            "bytes=",
            "bytes=-",
            "bytes=a-b",
            "bytes=9-5",
            "items=0-1",
            "0-1",
        ] {
            assert_eq!(RangeSpec::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn caps_open_range() {
        let span = plan(RangeSpec::parse("bytes=0-"), Some(10 * MB), 2 * MB).unwrap();
        assert_eq!(
            span,
            Span {
                start: 0,
                end: 2 * MB - 1
            }
        );
        assert_eq!(span.len(), 2 * MB);
        assert_eq!(
            span.content_range(10 * MB),
            format!("bytes 0-{}/{}", 2 * MB - 1, 10 * MB)
        );
    }

    #[test]
    fn clamps_to_total() {
        let span = plan(RangeSpec::parse("bytes=900-5000"), Some(1000), MB).unwrap();
        assert_eq!(
            span,
            Span {
                start: 900,
                end: 999
            }
        );
    }

    #[test]
    fn small_explicit_range_is_served_exactly() {
        let span = plan(RangeSpec::parse("bytes=10-19"), Some(1000), MB).unwrap();
        assert_eq!(span.len(), 10);
        assert_eq!(span.range_header(), "bytes=10-19");
    }

    #[test]
    fn missing_header_starts_at_zero() {
        assert_eq!(
            plan(None, Some(50), MB).unwrap(),
            Span { start: 0, end: 49 }
        );
    }

    #[test]
    fn suffix_range() {
        assert_eq!(
            plan(RangeSpec::parse("bytes=-100"), Some(1000), MB).unwrap(),
            Span {
                start: 900,
                end: 999
            }
        );
        // Larger than the file: the whole file.
        assert_eq!(
            plan(RangeSpec::parse("bytes=-5000"), Some(1000), MB).unwrap(),
            Span { start: 0, end: 999 }
        );
    }

    #[test]
    fn start_past_end_is_416() {
        let err = plan(RangeSpec::parse("bytes=1000-"), Some(1000), MB).unwrap_err();
        assert!(matches!(
            err,
            StreamError::RangeNotSatisfiable { total: 1000 }
        ));
        assert!(plan(RangeSpec::parse("bytes=-0"), Some(1000), MB).is_err());
    }

    #[test]
    fn unknown_total_only_caps() {
        assert_eq!(
            plan(RangeSpec::parse("bytes=100-"), None, 10).unwrap(),
            Span {
                start: 100,
                end: 109
            }
        );
    }

    #[test]
    fn parses_upstream_content_range() {
        assert_eq!(
            parse_content_range("bytes 0-65535/3433755"),
            Some((
                Span {
                    start: 0,
                    end: 65535
                },
                Some(3433755)
            ))
        );
        assert_eq!(
            parse_content_range("bytes 5-9/*"),
            Some((Span { start: 5, end: 9 }, None))
        );
        assert_eq!(parse_content_range("bytes */100"), None);
        assert_eq!(parse_content_range("garbage"), None);
    }
}
