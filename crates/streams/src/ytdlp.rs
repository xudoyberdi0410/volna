//! Running yt-dlp: finding the binary, resolving a video to an audio URL,
//! reporting its version and self-updating it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::{Result, StreamError};
use crate::model::{
    Codec, Delivery, Quality, ResolvedStream, Tier, VideoId, url_expiry, url_param,
};

const TARGET_TRIPLE: &str = env!("VOLNA_STREAMS_TARGET");

/// How long a resolve may take before we give up on yt-dlp.
pub const DEFAULT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
/// `yt-dlp -U` downloads a ~20 MB binary.
const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// yt-dlp `-f` selector. Opus first: it is what YouTube serves best and
/// WebKit/WebView2 both play WebM/Opus; m4a/AAC is the fallback.
pub fn format_selector(quality: Quality) -> &'static str {
    match (quality.tier, quality.codec) {
        (Tier::High, Codec::PreferOpus) => "bestaudio[acodec=opus]/bestaudio[ext=m4a]/bestaudio",
        (Tier::Low, Codec::PreferOpus) => "worstaudio[acodec=opus]/worstaudio[ext=m4a]/worstaudio",
        (Tier::High, Codec::AacOnly) => "bestaudio[ext=m4a]",
        (Tier::Low, Codec::AacOnly) => "worstaudio[ext=m4a]",
    }
}

/// Where yt-dlp takes YouTube cookies from. From datacenter IPs (a VPS
/// behind a VLESS link) YouTube answers most videos with "Sign in to confirm
/// you're not a bot"; a logged-in browser session gets past it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "lowercase")]
pub enum Cookies {
    #[default]
    None,
    /// `--cookies-from-browser`: a browser name, optionally `name:profile`.
    Browser(String),
    /// `--cookies`: a Netscape cookies.txt (yt-dlp also writes it back).
    File(PathBuf),
}

/// Browsers yt-dlp can read cookies from.
pub const COOKIE_BROWSERS: &[&str] = &[
    "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi", "whale",
];

/// YouTube client tried first when resolving (see [`YtDlp::resolve`]).
const PREFERRED_CLIENT: &str = "web_music";

/// Linux keyrings yt-dlp can take the Chromium cookie key from (`chrome+kwallet6`).
pub const COOKIE_KEYRINGS: &[&str] = &[
    "basictext",
    "gnomekeyring",
    "kwallet",
    "kwallet5",
    "kwallet6",
];

/// Chromium-based browsers encrypt cookies with a key kept in the desktop keyring.
const CHROMIUM_BROWSERS: &[&str] = &[
    "brave", "chrome", "chromium", "edge", "opera", "vivaldi", "whale",
];

/// Splits `name[+keyring][:profile]` (yt-dlp's `--cookies-from-browser` syntax).
fn split_browser_spec(spec: &str) -> (&str, Option<&str>, Option<&str>) {
    let (head, profile) = match spec.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (spec, None),
    };
    let (name, keyring) = match head.split_once('+') {
        Some((n, k)) => (n, Some(k)),
        None => (head, None),
    };
    (name, keyring, profile)
}

/// The keyring holding Chromium's cookie key on this desktop. yt-dlp guesses it
/// from XDG_CURRENT_DESKTOP, which fails outside KDE/GNOME (e.g. Hyprland with
/// KWallet running) and leaves every cookie undecryptable.
#[cfg(target_os = "linux")]
fn detect_keyring() -> Option<&'static str> {
    static KEYRING: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *KEYRING.get_or_init(|| {
        let names = std::process::Command::new("busctl")
            .args(["--user", "list", "--no-legend", "--acquired"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())?;
        let has = |svc: &str| {
            names
                .lines()
                .any(|l| l.split_whitespace().next() == Some(svc))
        };
        if has("org.kde.kwalletd6") {
            Some("kwallet6")
        } else if has("org.kde.kwalletd5") {
            Some("kwallet5")
        } else if has("org.freedesktop.secrets") {
            Some("gnomekeyring")
        } else {
            None
        }
    })
}

