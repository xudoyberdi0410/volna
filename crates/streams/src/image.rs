//! Cover images: fetched through the shared client (so the proxy applies)
//! from an allowlist of Google image hosts, cached on disk forever.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use sha2::{Digest, Sha256};
use url::Url;

use crate::error::{Result, StreamError};
use crate::stream::Network;

/// Covers are tens of kilobytes; anything this big is not a cover.
pub const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Hosts YouTube Music serves artwork from (the host itself or a subdomain).
const ALLOWED_HOSTS: &[&str] = &["googleusercontent.com", "ytimg.com", "ggpht.com"];

#[derive(Debug, Clone)]
pub struct Image {
    pub content_type: &'static str,
    pub bytes: Bytes,
    pub from_cache: bool,
}

pub struct ImageCache<N> {
    net: Arc<N>,
    dir: PathBuf,
}

impl<N: Network> ImageCache<N> {
    /// `dir` is created on first write.
    pub fn new(net: Arc<N>, dir: impl Into<PathBuf>) -> Self {
        Self {
            net,
            dir: dir.into(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Returns the image at `raw_url`, resized to `size` px where the host
    /// supports it (`lh3.googleusercontent.com` style `=w…-h…` suffixes).
    pub async fn get(&self, raw_url: &str, size: Option<u32>) -> Result<Image> {
        let (url, path) = self.locate(raw_url, size)?;

        if let Ok(bytes) = tokio::fs::read(&path).await
            && let Some(content_type) = sniff(&bytes)
        {
            return Ok(Image {
                content_type,
                bytes: bytes.into(),
                from_cache: true,
            });
        }

        let bytes = self.download(&url).await?;
        let content_type = sniff(&bytes).ok_or(StreamError::UpstreamStatus(200))?;
        if let Err(e) = self.store(&path, &bytes).await {
            tracing::warn!("cannot cache image at {}: {e}", path.display());
        }
        Ok(Image {
            content_type,
            bytes,
            from_cache: false,
        })
    }

    /// Like [`Self::get`], but returns the cache file (fetched if missing),
    /// for consumers that want a `file://` URL (MPRIS `mpris:artUrl`).
    /// The file has no extension; its content is a JPEG/PNG/WebP image.
    pub async fn get_path(&self, raw_url: &str, size: Option<u32>) -> Result<PathBuf> {
        let (url, path) = self.locate(raw_url, size)?;
        if tokio::fs::metadata(&path).await.is_ok_and(|m| m.len() > 0) {
            return Ok(path);
        }
        let bytes = self.download(&url).await?;
        sniff(&bytes).ok_or(StreamError::UpstreamStatus(200))?;
        if let Err(e) = self.store(&path, &bytes).await {
            // A concurrent `get` of the same URL may have won the rename.
            if tokio::fs::metadata(&path).await.is_ok_and(|m| m.len() > 0) {
                return Ok(path);
            }
            // No dedicated error kind for a full disk; callers only log it.
            return Err(StreamError::InvalidUrl(format!(
                "cannot cache {}: {e}",
                path.display()
            )));
        }
        Ok(path)
    }

    fn locate(&self, raw_url: &str, size: Option<u32>) -> Result<(Url, PathBuf)> {
        let url = check_url(raw_url)?;
        let url = match size {
            Some(s) => resize(url, s),
            None => url,
        };
        let path = self.dir.join(cache_name(url.as_str()));
        Ok((url, path))
    }

    async fn download(&self, url: &Url) -> Result<Bytes> {
        let mut resp = self.net.client().get(url.clone()).send().await?;
        if !resp.status().is_success() {
            return Err(StreamError::UpstreamStatus(resp.status().as_u16()));
        }
        if resp.content_length().is_some_and(|n| n > MAX_IMAGE_BYTES) {
            return Err(StreamError::ImageTooLarge(MAX_IMAGE_BYTES));
        }
        let mut buf = BytesMut::new();
        while let Some(chunk) = resp.chunk().await? {
            buf.extend_from_slice(&chunk);
            if buf.len() as u64 > MAX_IMAGE_BYTES {
                return Err(StreamError::ImageTooLarge(MAX_IMAGE_BYTES));
            }
        }
        Ok(buf.freeze())
    }

    /// Write to a temp file and rename, so a crash never leaves half an image.
    async fn store(&self, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}

fn check_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|e| StreamError::InvalidUrl(format!("{raw}: {e}")))?;
    if url.scheme() != "https" {
        return Err(StreamError::HostNotAllowed(raw.to_owned()));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let allowed = ALLOWED_HOSTS
        .iter()
        .any(|a| host == *a || host.ends_with(&format!(".{a}")));
    if allowed {
        Ok(url)
    } else {
        Err(StreamError::HostNotAllowed(host))
    }
}

/// `https://lh3.googleusercontent.com/<id>=w60-h60-l90-rj` → `=w<s>-h<s>-l90-rj`.
/// Other hosts (ytimg thumbnails have fixed names) are left alone.
fn resize(mut url: Url, size: u32) -> Url {
    let host = url.host_str().unwrap_or_default();
    if !(host.ends_with("googleusercontent.com") || host.ends_with("ggpht.com")) {
        return url;
    }
    let path = url.path();
    let base = match path.rfind('=') {
        Some(i) if is_size_spec(&path[i + 1..]) => &path[..i],
        _ => path,
    };
    let new_path = format!("{base}=w{size}-h{size}-l90-rj");
    url.set_path(&new_path);
    url
}

/// True for `w120-h120-l90-rj`, `s176-c-k-c0x00ffffff-no-rj` and the like.
fn is_size_spec(s: &str) -> bool {
    !s.is_empty()
        && s.split('-')
            .all(|part| part.chars().all(|c| c.is_ascii_alphanumeric()) && !part.is_empty())
        && s.starts_with(['w', 'h', 's'])
}

fn cache_name(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("{hex}.img")
}

/// Content type from magic bytes; `None` means "not an image we serve".
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("image/jpeg"),
        [0x89, b'P', b'N', b'G', ..] => Some("image/png"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        [b'G', b'I', b'F', b'8', ..] => Some("image/gif"),
        [
            _,
            _,
            _,
            _,
            b'f',
            b't',
            b'y',
            b'p',
            b'a',
            b'v',
            b'i',
            b'f',
            ..,
        ] => Some("image/avif"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist() {
        assert!(check_url("https://lh3.googleusercontent.com/abc=w60-h60").is_ok());
        assert!(check_url("https://i.ytimg.com/vi/x/hqdefault.jpg").is_ok());
        assert!(check_url("https://yt3.ggpht.com/abc=s88").is_ok());
        assert!(check_url("http://i.ytimg.com/vi/x/hq.jpg").is_err());
        assert!(check_url("https://evilytimg.com/x").is_err());
        assert!(check_url("https://ytimg.com.evil.org/x").is_err());
        assert!(check_url("https://127.0.0.1/x").is_err());
        assert!(check_url("not a url").is_err());
    }

    #[test]
    fn resizes_googleusercontent() {
        let url = Url::parse("https://lh3.googleusercontent.com/AbC_d-e=w60-h60-l90-rj").unwrap();
        assert_eq!(
            resize(url, 544).as_str(),
            "https://lh3.googleusercontent.com/AbC_d-e=w544-h544-l90-rj"
        );
        let url = Url::parse("https://yt3.ggpht.com/xyz=s176-c-k-c0x00ffffff-no-rj").unwrap();
        assert_eq!(
            resize(url, 120).as_str(),
            "https://yt3.ggpht.com/xyz=w120-h120-l90-rj"
        );
        let url = Url::parse("https://lh3.googleusercontent.com/plain").unwrap();
        assert_eq!(
            resize(url, 100).as_str(),
            "https://lh3.googleusercontent.com/plain=w100-h100-l90-rj"
        );
        let url = Url::parse("https://i.ytimg.com/vi/x/sddefault.jpg?sqp=1").unwrap();
        assert_eq!(resize(url.clone(), 100), url);
    }

    #[test]
    fn sniffs_formats() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"<html>"), None);
    }

    #[test]
    fn cache_names_are_stable_and_distinct() {
        assert_eq!(cache_name("a"), cache_name("a"));
        assert_ne!(cache_name("a"), cache_name("b"));
        assert_eq!(cache_name("a").len(), 32 + 4);
    }
}
