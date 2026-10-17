//! [`volna_player::Hooks`] over the app: streams through the loopback
//! audio server, «Моя волна», YouTube Music radio and the journal.

use std::collections::HashSet;
use std::sync::Arc;

use volna_player::{
    ErrorKind, FailureInfo, Hooks, MoreRequest, Origin, PlayEvent, PlayEventOutcome, PlayerTrack,
    Quality, StreamInfo, TrackInput,
};
use volna_streams::{Codec, FailureKind, VideoId};
use volna_wave::{PlayerEvent as WaveEvent, Wave, YtmSource};
use volna_ytm::YtmClient;

use super::convert::{cover_source, scrobble_track, track_input, ytm_track};
use super::journal::{Journal, Record};
use crate::api::library::Library;
use crate::state::Streams;

pub struct AppHooks {
    pub stream_base: Option<String>,
    /// `AacOnly` adds `codec=aac` to the stream URLs (AVPlayer).
    pub codec: Codec,
    pub streams: Option<Streams>,
    pub wave: Arc<Wave<YtmSource>>,
    pub ytm: YtmClient,
    pub journal: Arc<Journal>,
    /// History is recorded here, not by the window.
    pub library: Arc<Library>,
    /// Now playing on start, scrobbles on end (window or not).
    pub scrobble: volna_scrobble::Scrobbling,
}

fn quality(q: Quality, codec: Codec) -> volna_streams::Quality {
    let tier = match q {
        Quality::High => volna_streams::Quality::High,
        Quality::Low => volna_streams::Quality::Low,
    };
    tier.with_codec(codec)
}

/// `<base>/<id>[?q=low][&codec=aac]`, as `LoopbackServer` parses it.
fn stream_url(base: &str, id: &str, q: Quality, codec: Codec) -> String {
    let mut params = Vec::new();
    if q == Quality::Low {
        params.push("q=low");
    }
    if codec == Codec::AacOnly {
        params.push("codec=aac");
    }
    if params.is_empty() {
        format!("{base}/{id}")
    } else {
        format!("{base}/{id}?{}", params.join("&"))
    }
}

fn wave_error(e: volna_wave::Error) -> String {
    match e {
        volna_wave::Error::NoCandidates => format!("no_candidates: {e}"),
        other => format!("internal: {other}"),
    }
}

impl AppHooks {
    /// Feeds the scrobblers in the background, so the player never waits on
    /// them. They do nothing until a service is turned on in the daemon.
    fn scrobble_event(&self, ev: &PlayEvent) {
        let s = self.scrobble.clone();
        match ev {
            PlayEvent::Start { track, .. } => {
                let t = scrobble_track(track);
                tokio::spawn(async move { s.now_playing(&t).await });
            }
            PlayEvent::End {
                track,
                started_at,
                position,
                duration,
                at,
                ..
            } => {
                let t = scrobble_track(track);
                let (started_at, position, duration, at) = (*started_at, *position, *duration, *at);
                tokio::spawn(async move {
                    s.play_ended(t, started_at, position, duration, at).await;
                });
            }
            _ => {}
        }
    }

    /// YouTube Music radio of `seed`, minus `exclude`, at most `count`.
    async fn radio(
        &self,
        seed: &str,
        exclude: &HashSet<String>,
        count: usize,
        origin: Origin,
    ) -> Result<Vec<TrackInput>, String> {
        let queue = self
            .ytm
            .radio(seed)
            .await
            .map_err(|e| format!("internal: YouTube Music radio: {e}"))?;
        Ok(queue
            .tracks
            .iter()
            .filter(|t| t.playable && t.video_id != seed && !exclude.contains(&t.video_id))
            .take(count)
            .map(|t| track_input(t, origin))
            .collect())
    }
}

impl Hooks for AppHooks {
    fn stream_uri(&self, t: &PlayerTrack, q: Quality) -> Option<String> {
        self.streams.as_ref()?;
        let base = self.stream_base.as_ref()?;
        Some(stream_url(base, &t.id, q, self.codec))
    }

    fn prefetch(&self, t: &PlayerTrack, q: Quality) {
        if let (Some(streams), Ok(id)) = (&self.streams, VideoId::parse(&t.id)) {
            streams.prefetch(id, quality(q, self.codec));
        }
    }

    async fn failure(&self, track_id: &str) -> Option<FailureInfo> {
        let Some(streams) = &self.streams else {
            return Some(FailureInfo {
                kind: ErrorKind::NoYtDlp,
                message: "yt-dlp not found".into(),
            });
        };
        let f = streams.last_failure(&VideoId::parse(track_id).ok()?)?;
        Some(FailureInfo {
            kind: match f.kind {
                FailureKind::BotCheck => ErrorKind::BotCheck,
                FailureKind::Unavailable => ErrorKind::Unavailable,
                FailureKind::NoYtDlp => ErrorKind::NoYtDlp,
                FailureKind::Other => ErrorKind::Other,
            },
            message: f.message,
        })
    }

    async fn stream_info(&self, track_id: &str, q: Quality) -> Option<StreamInfo> {
        let streams = self.streams.as_ref()?;
        let id = VideoId::parse(track_id).ok()?;
        // Cached by the resolve the audio server just did.
        let s = streams.resolve(&id, quality(q, self.codec)).await.ok()?;
        Some(StreamInfo {
            codec: s.codec.as_deref().map(|c| {
                if c.starts_with("mp4a") {
                    "aac".to_owned()
                } else {
                    c.to_owned()
                }
            }),
            bitrate_kbps: s.bitrate_kbps.map(f64::round),
            quality: q,
            mime: Some(s.mime.clone()),
        })
    }

