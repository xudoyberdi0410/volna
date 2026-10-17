//! What the player needs from the app (streams, wave, radio, journal).
//! Implemented in `src-tauri/src/player/hooks.rs`; tests use a fake.

use std::collections::HashSet;
use std::future::Future;

use crate::model::{ErrorKind, PlayerTrack, Quality, StreamInfo, TrackInput};
use crate::tracker::PlayEvent;

/// Why the last attempt to play a track failed.
#[derive(Debug, Clone, PartialEq)]
pub struct FailureInfo {
    pub kind: ErrorKind,
    pub message: String,
}

/// A request for more tracks at the end of the queue.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum MoreRequest {
    WaveStart {
        count: usize,
    },
    WaveNext {
        count: usize,
    },
    /// After a re-seed: `dropped` are the stale wave tracks taken out of the queue.
    WaveRequeue {
        dropped: Vec<String>,
        count: usize,
    },
    Radio {
        seeds: Vec<PlayerTrack>,
        exclude: HashSet<String>,
        count: usize,
    },
    Autoplay {
        current: PlayerTrack,
        exclude: HashSet<String>,
        count: usize,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlayEventOutcome {
    /// The wave re-seeded: queued wave tracks are stale.
    pub requeue_wave: bool,
}

pub trait Hooks: Send + Sync + 'static {
    /// URI a deck plays `track` from, `None` when playback is impossible
    /// (no yt-dlp, no audio server).
    fn stream_uri(&self, track: &PlayerTrack, quality: Quality) -> Option<String>;
    fn prefetch(&self, track: &PlayerTrack, quality: Quality);
    /// Why `track_id` failed to load, if the stream layer knows.
    fn failure(&self, track_id: &str) -> impl Future<Output = Option<FailureInfo>> + Send;
    fn stream_info(
        &self,
        track_id: &str,
        quality: Quality,
    ) -> impl Future<Output = Option<StreamInfo>> + Send;
    /// Tracks to append (wave, radio, autoplay). Errors are `"<code>: <message>"`.
    fn more(&self, req: MoreRequest)
    -> impl Future<Output = Result<Vec<TrackInput>, String>> + Send;
    /// Called in order, one at a time (wave.record and the journal).
    fn on_play_event(&self, ev: PlayEvent) -> impl Future<Output = PlayEventOutcome> + Send;
    fn wave_stop(&self) -> impl Future<Output = ()> + Send;
    /// Highest journal seq written (for the snapshot).
    fn journal_seq(&self) -> u64 {
        0
    }
}
