//! [`NetManager`]: the one HTTP client the app uses, rebuilt whenever the
//! user changes the network setting.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use reqwest::{Client, Proxy};
use serde::Serialize;

use crate::config::{ProxyConfig, parse_proxy_url, redact_url};
use crate::error::{NetError, Result};
use crate::health::{self, HealthReport};
use crate::vless::{SocksInbound, VlessLink};
use crate::xray::{self, LogBuffer, XrayProcess};

/// Desktop Chrome on Windows: what YouTube serves the full web app to.
pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// Attempts to start xray when the chosen port is taken in the meantime.
const XRAY_START_ATTEMPTS: usize = 3;

#[derive(Debug, Clone)]
pub struct NetOptions {
    /// Explicit xray binary; `None` searches next to the executable and on PATH.
    pub xray_binary: Option<PathBuf>,
    /// How long xray may take to open its SOCKS port.
    pub xray_startup_timeout: Duration,
    pub user_agent: String,
    pub connect_timeout: Duration,
    /// Max silence between bytes. There is no total timeout, because audio
    /// streams are long downloads; set per-request timeouts where needed.
    pub read_timeout: Duration,
}

impl Default for NetOptions {
    fn default() -> Self {
        Self {
            xray_binary: None,
            xray_startup_timeout: Duration::from_secs(10),
            user_agent: DEFAULT_USER_AGENT.into(),
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
        }
    }
}

/// Snapshot of the active setting, safe to show in the UI.
#[derive(Debug, Clone, Serialize)]
pub struct NetStatus {
    pub mode: &'static str,
    /// Proxy URL with credentials masked.
    pub proxy: Option<String>,
    pub xray_running: Option<bool>,
    pub xray_pid: Option<u32>,
}

struct Active {
    config: ProxyConfig,
    client: Client,
    proxy_url: Option<String>,
    xray: Option<XrayProcess>,
}

/// Owns the shared [`reqwest::Client`] and, in vless mode, the xray process.
///
/// Readers ([`client`](Self::client), [`proxy_url`](Self::proxy_url)) never
/// block on a switch: [`apply`](Self::apply) builds the new client (and starts
/// the new xray) first, then swaps it in, and only then stops the old xray.
/// Requests already in flight on the old client may fail at that moment.
pub struct NetManager {
    opts: NetOptions,
    logs: LogBuffer,
    active: RwLock<Arc<Active>>,
    switching: tokio::sync::Mutex<()>,
}

impl NetManager {
    /// Starts in [`ProxyConfig::Direct`]; call [`apply`](Self::apply) with the
    /// saved setting afterwards.
    pub fn new(opts: NetOptions) -> Result<Self> {
        let client = build_client(&opts, None)?;
        Ok(Self {
            opts,
            logs: LogBuffer::default(),
            active: RwLock::new(Arc::new(Active {
                config: ProxyConfig::Direct,
                client,
                proxy_url: None,
                xray: None,
            })),
            switching: tokio::sync::Mutex::new(()),
        })
    }

    /// Switches to `config`. On error the previous setting stays active.
    pub async fn apply(&self, config: ProxyConfig) -> Result<()> {
        let _guard = self.switching.lock().await;
        tracing::info!(?config, "applying network setting");

        let next = match &config {
            ProxyConfig::Direct => Active {
                client: build_client(&self.opts, None)?,
                config,
                proxy_url: None,
                xray: None,
            },
            ProxyConfig::Url(raw) => {
                let url = parse_proxy_url(raw)?.to_string();
                Active {
                    client: build_client(&self.opts, Some(&url))?,
                    config,
                    proxy_url: Some(url),
                    xray: None,
                }
            }
            ProxyConfig::Vless(link) => {
                let link = VlessLink::parse(link)?;
                let (xray, url) = self.start_xray(&link).await?;
                Active {
                    client: build_client(&self.opts, Some(&url))?,
                    config,
                    proxy_url: Some(url),
                    xray: Some(xray),
                }
            }
        };

        let previous = std::mem::replace(&mut *self.write(), Arc::new(next));
        stop(previous).await;
        Ok(())
    }

