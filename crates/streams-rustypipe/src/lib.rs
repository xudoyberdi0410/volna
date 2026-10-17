//! [`RustyPipeBackend`]: resolves in-process through RustyPipe's InnerTube
//! player requests and signature deobfuscation, no yt-dlp binary.
//!
//! RustyPipe brings its own reqwest (a newer major than the rest of the app),
//! so it cannot share [`volna_net::NetManager`]'s client. It gets the same
//! exit instead: the proxy URL the resolver is called with is the one
//! `NetManager` hands yt-dlp, and a RustyPipe instance is built per proxy.
//!
//! GPL-3.0 because of RustyPipe: this crate is excluded from the workspace
//! and never linked into the app. It exists for the phase 5 spike and for
//! re-running it (docs/streams.md).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use rustypipe::client::{ClientType, RustyPipe};
use rustypipe::error::{Error as RpError, ExtractionError};
use rustypipe::model::{AudioCodec, AudioFormat, AudioStream};

use volna_streams::{
    Backend, Codec, Delivery, Quality, ResolvedStream, Result, StreamError, Tier, VideoId,
};

/// `expire` of a googlevideo URL, if it carries one.
fn url_expiry(url: &url::Url) -> Option<SystemTime> {
    url.query_pairs()
        .find(|(k, _)| k == "expire")
        .and_then(|(_, v)| v.parse::<u64>().ok())
        .map(|secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// Clients tried in order. From the phase 5 spike (docs/streams.md): only
/// the Android VR client returned plain URLs googlevideo served without a
/// PO token; the web and TV clients need the signature deobfuscator, which
/// failed on the current player JS, and Android answered SABR-only formats.
pub const DEFAULT_CLIENTS: &[ClientType] = &[ClientType::AndroidVr, ClientType::Ios];

/// How long a player request may take.
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct RustyPipeOptions {
    /// Where RustyPipe keeps its cache (deobfuscation data, visitor data).
    /// `None` keeps it in memory only.
    pub storage_dir: Option<PathBuf>,
    /// Player clients, tried in order.
    pub clients: Vec<ClientType>,
}

impl Default for RustyPipeOptions {
    fn default() -> Self {
        Self {
            storage_dir: None,
            clients: DEFAULT_CLIENTS.to_vec(),
        }
    }
}

pub struct RustyPipeBackend {
    opts: RustyPipeOptions,
    /// The instance for the last proxy seen; rebuilt when the proxy changes.
    current: Mutex<Option<(Option<String>, RustyPipe)>>,
}

impl std::fmt::Debug for RustyPipeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustyPipeBackend")
            .field("opts", &self.opts)
            .finish_non_exhaustive()
    }
}

impl RustyPipeBackend {
    pub fn new(opts: RustyPipeOptions) -> Self {
        Self {
            opts,
            current: Mutex::new(None),
        }
    }

    /// The RustyPipe instance that talks through `proxy`.
    fn client(&self, proxy: Option<&str>) -> Result<RustyPipe> {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((p, rp)) = current.as_ref()
            && p.as_deref() == proxy
        {
            return Ok(rp.clone());
        }
        let mut http = reqwest::Client::builder();
        if let Some(proxy) = proxy {
            let proxy = reqwest::Proxy::all(proxy)
                .map_err(|e| StreamError::InvalidUrl(format!("proxy: {e}")))?;
            http = http.proxy(proxy);
        } else {
            http = http.no_proxy();
        }
        let mut builder = RustyPipe::builder().no_botguard().timeout(TIMEOUT);
        builder = match &self.opts.storage_dir {
            Some(dir) => builder.storage_dir(dir),
            None => builder.no_storage(),
        };
        let rp = builder
            .build_with_client(http)
            .map_err(|e| StreamError::ResolveFailed(e.to_string()))?;
        *current = Some((proxy.map(str::to_owned), rp.clone()));
        Ok(rp)
    }