#[cfg(not(target_os = "linux"))]
fn detect_keyring() -> Option<&'static str> {
    None
}

/// Adds the detected keyring to a Chromium browser spec that names none.
fn with_keyring(spec: &str, keyring: Option<&str>) -> String {
    let (name, explicit, profile) = split_browser_spec(spec);
    let keyring = match (explicit, keyring) {
        (Some(k), _) => Some(k),
        (None, k) if CHROMIUM_BROWSERS.contains(&name) => k,
        _ => None,
    };
    let mut out = name.to_owned();
    if let Some(k) = keyring {
        out.push('+');
        out.push_str(k);
    }
    if let Some(p) = profile {
        out.push(':');
        out.push_str(p);
    }
    out
}

impl Cookies {
    /// Checks the setting before it is used or saved.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::None => Ok(()),
            Self::Browser(spec) => {
                let (name, keyring, profile) = split_browser_spec(spec);
                if keyring.is_some_and(|k| !COOKIE_KEYRINGS.contains(&k)) {
                    return Err(StreamError::InvalidCookies(format!(
                        "unknown keyring {:?} (expected one of {})",
                        keyring.unwrap_or_default(),
                        COOKIE_KEYRINGS.join(", ")
                    )));
                }
                if !COOKIE_BROWSERS.contains(&name) {
                    return Err(StreamError::InvalidCookies(format!(
                        "unknown browser {name:?} (expected one of {})",
                        COOKIE_BROWSERS.join(", ")
                    )));
                }
                if profile.is_some_and(|p| p.is_empty() || p.contains(['\n', '\r'])) {
                    return Err(StreamError::InvalidCookies(
                        "empty or invalid profile".into(),
                    ));
                }
                Ok(())
            }
            Self::File(path) => {
                if path.is_file() {
                    Ok(())
                } else {
                    Err(StreamError::InvalidCookies(format!(
                        "{} is not a file",
                        path.display()
                    )))
                }
            }
        }
    }

    fn push_args(&self, args: &mut Vec<String>) {
        match self {
            Self::None => {}
            Self::Browser(spec) => {
                args.push("--cookies-from-browser".into());
                args.push(with_keyring(spec, detect_keyring()));
            }
            Self::File(path) => {
                args.push("--cookies".into());
                args.push(path.display().to_string());
            }
        }
    }
}

/// The subset of `yt-dlp -J` output we use.
#[derive(Debug, Deserialize)]
struct YtDlpInfo {
    url: Option<String>,
    format_id: Option<String>,
    ext: Option<String>,
    acodec: Option<String>,
    abr: Option<f64>,
    tbr: Option<f64>,
    filesize: Option<u64>,
    #[serde(default)]
    http_headers: std::collections::HashMap<String, String>,
}

/// Parses `yt-dlp -J` output for a single selected audio format.
pub fn parse_info(json: &[u8], now: SystemTime) -> Result<ResolvedStream> {
    let info: YtDlpInfo =
        serde_json::from_slice(json).map_err(|e| StreamError::BadYtDlpOutput(e.to_string()))?;
    let url = info
        .url
        .ok_or_else(|| StreamError::BadYtDlpOutput("no url (no single format selected?)".into()))?;
    let parsed = url::Url::parse(&url).map_err(|e| StreamError::BadYtDlpOutput(e.to_string()))?;
    let param = |name: &str| url_param(&parsed, name);
    let expires_at = url_expiry(&parsed, now);
    let filesize = info
        .filesize
        .or_else(|| param("clen").and_then(|v| v.parse().ok()));
    let mime = param("mime")
        .filter(|m| m.starts_with("audio/"))
        .unwrap_or_else(|| mime_for_ext(info.ext.as_deref()).to_owned());
    let codec = info.acodec.filter(|c| c != "none");
    let user_agent = info
        .http_headers
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("user-agent"))
        .map(|(_, v)| v);

    Ok(ResolvedStream {
        url,
        delivery: Delivery::Url,
        format_id: info.format_id.unwrap_or_default(),
        mime,
        codec,
        bitrate_kbps: info.abr.or(info.tbr),
        filesize,
        expires_at,
        user_agent,
    })
}

