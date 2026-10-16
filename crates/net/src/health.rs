//! "Does YouTube Music work through this exit?"

use std::net::IpAddr;
use std::time::{Duration, Instant};

use reqwest::Client;
use serde::Serialize;
use url::Url;

const YTM_URL: &str = "https://music.youtube.com/";
const IP_URL: &str = "https://api.ipify.org";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// Enough of the page to see `ytcfg` or the region notice.
const BODY_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthReport {
    /// music.youtube.com answered with a success status.
    pub ok: bool,
    /// Time until the YouTube Music response headers arrived.
    pub latency_ms: Option<u64>,
    /// Public IP as seen by api.ipify.org.
    pub exit_ip: Option<String>,
    /// `Some(false)` when YouTube Music says it is not available in the exit's
    /// country, `Some(true)` when the app page loaded, `None` if unclear.
    pub ytm_available: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Targets {
    pub ytm: String,
    pub ip: String,
}

impl Default for Targets {
    fn default() -> Self {
        Self {
            ytm: YTM_URL.into(),
            ip: IP_URL.into(),
        }
    }
}

/// Fetches music.youtube.com and the exit IP in parallel through `client`.
pub async fn health_check(client: &Client) -> HealthReport {
    health_check_with(client, &Targets::default()).await
}

pub(crate) async fn health_check_with(client: &Client, targets: &Targets) -> HealthReport {
    let (ytm, ip) = tokio::join!(
        probe_ytm(client, &targets.ytm),
        exit_ip(client, &targets.ip)
    );
    match ytm {
        Ok(p) => HealthReport {
            ok: p.success,
            latency_ms: Some(p.latency_ms),
            exit_ip: ip,
            ytm_available: p.available,
            error: (!p.success).then(|| format!("music.youtube.com answered HTTP {}", p.status)),
        },
        Err(e) => HealthReport {
            ok: false,
            latency_ms: None,
            exit_ip: ip,
            ytm_available: None,
            error: Some(error_chain(&e)),
        },
    }
}

struct YtmProbe {
    success: bool,
    status: u16,
    latency_ms: u64,
    available: Option<bool>,
}

async fn probe_ytm(client: &Client, url: &str) -> reqwest::Result<YtmProbe> {
    let started = Instant::now();
    let mut resp = client
        .get(url)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header(reqwest::header::ACCEPT, "text/html")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await?;
    let latency_ms = started.elapsed().as_millis() as u64;
    let status = resp.status();
    let final_url = resp.url().clone();

    let mut body = Vec::new();
    while body.len() < BODY_LIMIT {
        match resp.chunk().await? {
            Some(chunk) => body.extend_from_slice(&chunk),
            None => break,
        }
    }
    let body = String::from_utf8_lossy(&body);

    Ok(YtmProbe {
        success: status.is_success(),
        status: status.as_u16(),
        latency_ms,
        available: ytm_availability(&final_url, &body),
    })
}

/// Reads the region state from where YouTube Music sent us and what it sent.
pub(crate) fn ytm_availability(final_url: &Url, body: &str) -> Option<bool> {
    let host = final_url.host_str().unwrap_or_default();
    // Unsupported regions are redirected to a "coming soon" landing page.
    if host == "music.youtube.com" && final_url.path().starts_with("/coming-soon") {
        return Some(false);
    }
    let lower = body.to_ascii_lowercase();
    if [
        "not available in your country",
        "isn't available in your country",
        "not available in your region",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return Some(false);
    }
    if host == "music.youtube.com" && (body.contains("INNERTUBE_API_KEY") || body.contains("ytcfg"))
    {
        return Some(true);
    }
    // e.g. the EU consent interstitial on consent.youtube.com.
    None
}

async fn exit_ip(client: &Client, url: &str) -> Option<String> {
    let text = client
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .text()
        .await
        .ok()?;
    let ip: IpAddr = text.trim().parse().ok()?;
    Some(ip.to_string())
}

/// `reqwest::Error`'s Display hides the cause (e.g. "connection refused").
fn error_chain(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(": ");
        out.push_str(&s.to_string());
        source = s.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn test_client() -> Client {
        Client::builder().no_proxy().build().unwrap()
    }

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn detects_region_block() {
        assert_eq!(
            ytm_availability(&url("https://music.youtube.com/coming-soon/"), ""),
            Some(false)
        );
        assert_eq!(
            ytm_availability(
                &url("https://music.youtube.com/"),
                "<p>YouTube Music isn't available in your country yet</p>"
            ),
            Some(false)
        );
    }

    #[test]
    fn detects_working_app() {
        let body = r#"<script>ytcfg.set({"INNERTUBE_API_KEY":"x"})</script>"#;
        assert_eq!(
            ytm_availability(&url("https://music.youtube.com/"), body),
            Some(true)
        );
    }

    #[test]
    fn consent_page_is_unknown() {
        assert_eq!(
            ytm_availability(&url("https://consent.youtube.com/m?continue=x"), "<form>"),
            None
        );
    }

    /// Serves `/ytm` (with a ytcfg body) and `/ip` (a plain IP) over HTTP.
    async fn fake_server(ytm_status: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let (status, body) = if req.starts_with("GET /ip") {
                        (200, "198.51.100.7\n".to_string())
                    } else {
                        (ytm_status, "<script>ytcfg.set({})</script>".to_string())
                    };
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn reports_success_and_exit_ip() {
        let base = fake_server(200).await;
        let targets = Targets {
            ytm: format!("{base}/ytm"),
            ip: format!("{base}/ip"),
        };
        let report = health_check_with(&test_client(), &targets).await;
        assert!(report.ok, "{report:?}");
        assert!(report.latency_ms.is_some());
        assert_eq!(report.exit_ip.as_deref(), Some("198.51.100.7"));
        // The fake host isn't music.youtube.com, so availability is unknown.
        assert_eq!(report.ytm_available, None);
        assert_eq!(report.error, None);
    }

    #[tokio::test]
    async fn reports_http_errors() {
        let base = fake_server(503).await;
        let targets = Targets {
            ytm: format!("{base}/ytm"),
            ip: format!("{base}/ip"),
        };
        let report = health_check_with(&test_client(), &targets).await;
        assert!(!report.ok);
        assert!(report.error.unwrap().contains("503"));
    }

    #[tokio::test]
    async fn reports_connection_errors() {
        let port = crate::xray::free_local_port().unwrap();
        let targets = Targets {
            ytm: format!("http://127.0.0.1:{port}/"),
            ip: format!("http://127.0.0.1:{port}/ip"),
        };
        let report = health_check_with(&test_client(), &targets).await;
        assert!(!report.ok);
        assert_eq!(report.latency_ms, None);
        assert_eq!(report.exit_ip, None);
        assert!(report.error.is_some());
    }
}