    /// Starts xray for `link` and returns it with the `socks5h://` URL of its
    /// inbound (remote DNS, so names resolve on the server side).
    async fn start_xray(&self, link: &VlessLink) -> Result<(XrayProcess, String)> {
        let binary = xray::find_xray(self.opts.xray_binary.as_deref())?;
        let mut last_err = None;
        for _ in 0..XRAY_START_ATTEMPTS {
            let inbound = SocksInbound {
                port: xray::free_local_port()?,
                user: "volna".into(),
                pass: random_token(),
            };
            let config = link.xray_config(&inbound);
            match XrayProcess::spawn(
                &binary,
                &config,
                inbound.port,
                &self.logs,
                self.opts.xray_startup_timeout,
            )
            .await
            {
                Ok(proc) => {
                    let url = format!(
                        "socks5h://{}:{}@127.0.0.1:{}",
                        inbound.user, inbound.pass, inbound.port
                    );
                    return Ok((proc, url));
                }
                // Most likely the port was taken between reserving and binding.
                Err(e @ NetError::XrayExited { .. }) => {
                    tracing::warn!("xray failed to start, retrying: {e}");
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.expect("at least one attempt was made"))
    }

    /// The current client. Cheap to clone; hold it for one logical operation
    /// and fetch it again afterwards, so a switch is picked up.
    pub fn client(&self) -> Client {
        self.read().client.clone()
    }

    /// The proxy URL for tools that need it (yt-dlp `--proxy`), with
    /// credentials. `None` in direct mode.
    pub fn proxy_url(&self) -> Option<String> {
        self.read().proxy_url.clone()
    }

    pub fn config(&self) -> ProxyConfig {
        self.read().config.clone()
    }

    pub fn status(&self) -> NetStatus {
        let active = self.read();
        NetStatus {
            mode: active.config.mode(),
            proxy: active.proxy_url.as_deref().map(redact_url),
            xray_running: active.xray.as_ref().map(XrayProcess::is_running),
            xray_pid: active.xray.as_ref().and_then(XrayProcess::pid),
        }
    }

    /// Recent xray output, oldest first, across restarts.
    pub fn xray_logs(&self) -> Vec<String> {
        self.logs.lines()
    }

    /// Checks YouTube Music reachability through the current client.
    pub async fn health_check(&self) -> HealthReport {
        health::health_check(&self.client()).await
    }

    /// Switches to direct mode and waits for xray to exit. Call on app exit.
    pub async fn shutdown(&self) {
        if let Err(e) = self.apply(ProxyConfig::Direct).await {
            tracing::warn!("network shutdown: {e}");
        }
    }

    fn read(&self) -> Arc<Active> {
        self.active
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Arc<Active>> {
        self.active.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// Stops the old xray once nothing else holds the old state. A reader that
/// still holds the `Arc` keeps it alive briefly; `kill_on_drop` then ends it.
async fn stop(previous: Arc<Active>) {
    if let Ok(Active {
        xray: Some(proc), ..
    }) = Arc::try_unwrap(previous)
    {
        proc.shutdown().await;
    }
}

fn build_client(opts: &NetOptions, proxy: Option<&str>) -> Result<Client> {
    let builder = Client::builder()
        .user_agent(&opts.user_agent)
        .gzip(true)
        .brotli(true)
        .connect_timeout(opts.connect_timeout)
        .read_timeout(opts.read_timeout)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .use_rustls_tls();
    let builder = match proxy {
        Some(url) => {
            builder.proxy(Proxy::all(url).map_err(|e| NetError::InvalidProxyUrl(e.to_string()))?)
        }
        // Direct means the system route, not $HTTP_PROXY.
        None => builder.no_proxy(),
    };
    Ok(builder.build()?)
}

/// 32 hex chars from the std per-process random hash keys. Only guards the
/// local SOCKS port against other local users, so this is enough.
fn random_token() -> String {
    (0..2)
        .map(|_| {
            let mut h = RandomState::new().build_hasher();
            h.write_u64(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or_default(),
            );
            format!("{:016x}", h.finish())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn switches_between_direct_and_url() {
        let net = NetManager::new(NetOptions::default()).unwrap();
        assert_eq!(net.config(), ProxyConfig::Direct);
        assert_eq!(net.proxy_url(), None);

        net.apply(ProxyConfig::Url("socks5h://u:p@127.0.0.1:1080".into()))
            .await
            .unwrap();
        assert_eq!(
            net.proxy_url().as_deref(),
            Some("socks5h://u:p@127.0.0.1:1080")
        );
        let status = net.status();
        assert_eq!(status.mode, "url");
        assert_eq!(
            status.proxy.as_deref(),
            Some("socks5h://***:***@127.0.0.1:1080")
        );
        assert_eq!(status.xray_running, None);

        net.apply(ProxyConfig::Direct).await.unwrap();
        assert_eq!(net.proxy_url(), None);
    }

    #[tokio::test]
    async fn failed_apply_keeps_previous_setting() {
        let net = NetManager::new(NetOptions::default()).unwrap();
        let good = ProxyConfig::Url("http://127.0.0.1:3128".into());
        net.apply(good.clone()).await.unwrap();

        assert!(matches!(
            net.apply(ProxyConfig::Url("ftp://x:1".into())).await,
            Err(NetError::InvalidProxyUrl(_))
        ));
        assert!(matches!(
            net.apply(ProxyConfig::Vless("vless://nope".into())).await,
            Err(NetError::InvalidVlessLink(_))
        ));
        assert_eq!(net.config(), good);
    }

    #[tokio::test]
    async fn missing_xray_binary_is_reported() {
        let net = NetManager::new(NetOptions {
            xray_binary: Some("/nonexistent/xray".into()),
            ..NetOptions::default()
        })
        .unwrap();
        let link = "vless://92cc34c0-f305-4528-9473-28bf84c6862b@203.0.113.1:443";
        assert!(matches!(
            net.apply(ProxyConfig::Vless(link.into())).await,
            Err(NetError::XrayNotFound)
        ));
        assert_eq!(net.config(), ProxyConfig::Direct);
    }

    #[test]
    fn random_tokens_differ() {
        let a = random_token();
        assert_eq!(a.len(), 32);
        assert_ne!(a, random_token());
    }
}