fn mime_for_ext(ext: Option<&str>) -> &'static str {
    match ext {
        Some("webm" | "weba" | "opus") => "audio/webm",
        Some("m4a" | "mp4") => "audio/mp4",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        _ => "application/octet-stream",
    }
}

/// Turns yt-dlp's stderr into a typed error.
fn classify_failure(stderr: &str) -> StreamError {
    let message = stderr
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("no output")
        .trim()
        .to_owned();
    let lower = message.to_ascii_lowercase();
    if lower.contains("confirm you") && lower.contains("not a bot") {
        return StreamError::BotCheck(message);
    }
    let unavailable = [
        "video unavailable",
        "private video",
        "not available",
        "has been removed",
        "does not exist",
        "members-only",
    ];
    if unavailable.iter().any(|m| lower.contains(m)) && !lower.contains("sign in to confirm") {
        StreamError::Unavailable(message)
    } else {
        StreamError::YtDlpFailed(message)
    }
}

/// Where the binary came from; decides whether we may self-update it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Explicit path or `$VOLNA_YTDLP`.
    Configured,
    /// Shipped next to the executable (Tauri `externalBin`).
    Sidecar,
    /// Found on `PATH`: most likely a distro package, updated by its owner.
    System,
}

/// Result of [`YtDlp::self_update`].
#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub binary: PathBuf,
    pub origin: Origin,
    pub version_before: String,
    pub version_after: String,
    pub updated: bool,
    /// yt-dlp's own output, or why no update was attempted.
    pub message: String,
}

/// A located yt-dlp binary.
#[derive(Debug, Clone)]
pub struct YtDlp {
    binary: PathBuf,
    origin: Origin,
    timeout: Duration,
    /// Arguments for the helpers bundled next to the app (see [`helper_args`]).
    helpers: Vec<String>,
    /// Shared by clones, so a change applies to the running service.
    cookies: std::sync::Arc<std::sync::RwLock<Cookies>>,
}

