//! User-facing network settings, stored in the app settings as JSON.

use std::fmt;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{NetError, Result};
use crate::vless::VlessLink;

/// How the app reaches the internet.
///
/// Serialized adjacently tagged so it reads well in a settings file:
///
/// ```json
/// {"mode": "direct"}
/// {"mode": "url", "value": "socks5h://user:pass@127.0.0.1:1080"}
/// {"mode": "vless", "value": "vless://uuid@host:443?security=reality&..."}
/// ```
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "lowercase")]
pub enum ProxyConfig {
    /// System route (including a system-wide VPN, if one is on).
    #[default]
    Direct,
    /// An existing proxy: `socks5://`, `socks5h://`, `http://` or `https://`,
    /// optionally with `user:pass@`.
    Url(String),
    /// A `vless://` share link, served by a bundled xray process.
    Vless(String),
}

/// Proxy schemes reqwest and yt-dlp both understand.
const PROXY_SCHEMES: &[&str] = &["socks5", "socks5h", "http", "https"];

impl ProxyConfig {
    /// Checks the setting without touching the network or spawning anything.
    pub fn validate(&self) -> Result<()> {
        match self {
            ProxyConfig::Direct => Ok(()),
            ProxyConfig::Url(raw) => parse_proxy_url(raw).map(|_| ()),
            ProxyConfig::Vless(link) => VlessLink::parse(link).map(|_| ()).map_err(Into::into),
        }
    }

    /// Short name of the mode, for status displays.
    pub fn mode(&self) -> &'static str {
        match self {
            ProxyConfig::Direct => "direct",
            ProxyConfig::Url(_) => "url",
            ProxyConfig::Vless(_) => "vless",
        }
    }
}

/// Debug output never shows credentials or the vless UUID.
impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyConfig::Direct => f.write_str("Direct"),
            ProxyConfig::Url(raw) => write!(f, "Url({})", redact_url(raw)),
            ProxyConfig::Vless(link) => match VlessLink::parse(link) {
                Ok(l) => write!(f, "Vless({}:{})", l.address, l.port),
                Err(_) => f.write_str("Vless(<invalid>)"),
            },
        }
    }
}

/// Parses and validates a proxy URL. The returned URL is normalized but keeps
/// the credentials, since reqwest and yt-dlp both need them.
pub(crate) fn parse_proxy_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).map_err(|e| NetError::InvalidProxyUrl(e.to_string()))?;
    if !PROXY_SCHEMES.contains(&url.scheme()) {
        return Err(NetError::InvalidProxyUrl(format!(
            "unsupported scheme `{}` (expected one of {})",
            url.scheme(),
            PROXY_SCHEMES.join(", ")
        )));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(NetError::InvalidProxyUrl("missing host".into()));
    }
    if url.port_or_known_default().is_none() {
        return Err(NetError::InvalidProxyUrl(format!(
            "missing port for {}://",
            url.scheme()
        )));
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() {
        return Err(NetError::InvalidProxyUrl(
            "a proxy URL has no path or query".into(),
        ));
    }
    Ok(url)
}

/// Replaces the password (and user) of a URL with `***` for logs.
pub(crate) fn redact_url(raw: &str) -> String {
    match Url::parse(raw) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _ = url.set_username("***");
            }
            if url.password().is_some() {
                let _ = url.set_password(Some("***"));
            }
            url.to_string()
        }
        Err(_) => "<invalid url>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_round_trip() {
        let cases = [
            (ProxyConfig::Direct, r#"{"mode":"direct"}"#),
            (
                ProxyConfig::Url("socks5h://127.0.0.1:1080".into()),
                r#"{"mode":"url","value":"socks5h://127.0.0.1:1080"}"#,
            ),
            (
                ProxyConfig::Vless("vless://x@h:1".into()),
                r#"{"mode":"vless","value":"vless://x@h:1"}"#,
            ),
        ];
        for (cfg, json) in cases {
            assert_eq!(serde_json::to_string(&cfg).unwrap(), json);
            assert_eq!(serde_json::from_str::<ProxyConfig>(json).unwrap(), cfg);
        }
    }

    #[test]
    fn accepts_supported_proxy_urls() {
        for ok in [
            "socks5://127.0.0.1:1080",
            "socks5h://user:p%40ss@10.0.0.1:1080",
            "http://proxy.local:3128",
            "http://proxy.local",
            "https://user:pass@proxy.example.com:8443/",
        ] {
            assert!(
                ProxyConfig::Url(ok.into()).validate().is_ok(),
                "should accept {ok}"
            );
        }
    }

    #[test]
    fn rejects_bad_proxy_urls() {
        for bad in [
            "",
            "127.0.0.1:1080",
            "ftp://host:21",
            "socks5://:1080",
            "socks5://host",
            "http://host:80/path",
            "socks4://host:1080",
        ] {
            assert!(
                ProxyConfig::Url(bad.into()).validate().is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn debug_hides_secrets() {
        let cfg = ProxyConfig::Url("socks5://alice:hunter2@1.2.3.4:1080".into());
        let shown = format!("{cfg:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("alice"),
            "{shown}"
        );

        let cfg = ProxyConfig::Vless(
            "vless://92cc34c0-f305-4528-9473-28bf84c6862b@example.com:443?security=none".into(),
        );
        let shown = format!("{cfg:?}");
        assert_eq!(shown, "Vless(example.com:443)");
    }
}