    async fn more(&self, req: MoreRequest) -> Result<Vec<TrackInput>, String> {
        let wave = |tracks: Vec<volna_ytm::Track>| -> Vec<TrackInput> {
            tracks
                .iter()
                .map(|t| track_input(t, Origin::Wave))
                .collect()
        };
        match req {
            MoreRequest::WaveStart { count } => {
                self.wave.start(count).await.map(wave).map_err(wave_error)
            }
            MoreRequest::WaveNext { count } => {
                self.wave.next(count).await.map(wave).map_err(wave_error)
            }
            MoreRequest::WaveRequeue { dropped, count } => self
                .wave
                .requeue(&dropped, count)
                .await
                .map(wave)
                .map_err(wave_error),
            MoreRequest::Radio {
                seeds,
                exclude,
                count,
            } => {
                if seeds.is_empty() {
                    return Err("bad_args: radio without seeds".into());
                }
                // A few random seeds until one gives enough.
                let mut out = Vec::new();
                let mut exclude = exclude;
                for _ in 0..3 {
                    let seed = &seeds[fastrand_index(seeds.len())];
                    let got = self
                        .radio(&seed.id, &exclude, count - out.len(), Origin::Radio)
                        .await?;
                    exclude.extend(got.iter().map(|t| t.id.clone()));
                    out.extend(got);
                    if out.len() >= count {
                        break;
                    }
                }
                Ok(out)
            }
            MoreRequest::Autoplay {
                current,
                exclude,
                count,
            } => {
                self.radio(&current.id, &exclude, count, Origin::Autoplay)
                    .await
            }
        }
    }

    async fn on_play_event(&self, ev: PlayEvent) -> PlayEventOutcome {
        let mut out = PlayEventOutcome::default();
        self.scrobble_event(&ev);
        let wave_ev = match &ev {
            PlayEvent::Start { track, in_wave, .. } => Some(WaveEvent::Start {
                track: ytm_track(track),
                in_wave: *in_wave,
            }),
            PlayEvent::End {
                track,
                position,
                duration,
                ended,
                in_wave,
                ..
            } => Some(WaveEvent::End {
                track: ytm_track(track),
                position_s: position.max(0.0),
                duration_s: duration.filter(|d| d.is_finite() && *d > 0.0),
                ended: *ended,
                in_wave: *in_wave,
            }),
            PlayEvent::Dislike { track, .. } => Some(WaveEvent::Dislike {
                track: ytm_track(track),
                disliked: true,
            }),
            PlayEvent::History { .. } => None,
        };
        if let Some(wave_ev) = wave_ev {
            match self.wave.record(wave_ev).await {
                Ok(r) => out.requeue_wave = r.reseeded,
                Err(e) => tracing::warn!("wave record failed: {e}"),
            }
        }
        match ev {
            PlayEvent::History { track, at } => {
                self.library.record_play(&history_track(&track), at as i64);
                self.journal.append(Record::History { at, track });
            }
            PlayEvent::End {
                track,
                started_at,
                position,
                duration,
                ended,
                in_wave,
                at,
            } => {
                self.journal.append(Record::End {
                    at,
                    track,
                    started_at,
                    position,
                    duration,
                    ended,
                    in_wave,
                });
            }
            _ => {}
        }
        out
    }

    async fn wave_stop(&self) {
        self.wave.stop().await;
    }

    fn journal_seq(&self) -> u64 {
        self.journal.seq()
    }
}

/// A queue entry as a library history row: the wave's view plus the album
/// and the cover the wave does not need.
fn history_track(t: &volna_player::PlayerTrack) -> volna_ytm::Track {
    let mut out = ytm_track(t);
    out.album = t
        .album
        .as_ref()
        .filter(|a| a.id.is_some() || !a.title.is_empty())
        .map(|a| volna_ytm::AlbumRef {
            id: a.id.clone(),
            name: a.title.clone(),
        });
    out.thumbnails = t
        .cover
        .as_deref()
        .and_then(cover_source)
        .map(|url| volna_ytm::Thumbnail {
            url,
            width: 0,
            height: 0,
        })
        .into_iter()
        .collect();
    out
}

fn fastrand_index(n: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    nanos as usize % n.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_urls_carry_tier_and_codec() {
        let b = "http://127.0.0.1:1/t/stream";
        let id = "dQw4w9WgXcQ";
        assert_eq!(stream_url(b, id, Quality::High, Codec::PreferOpus), format!("{b}/{id}"));
        assert_eq!(stream_url(b, id, Quality::Low, Codec::PreferOpus), format!("{b}/{id}?q=low"));
        assert_eq!(stream_url(b, id, Quality::High, Codec::AacOnly), format!("{b}/{id}?codec=aac"));
        assert_eq!(
            stream_url(b, id, Quality::Low, Codec::AacOnly),
            format!("{b}/{id}?q=low&codec=aac")
        );
        // What the audio server makes of them.
        assert_eq!(
            volna_streams::Quality::from_params(Some("low"), Some("aac")),
            quality(Quality::Low, Codec::AacOnly)
        );
    }
}