impl YtDlp {
    /// Finds yt-dlp: `explicit`, then `$VOLNA_YTDLP`, then a sidecar next to
    /// the executable (`volna-yt-dlp` as Tauri installs the bundled one, or
    /// `volna-yt-dlp-<triple>`/`yt-dlp-<triple>`), then `yt-dlp` on `PATH`.
    pub fn locate(explicit: Option<&Path>) -> Result<Self> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let (binary, origin) = if let Some(p) = explicit {
            if !p.is_file() {
                return Err(StreamError::YtDlpNotFound);
            }
            (p.to_path_buf(), Origin::Configured)
        } else if let Some(p) = std::env::var_os("VOLNA_YTDLP")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
        {
            (p, Origin::Configured)
        } else {
            find_in(exe_dir.as_deref(), std::env::var_os("PATH").as_deref())
                .ok_or(StreamError::YtDlpNotFound)?
        };
        Ok(Self {
            binary,
            origin,
            timeout: DEFAULT_RESOLVE_TIMEOUT,
            helpers: exe_dir.as_deref().map(helper_args).unwrap_or_default(),
            cookies: Default::default(),
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn cookies(&self) -> Cookies {
        self.cookies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Validates and switches the cookie source for later resolves.
    pub fn set_cookies(&self, cookies: Cookies) -> Result<()> {
        cookies.validate()?;
        *self.cookies.write().unwrap_or_else(|e| e.into_inner()) = cookies;
        Ok(())
    }

    /// The arguments for resolving `id` (without the binary itself);
    /// `player_client` overrides yt-dlp's own choice of YouTube clients.
    pub fn resolve_args(
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
        cookies: &Cookies,
        player_client: Option<&str>,
    ) -> Vec<String> {
        let mut args: Vec<String> = [
            "--ignore-config",
            "--no-playlist",
            "--no-progress",
            "-J",
            "-f",
            format_selector(quality),
        ]
        .map(String::from)
        .into();
        if let Some(proxy) = proxy {
            args.push("--proxy".into());
            args.push(proxy.into());
        }
        cookies.push_args(&mut args);
        if let Some(client) = player_client {
            args.push("--extractor-args".into());
            args.push(format!("youtube:player_client={client}"));
        }
        args.push("--".into());
        args.push(id.watch_url());
        args
    }

    /// Resolves `id` to a googlevideo URL, through `proxy` when set (the
    /// URL is bound to the exit IP, so playback must use the same proxy).
    pub async fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> Result<ResolvedStream> {
        let started = std::time::Instant::now();
        let cookies = self.cookies();
        // yt-dlp's default picks web_embedded, whose URLs googlevideo often
        // refuses with 403 (video-bound PO token experiment). The YouTube
        // Music client works with cookies and a PO token provider, so try it
        // first and fall back to the default when it yields nothing.
        let mut output = self
            .run(
                Self::resolve_args(id, quality, proxy, &cookies, Some(PREFERRED_CLIENT)),
                self.timeout,
            )
            .await?;
        if !output.status.success() {
            tracing::debug!(
                %id,
                "{PREFERRED_CLIENT} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            output = self
                .run(
                    Self::resolve_args(id, quality, proxy, &cookies, None),
                    self.timeout,
                )
                .await?;
        }
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(%id, "yt-dlp failed: {}", stderr.trim());
            return Err(classify_failure(&stderr));
        }
        let stream = parse_info(&output.stdout, SystemTime::now())?;
        tracing::debug!(
            %id,
            format = %stream.format_id,
            mime = %stream.mime,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved"
        );
        Ok(stream)
    }

    pub async fn version(&self) -> Result<String> {
        let output = self
            .run(vec!["--version".into()], Duration::from_secs(30))
            .await?;
        if !output.status.success() {
            return Err(classify_failure(&String::from_utf8_lossy(&output.stderr)));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// Runs `yt-dlp -U` for a bundled or configured binary. A system binary
    /// is left to the package manager; only its version is reported.
    pub async fn self_update(&self, proxy: Option<&str>) -> Result<UpdateReport> {
        let version_before = self.version().await?;
        let mut report = UpdateReport {
            binary: self.binary.clone(),
            origin: self.origin,
            version_after: version_before.clone(),
            version_before,
            updated: false,
            message: String::new(),
        };

        if self.origin == Origin::System {
            report.message =
                "yt-dlp comes from the system (PATH); update it with the package manager".into();
            return Ok(report);
        }

        let mut args = vec!["-U".to_owned()];
        if let Some(proxy) = proxy {
            args.push("--proxy".into());
            args.push(proxy.into());
        }
        let output = self.run(args, UPDATE_TIMEOUT).await?;
        let mut message = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            message.push('\n');
            message.push_str(stderr.trim());
        }
        report.message = message;
        if !output.status.success() {
            return Err(StreamError::YtDlpFailed(report.message));
        }
        report.version_after = self.version().await?;
        report.updated = report.version_after != report.version_before;
        Ok(report)
    }

    /// Writes every cookie of the configured source (all sites, not only
    /// YouTube) to `dest` as a Netscape cookies.txt, for other sign-ins that
    /// ride on the browser's session. Returns `false` when no source is set.
    pub async fn dump_cookies(&self, dest: &Path) -> Result<bool> {
        let cookies = self.cookies();
        if let Cookies::File(path) = &cookies {
            std::fs::copy(path, dest).map_err(StreamError::YtDlpSpawn)?;
            return Ok(true);
        }
        let mut args = Vec::new();
        cookies.push_args(&mut args);
        if args.is_empty() {
            return Ok(false);
        }
        args.push("--cookies".into());
        args.push(dest.display().to_string());
        // Without a URL yt-dlp exits with an error, but only after saving the jar.
        let output = self.run(args, Duration::from_secs(60)).await?;
        if !dest.is_file() {
            return Err(StreamError::YtDlpFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        Ok(true)
    }

    async fn run(&self, args: Vec<String>, timeout: Duration) -> Result<std::process::Output> {
        let mut cmd = Command::new(&self.binary);
        cmd.args(&self.helpers)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = cmd.spawn().map_err(StreamError::YtDlpSpawn)?;
        match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(result) => result.map_err(StreamError::YtDlpSpawn),
            // Dropping the future drops the child, which kills it.
            Err(_) => Err(StreamError::YtDlpTimeout(timeout)),
        }
    }
}

/// yt-dlp arguments for the helpers the Windows installer ships next to the
/// app (on Linux they come from the distro): Deno for YouTube's player
/// JavaScript, and bgutil-pot as the PO token provider in script mode. Its
/// yt-dlp plugin is a zip in `yt-dlp-plugins/` next to the yt-dlp sidecar,
/// where yt-dlp looks by itself.
fn helper_args(exe_dir: &Path) -> Vec<String> {
    let suffix = std::env::consts::EXE_SUFFIX;
    let mut args = Vec::new();
    let deno = exe_dir.join(format!("volna-deno{suffix}"));
    if deno.is_file() {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", deno.display()));
    }
    let pot = exe_dir.join(format!("volna-bgutil-pot{suffix}"));
    if pot.is_file() {
        args.push("--extractor-args".into());
        args.push(format!("youtubepot-bgutilcli:cli_path={}", pot.display()));
    }
    args
}

fn find_in(exe_dir: Option<&Path>, path_var: Option<&OsStr>) -> Option<(PathBuf, Origin)> {
    let suffix = std::env::consts::EXE_SUFFIX;
    // Bundles install the sidecar as `volna-yt-dlp`; a plain `yt-dlp` next to
    // the app is the distro's when the app lives in /usr/bin, so it counts
    // as a system binary (found through PATH), not as ours to self-update.
    let sidecars = [
        format!("volna-yt-dlp{suffix}"),
        format!("volna-yt-dlp-{TARGET_TRIPLE}{suffix}"),
        format!("yt-dlp-{TARGET_TRIPLE}{suffix}"),
    ];
    let next_to_exe = exe_dir
        .into_iter()
        .flat_map(|dir| sidecars.iter().map(move |name| dir.join(name)))
        .map(|p| (p, Origin::Sidecar));
    let on_path = path_var
        .into_iter()
        .flat_map(std::env::split_paths)
        .map(|dir| (dir.join(format!("yt-dlp{suffix}")), Origin::System));
    next_to_exe.chain(on_path).find(|(p, _)| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FALLBACK_URL_LIFETIME;
    use std::time::UNIX_EPOCH;

    const SAMPLE: &str = r#"{
        "id": "dQw4w9WgXcQ",
        "format_id": "251",
        "ext": "webm",
        "acodec": "opus",
        "vcodec": "none",
        "abr": 128.93,
        "tbr": 128.93,
        "filesize": 3433755,
        "url": "https://rr2---sn-x.googlevideo.com/videoplayback?expire=1792193595&itag=251&mime=audio%2Fwebm&clen=3433755&ip=203.0.113.9",
        "http_headers": {"User-Agent": "Mozilla/5.0 test", "Accept": "*/*"}
    }"#;

    #[test]
    fn parses_selected_format() {
        let s = parse_info(SAMPLE.as_bytes(), UNIX_EPOCH).unwrap();
        assert_eq!(s.format_id, "251");
        assert_eq!(s.mime, "audio/webm");
        assert_eq!(s.codec.as_deref(), Some("opus"));
        assert_eq!(s.bitrate_kbps, Some(128.93));
        assert_eq!(s.filesize, Some(3433755));
        assert_eq!(s.expires_at, UNIX_EPOCH + Duration::from_secs(1792193595));
        assert_eq!(s.user_agent.as_deref(), Some("Mozilla/5.0 test"));
    }

    #[test]
    fn falls_back_to_url_params_and_ext() {
        let json = r#"{"ext":"m4a","acodec":"mp4a.40.2","url":"https://x.googlevideo.com/videoplayback?clen=42"}"#;
        let now = UNIX_EPOCH + Duration::from_secs(1000);
        let s = parse_info(json.as_bytes(), now).unwrap();
        assert_eq!(s.mime, "audio/mp4");
        assert_eq!(s.filesize, Some(42));
        assert_eq!(s.expires_at, now + FALLBACK_URL_LIFETIME);
        assert_eq!(s.bitrate_kbps, None);
    }

    #[test]
    fn rejects_output_without_url() {
        let err = parse_info(br#"{"formats":[]}"#, UNIX_EPOCH).unwrap_err();
        assert!(matches!(err, StreamError::BadYtDlpOutput(_)));
        assert!(parse_info(b"not json", UNIX_EPOCH).is_err());
    }

    #[test]
    fn builds_args_with_and_without_proxy() {
        let id = VideoId::parse("dQw4w9WgXcQ").unwrap();
        let direct = YtDlp::resolve_args(&id, Quality::High, None, &Cookies::None, None);
        assert!(!direct.contains(&"--proxy".to_owned()));
        assert_eq!(
            direct.last().unwrap(),
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        let f = direct.iter().position(|a| a == "-f").unwrap();
        assert_eq!(direct[f + 1], format_selector(Quality::High));

        let proxied = YtDlp::resolve_args(
            &id,
            Quality::Low,
            Some("socks5h://u:p@127.0.0.1:1"),
            &Cookies::None,
            None,
        );
        let p = proxied.iter().position(|a| a == "--proxy").unwrap();
        assert_eq!(proxied[p + 1], "socks5h://u:p@127.0.0.1:1");
        assert!(proxied.contains(&format_selector(Quality::Low).to_owned()));
        assert!(!proxied.iter().any(|a| a.starts_with("--cookies")));

        let browser = YtDlp::resolve_args(
            &id,
            Quality::High,
            None,
            &Cookies::Browser("firefox".into()),
            None,
        );
        let c = browser
            .iter()
            .position(|a| a == "--cookies-from-browser")
            .unwrap();
        assert_eq!(browser[c + 1], "firefox");
        // Options stay before the `--` that ends them.
        assert!(c < browser.iter().position(|a| a == "--").unwrap());

        let file = YtDlp::resolve_args(
            &id,
            Quality::High,
            None,
            &Cookies::File("/tmp/c.txt".into()),
            None,
        );
        let c = file.iter().position(|a| a == "--cookies").unwrap();
        assert_eq!(file[c + 1], "/tmp/c.txt");
    }

    #[test]
    fn validates_cookie_settings() {
        assert!(Cookies::None.validate().is_ok());
        assert!(Cookies::Browser("firefox".into()).validate().is_ok());
        assert!(
            Cookies::Browser("chrome+kwallet6:Default".into())
                .validate()
                .is_ok()
        );
        assert!(Cookies::Browser("chrome+wallet".into()).validate().is_err());
        assert!(
            Cookies::Browser("chrome:Profile 1".into())
                .validate()
                .is_ok()
        );
        assert!(Cookies::Browser("netscape".into()).validate().is_err());
        assert!(Cookies::Browser("firefox:".into()).validate().is_err());
        assert!(
            Cookies::File("/definitely/not/here.txt".into())
                .validate()
                .is_err()
        );
        let json = serde_json::to_string(&Cookies::Browser("firefox".into())).unwrap();
        assert_eq!(json, r#"{"mode":"browser","value":"firefox"}"#);
        assert_eq!(
            serde_json::from_str::<Cookies>(r#"{"mode":"none"}"#).unwrap(),
            Cookies::None
        );
    }

    #[test]
    fn cookie_changes_reach_clones() {
        let dir = std::env::temp_dir().join(format!("volna-cookies-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("yt-dlp");
        std::fs::write(&bin, "").unwrap();
        let a = YtDlp::locate(Some(&bin)).unwrap();
        let b = a.clone();
        a.set_cookies(Cookies::Browser("firefox".into())).unwrap();
        assert_eq!(b.cookies(), Cookies::Browser("firefox".into()));
        assert!(a.set_cookies(Cookies::Browser("nope".into())).is_err());
        assert_eq!(b.cookies(), Cookies::Browser("firefox".into()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn classifies_errors() {
        let e = classify_failure("WARNING: x\nERROR: [youtube] abc: Video unavailable\n");
        assert!(matches!(e, StreamError::Unavailable(m) if m.contains("Video unavailable")));
        let e = classify_failure(
            "ERROR: [youtube] abc: Sign in to confirm you're not a bot. This video is not available",
        );
        assert!(matches!(e, StreamError::BotCheck(_)));
        // yt-dlp prints a typographic apostrophe.
        let e =
            classify_failure("ERROR: [youtube] abc: Sign in to confirm you\u{2019}re not a bot.");
        assert!(matches!(e, StreamError::BotCheck(_)));
        assert_eq!(e.kind(), crate::FailureKind::BotCheck);
        assert_eq!(e.http_status(), 403);
        let e = classify_failure("ERROR: [youtube] abc: Requested format is not available");
        assert!(matches!(e, StreamError::Unavailable(_)));
        assert!(matches!(classify_failure(""), StreamError::YtDlpFailed(m) if m == "no output"));
    }

    #[test]
    fn passes_bundled_helpers() {
        let dir = std::env::temp_dir().join(format!("volna-helpers-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(helper_args(&dir).is_empty());

        let suffix = std::env::consts::EXE_SUFFIX;
        let deno = dir.join(format!("volna-deno{suffix}"));
        let pot = dir.join(format!("volna-bgutil-pot{suffix}"));
        std::fs::write(&deno, b"").unwrap();
        std::fs::write(&pot, b"").unwrap();
        assert_eq!(
            helper_args(&dir),
            [
                "--js-runtimes".to_owned(),
                format!("deno:{}", deno.display()),
                "--extractor-args".to_owned(),
                format!("youtubepot-bgutilcli:cli_path={}", pot.display()),
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn finds_sidecar_before_path() {
        let dir = std::env::temp_dir().join(format!("volna-ytdlp-test-{}", std::process::id()));
        let exe_dir = dir.join("app");
        let path_dir = dir.join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&path_dir).unwrap();
        let suffix = std::env::consts::EXE_SUFFIX;
        let on_path = path_dir.join(format!("yt-dlp{suffix}"));
        std::fs::write(&on_path, b"").unwrap();

        let path_var = std::env::join_paths([&path_dir]).unwrap();
        assert_eq!(
            find_in(Some(&exe_dir), Some(&path_var)),
            Some((on_path.clone(), Origin::System))
        );

        std::fs::write(exe_dir.join(format!("yt-dlp{suffix}")), b"").unwrap();
        assert_eq!(
            find_in(Some(&exe_dir), Some(&path_var)),
            Some((on_path, Origin::System)),
            "a plain yt-dlp next to the app is not a sidecar"
        );

        let sidecar = exe_dir.join(format!("yt-dlp-{TARGET_TRIPLE}{suffix}"));
        std::fs::write(&sidecar, b"").unwrap();
        assert_eq!(
            find_in(Some(&exe_dir), Some(&path_var)),
            Some((sidecar, Origin::Sidecar))
        );

        let bundled = exe_dir.join(format!("volna-yt-dlp{suffix}"));
        std::fs::write(&bundled, b"").unwrap();
        assert_eq!(
            find_in(Some(&exe_dir), Some(&path_var)),
            Some((bundled, Origin::Sidecar))
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chromium_specs_get_the_detected_keyring() {
        assert_eq!(with_keyring("chrome", Some("kwallet6")), "chrome+kwallet6");
        assert_eq!(
            with_keyring("brave:Profile 1", Some("gnomekeyring")),
            "brave+gnomekeyring:Profile 1"
        );
        assert_eq!(
            with_keyring("chrome+basictext", Some("kwallet6")),
            "chrome+basictext"
        );
        assert_eq!(with_keyring("firefox", Some("kwallet6")), "firefox");
        assert_eq!(with_keyring("chrome", None), "chrome");
    }
}
