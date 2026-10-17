//! A logged-in YouTube session as cookies, and the `Authorization` header
//! (SAPISIDHASH) InnerTube wants alongside them.

use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};

/// One cookie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cookie {
    /// As stored: `.youtube.com` (domain cookie) or `music.youtube.com` (host-only).
    pub domain: String,
    pub name: String,
    pub value: String,
}

impl Cookie {
    pub fn new(
        domain: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            domain: domain.into(),
            name: name.into(),
            value: value.into(),
        }
    }

    /// Whether the cookie is sent to `host` (path and `Secure` are ignored:
    /// every request here is `https://<host>/...`).
    fn matches(&self, host: &str) -> bool {
        let domain = self.domain.trim_start_matches('.');
        host.eq_ignore_ascii_case(domain)
            || (host.len() > domain.len()
                && host[host.len() - domain.len()..].eq_ignore_ascii_case(domain)
                && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
    }
}

/// The cookies of a browser session (only the Google/YouTube ones are used).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new(cookies: impl IntoIterator<Item = Cookie>) -> Self {
        Self {
            cookies: cookies.into_iter().collect(),
        }
    }

    /// Parses a Netscape `cookies.txt` (what yt-dlp `--cookies` writes and
    /// browser extensions export). Expired cookies are dropped.
    pub fn from_netscape(text: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cookies = text
            .lines()
            .filter_map(|line| {
                let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
                if line.starts_with('#') || line.trim().is_empty() {
                    return None;
                }
                let f: Vec<&str> = line.split('\t').collect();
                if f.len() < 7 {
                    return None;
                }
                let expires: u64 = f[4].parse().unwrap_or(0);
                if expires != 0 && expires < now {
                    return None;
                }
                Some(Cookie::new(f[0], f[5], f[6].trim_end_matches('\r')))
            })
            .collect();
        Self { cookies }
    }

    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// The first cookie called `name` that `host` gets.
    pub fn get(&self, host: &str, name: &str) -> Option<&str> {
        self.cookies
            .iter()
            .find(|c| c.name == name && c.matches(host))
            .map(|c| c.value.as_str())
    }

    /// `Cookie` header value for a request to `host`, if any cookie applies.
    pub fn header_for(&self, host: &str) -> Option<String> {
        let mut seen = std::collections::HashSet::new();
        let pairs: Vec<String> = self
            .cookies
            .iter()
            .filter(|c| c.matches(host) && seen.insert(c.name.as_str()))
            .map(|c| format!("{}={}", c.name, c.value))
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }

    /// Logged in: `LOGIN_INFO` plus one of the SAPISID cookies, as yt-dlp checks.
    pub fn is_logged_in(&self) -> bool {
        let host = "www.youtube.com";
        self.get(host, "LOGIN_INFO").is_some()
            && (self.get(host, "SAPISID").is_some()
                || self.get(host, "__Secure-1PAPISID").is_some()
                || self.get(host, "__Secure-3PAPISID").is_some())
    }

    /// `SAPISIDHASH ... SAPISID1PHASH ... SAPISID3PHASH ...` for a request
    /// from `origin` (`https://music.youtube.com`), at unix time `now`.
    pub fn authorization(
        &self,
        origin: &str,
        user_session_id: Option<&str>,
        now: u64,
    ) -> Option<String> {
        let host = "www.youtube.com";
        let three = self.get(host, "__Secure-3PAPISID");
        let sapisid = self.get(host, "SAPISID").or(three);
        let one = self.get(host, "__Secure-1PAPISID");
        let parts: Vec<String> = [
            ("SAPISIDHASH", sapisid),
            ("SAPISID1PHASH", one),
            ("SAPISID3PHASH", three),
        ]
        .into_iter()
        .filter_map(|(scheme, sid)| Some(sid_hash(scheme, sid?, origin, user_session_id, now)))
        .collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

/// `<scheme> <ts>_<sha1("[u ]ts sid origin")>[_u]`, as the web client builds it.
fn sid_hash(scheme: &str, sid: &str, origin: &str, user: Option<&str>, now: u64) -> String {
    let mut input = String::new();
    if let Some(u) = user {
        input.push_str(u);
        input.push(' ');
    }
    input.push_str(&format!("{now} {sid} {origin}"));
    let hash = Sha1::digest(input.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    match user {
        Some(_) => format!("{scheme} {now}_{hex}_u"),
        None => format!("{scheme} {now}_{hex}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JAR: &str = "# Netscape HTTP Cookie File\n\
        .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tabc/def\n\
        #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t0\tLOGIN_INFO\tlogin\n\
        .youtube.com\tTRUE\t/\tTRUE\t0\t__Secure-3PAPISID\tthree\n\
        music.youtube.com\tFALSE\t/\tTRUE\t0\tHOSTONLY\tm\n\
        .google.com\tTRUE\t/\tTRUE\t0\tSID\tg\n\
        .youtube.com\tTRUE\t/\tTRUE\t1\tOLD\texpired\n";

    #[test]
    fn parses_netscape_and_matches_hosts() {
        let jar = CookieJar::from_netscape(JAR);
        assert_eq!(jar.len(), 5, "expired cookie dropped");
        assert_eq!(jar.get("music.youtube.com", "SAPISID"), Some("abc/def"));
        assert_eq!(jar.get("music.youtube.com", "HOSTONLY"), Some("m"));
        assert_eq!(jar.get("www.youtube.com", "HOSTONLY"), None);
        assert_eq!(jar.get("notyoutube.com", "SAPISID"), None);
        let header = jar.header_for("music.youtube.com").unwrap();
        assert!(header.contains("SAPISID=abc/def"));
        assert!(header.contains("LOGIN_INFO=login"));
        assert!(!header.contains("SID=g"));
        assert!(jar.is_logged_in());
        assert!(!CookieJar::default().is_logged_in());
    }

    #[test]
    fn sapisidhash_matches_the_web_client() {
        // sha1("1700000000 abc/def https://music.youtube.com")
        let jar = CookieJar::from_netscape(JAR);
        let auth = jar
            .authorization("https://music.youtube.com", None, 1_700_000_000)
            .unwrap();
        let mut parts = auth.split(' ');
        assert_eq!(parts.next(), Some("SAPISIDHASH"));
        let expected: String = Sha1::digest(b"1700000000 abc/def https://music.youtube.com")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            parts.next(),
            Some(format!("1700000000_{expected}").as_str())
        );
        assert_eq!(parts.next(), Some("SAPISID3PHASH"));

        let with_user = jar
            .authorization("https://music.youtube.com", Some("123"), 1_700_000_000)
            .unwrap();
        assert!(with_user.split(' ').nth(1).unwrap().ends_with("_u"));
    }
}