    pub async fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> Result<ResolvedStream> {
        let started = std::time::Instant::now();
        let rp = self.client(proxy)?;
        let query = rp.query().unauthenticated();
        let player = query
            .player_from_clients(id.as_str(), &self.opts.clients)
            .await
            .map_err(classify)?;
        let stream = select(&player.audio_streams, quality).ok_or_else(|| {
            StreamError::NoAudioStream(format!(
                "{id}: no {:?} stream among {} from {:?}",
                quality.codec,
                player.audio_streams.len(),
                player.client_type
            ))
        })?;
        let url = stream.url.clone().unwrap_or_default();
        let parsed = url::Url::parse(&url).map_err(|e| StreamError::InvalidUrl(e.to_string()))?;
        let now = SystemTime::now();
        let valid_until = now + Duration::from_secs(player.expires_in_seconds.into());
        let expires_at = url_expiry(&parsed).map_or(valid_until, |e| e.min(valid_until));
        let resolved = ResolvedStream {
            url,
            delivery: Delivery::Url,
            format_id: stream.itag.to_string(),
            mime: stream
                .mime
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned(),
            codec: Some(codec_name(stream)),
            bitrate_kbps: Some(f64::from(stream.average_bitrate.max(1)) / 1000.0),
            filesize: Some(stream.size).filter(|s| *s > 0),
            expires_at,
            user_agent: Some(query.user_agent(player.client_type).into_owned()),
        };
        tracing::debug!(
            %id,
            client = ?player.client_type,
            format = %resolved.format_id,
            mime = %resolved.mime,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "resolved via rustypipe"
        );
        Ok(resolved)
    }
}

impl Backend for RustyPipeBackend {
    fn resolve(
        &self,
        id: &VideoId,
        quality: Quality,
        proxy: Option<&str>,
    ) -> impl std::future::Future<Output = Result<ResolvedStream>> + Send {
        RustyPipeBackend::resolve(self, id, quality, proxy)
    }
}

fn codec_name(s: &AudioStream) -> String {
    match s.codec {
        AudioCodec::Opus => "opus".into(),
        AudioCodec::Mp4a => s
            .mime
            .split("codecs=")
            .nth(1)
            .map(|c| c.trim_matches(|c| c == '"' || c == ' ').to_owned())
            .unwrap_or_else(|| "mp4a.40.2".into()),
        other => format!("{other:?}").to_ascii_lowercase(),
    }
}

/// Picks the stream for `quality` among plain-URL audio streams.
fn select(streams: &[AudioStream], quality: Quality) -> Option<&AudioStream> {
    // Streams without a URL are SABR-only (not supported yet).
    let usable = |s: &&AudioStream| s.url.as_deref().is_some_and(|u| !u.is_empty());
    let pick = |codec: AudioCodec, format: AudioFormat| {
        let candidates = streams
            .iter()
            .filter(usable)
            .filter(move |s| s.codec == codec && s.format == format);
        match quality.tier {
            Tier::High => candidates.max_by_key(|s| s.bitrate),
            Tier::Low => candidates.min_by_key(|s| s.bitrate),
        }
    };
    match quality.codec {
        Codec::PreferOpus => pick(AudioCodec::Opus, AudioFormat::Webm)
            .or_else(|| pick(AudioCodec::Mp4a, AudioFormat::M4a)),
        Codec::AacOnly => pick(AudioCodec::Mp4a, AudioFormat::M4a),
    }
}

/// Maps RustyPipe's errors onto the categories the UI explains.
fn classify(e: RpError) -> StreamError {
    let message = e.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("not a bot") || lower.contains("captcha") || lower.contains("vpn") {
        return StreamError::BotCheck(message);
    }
    match e {
        RpError::Extraction(ExtractionError::Unavailable { .. })
        | RpError::Extraction(ExtractionError::NotFound { .. }) => {
            StreamError::Unavailable(message)
        }
        _ => StreamError::ResolveFailed(message),
    }
}
