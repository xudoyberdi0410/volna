//! The Rust ↔ UI contract (`docs/native-player.md` §3.2). JSON is camelCase.
//! Changing anything here changes the contract: update the doc in the same
//! commit.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where a queue entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    #[default]
    User,
    Wave,
    Radio,
    Autoplay,
}

/// UI `REPEAT_MODE` 0/1/2 ↔ off/all/one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Playing,
    Paused,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    #[default]
    Track,
    Video,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ArtistRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AlbumRef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: String,
}

/// What the UI (or a [`crate::Hooks::more`] implementation) sends to enqueue a track.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrackInput {
    /// YouTube videoId, `[A-Za-z0-9_-]{11}`.
    pub id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    /// Seconds, from metadata.
    pub duration: Option<f64>,
    pub cover: Option<String>,
    pub explicit: bool,
    pub kind: TrackKind,
    pub origin: Option<Origin>,
    /// Opaque prepared UI track, stored and returned verbatim.
    pub ui: Option<Value>,
    /// hifi-api shaped track for Rust-originated entries. Not sent by the UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

/// True for a YouTube videoId.
pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A queue entry as Rust reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerTrack {
    pub uid: String,
    pub id: String,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration: Option<f64>,
    pub cover: Option<String>,
    pub explicit: bool,
    pub kind: TrackKind,
    pub origin: Origin,
    pub raw: Option<Value>,
    pub ui: Option<Value>,
    #[serde(default)]
    pub unavailable: bool,
}

impl PlayerTrack {
    pub fn from_input(uid: String, t: TrackInput) -> Self {
        Self {
            uid,
            id: t.id,
            title: t.title,
            artists: t.artists,
            album: t.album,
            duration: t.duration.filter(|d| d.is_finite() && *d > 0.0),
            cover: t.cover,
            explicit: t.explicit,
            kind: t.kind,
            origin: t.origin.unwrap_or_default(),
            raw: t.raw,
            ui: t.ui,
            unavailable: false,
        }
    }

    /// "Title — Artist, Artist".
    pub fn display(&self) -> String {
        let artists = self.artist_names();
        if artists.is_empty() {
            self.title.clone()
        } else {
            format!("{} — {artists}", self.title)
        }
    }

    pub fn artist_names(&self) -> String {
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    #[serde(rename = "HIGH", alias = "high")]
    High,
    #[serde(rename = "LOW", alias = "low")]
    Low,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub codec: Option<String>,
    pub bitrate_kbps: Option<f64>,
    pub quality: Quality,
    pub mime: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    BotCheck,
    Unavailable,
    NoYtDlp,
    Decode,
    Timeout,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorAction {
    Skipped,
    Stopped,
    Retrying,
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerError {
    pub uid: Option<String>,
    pub track_id: Option<String>,
    pub kind: ErrorKind,
    pub message: String,
    pub action: ErrorAction,
    pub streak: u32,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub status: Status,
    pub uid: Option<String>,
    pub track_id: Option<String>,
    pub position: f64,
    /// Unix ms of the `position` sample.
    pub at: f64,
    pub duration: Option<f64>,
    pub rate: f64,
    pub buffering: bool,
    pub crossfading: bool,
    pub volume: f64,
    pub muted: bool,
    pub stream: Option<StreamInfo>,
    pub error: Option<PlayerError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContinuationKind {
    #[default]
    None,
    Wave,
    Radio,
    Ui,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Continuation {
    pub kind: ContinuationKind,
    pub fetching: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueChanged {
    pub rev: u64,
    pub order: Vec<String>,
    pub original: Option<Vec<String>>,
    pub index: i64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub continuation: Continuation,
    pub added: Vec<PlayerTrack>,
    pub updated: Vec<PlayerTrack>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSnapshot {
    pub rev: u64,
    pub items: Vec<PlayerTrack>,
    pub original: Option<Vec<String>>,
    pub index: i64,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub continuation: Continuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolumeCurve {
    #[default]
    Linear,
    Cubic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualitySetting {
    #[default]
    High,
    Low,
}

impl From<QualitySetting> for Quality {
    fn from(q: QualitySetting) -> Self {
        match q {
            QualitySetting::High => Quality::High,
            QualitySetting::Low => Quality::Low,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crossfade {
    pub enabled: bool,
    pub seconds: f64,
}

impl Default for Crossfade {
    fn default() -> Self {
        Self {
            enabled: false,
            seconds: 5.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplayGainMode {
    #[default]
    Off,
    Track,
    Album,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ReplayGain {
    pub mode: ReplayGainMode,
    pub preamp_db: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Blocklist {
    pub track_ids: Vec<String>,
    pub artist_ids: Vec<String>,
    pub album_ids: Vec<String>,
}

impl Blocklist {
    /// Same rule as `contentBlockingSettings.shouldHideTrack`.
    pub fn blocks(&self, t: &PlayerTrack) -> bool {
        self.track_ids.contains(&t.id)
            || t.artists
                .iter()
                .filter_map(|a| a.id.as_deref())
                .any(|id| self.artist_ids.iter().any(|b| b == id))
            || t.album
                .as_ref()
                .and_then(|a| a.id.as_deref())
                .is_some_and(|id| self.album_ids.iter().any(|b| b == id))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlayerSettings {
    pub volume: f64,
    pub muted: bool,
    pub volume_curve: VolumeCurve,
    pub quality: QualitySetting,
    pub crossfade: Crossfade,
    pub gapless: bool,
    pub speed: f64,
    pub preserve_pitch: bool,
    pub replay_gain: ReplayGain,
    pub autoplay: bool,
    pub blocklist: Blocklist,
    pub migrated: bool,
}

impl Default for PlayerSettings {
    fn default() -> Self {
        Self {
            volume: 0.7,
            muted: false,
            volume_curve: VolumeCurve::Linear,
            quality: QualitySetting::High,
            crossfade: Crossfade::default(),
            gapless: true,
            speed: 1.0,
            preserve_pitch: true,
            replay_gain: ReplayGain::default(),
            autoplay: false,
            blocklist: Blocklist::default(),
            migrated: false,
        }
    }
}

impl PlayerSettings {
    /// Brings every value into its documented range.
    pub fn clamp(&mut self) {
        self.volume = finite_or(self.volume, 0.7).clamp(0.0, 1.0);
        self.speed = finite_or(self.speed, 1.0).clamp(0.25, 4.0);
        self.crossfade.seconds = finite_or(self.crossfade.seconds, 5.0).clamp(1.0, 12.0);
        self.replay_gain.preamp_db = finite_or(self.replay_gain.preamp_db, 0.0).clamp(-20.0, 20.0);
    }

    /// Linear gain of the master stage: curve(volume) × (muted ? 0 : 1).
    pub fn master_gain(&self) -> f64 {
        if self.muted {
            return 0.0;
        }
        match self.volume_curve {
            VolumeCurve::Linear => self.volume,
            VolumeCurve::Cubic => self.volume.powi(3),
        }
    }

    /// `player_set_settings`: merged one level deep (nested objects replaced whole).
    pub fn merged(&self, patch: &serde_json::Map<String, Value>) -> Result<Self, String> {
        let mut base = serde_json::to_value(self).map_err(|e| e.to_string())?;
        let obj = base.as_object_mut().expect("settings serialise to an object");
        for (k, v) in patch {
            if !obj.contains_key(k) {
                return Err(format!("unknown setting {k:?}"));
            }
            obj.insert(k.clone(), v.clone());
        }
        let mut s: Self = serde_json::from_value(base).map_err(|e| e.to_string())?;
        s.clamp();
        Ok(s)
    }
}

fn finite_or(v: f64, d: f64) -> f64 {
    if v.is_finite() { v } else { d }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BandType {
    #[default]
    Peaking,
    Lowshelf,
    Highshelf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BandChannel {
    #[default]
    Stereo,
    Mid,
    Side,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqBand {
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
    #[serde(default, rename = "type")]
    pub kind: BandType,
    #[serde(default)]
    pub channel: BandChannel,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EqConfig {
    pub enabled: bool,
    pub preamp_db: f64,
    pub bands: Vec<EqBand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeqBand {
    pub freq: f64,
    pub gain_db: f64,
    pub q: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GraphicEqConfig {
    pub enabled: bool,
    pub preamp_db: f64,
    pub bands: Vec<GeqBand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrossfeedLevel {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrossfeedConfig {
    pub enabled: bool,
    pub level: CrossfeedLevel,
}

impl Default for CrossfeedConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: CrossfeedLevel::Medium,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WideningConfig {
    pub enabled: bool,
    /// 0..2, 1 = neutral.
    pub amount: f64,
}

impl Default for WideningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            amount: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BinauralConfig {
    pub enabled: bool,
    pub crossfeed: CrossfeedConfig,
    pub widening: WideningConfig,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DspConfig {
    pub eq: EqConfig,
    pub graphic_eq: GraphicEqConfig,
    pub mono: bool,
    pub binaural: BinauralConfig,
}

impl DspConfig {
    /// Clamps values and drops bands that cannot be built.
    pub fn sanitized(mut self) -> Self {
        let fix = |v: f64, d: f64, lo: f64, hi: f64| finite_or(v, d).clamp(lo, hi);
        self.eq.preamp_db = fix(self.eq.preamp_db, 0.0, -20.0, 20.0);
        self.eq.bands.retain(|b| b.freq.is_finite() && b.freq > 0.0);
        self.eq.bands.truncate(32);
        for b in &mut self.eq.bands {
            b.gain_db = fix(b.gain_db, 0.0, -30.0, 30.0);
            b.q = fix(b.q, 1.0, 0.05, 50.0);
        }
        self.graphic_eq.preamp_db = fix(self.graphic_eq.preamp_db, 0.0, -20.0, 20.0);
        self.graphic_eq
            .bands
            .retain(|b| b.freq.is_finite() && b.freq > 0.0);
        self.graphic_eq.bands.truncate(64);
        for b in &mut self.graphic_eq.bands {
            b.gain_db = fix(b.gain_db, 0.0, -30.0, 30.0);
            b.q = fix(b.q, 1.0, 0.05, 50.0);
        }
        self.binaural.widening.amount = fix(self.binaural.widening.amount, 1.0, 0.0, 2.0);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SleepTimer {
    pub ends_at: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WaveInfo {
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerSnapshot {
    pub protocol: u32,
    pub state: PlaybackState,
    pub queue: QueueSnapshot,
    pub settings: PlayerSettings,
    pub dsp: DspConfig,
    pub sleep_timer: Option<SleepTimer>,
    pub wave: WaveInfo,
    pub journal_seq: u64,
}

/// `player:position`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionEvent {
    pub uid: Option<String>,
    pub position: f64,
    pub duration: Option<f64>,
    pub rate: f64,
    pub at: f64,
    pub playing: bool,
}

/// One journal line (§3.2 `JournalEntry`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum JournalEntry {
    /// Played ≥ 10 s.
    History { seq: u64, at: f64, track: PlayerTrack },
    /// A started play closed.
    End {
        seq: u64,
        at: f64,
        track: PlayerTrack,
        started_at: f64,
        position: f64,
        duration: Option<f64>,
        ended: bool,
        in_wave: bool,
    },
}

impl JournalEntry {
    pub fn seq(&self) -> u64 {
        match self {
            Self::History { seq, .. } | Self::End { seq, .. } => *seq,
        }
    }
}

/// Unix time in ms.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
        .floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"{"protocol":1,
 "state":{"status":"playing","uid":"q1a","trackId":"dQw4w9WgXcQ","position":42.1,"at":1760700000000,
          "duration":213.0,"rate":1,"buffering":false,"crossfading":false,"volume":0.7,"muted":false,
          "stream":{"codec":"opus","bitrateKbps":160,"quality":"HIGH","mime":"audio/webm"},"error":null},
 "queue":{"rev":12,"index":0,"shuffle":false,"repeat":"off","original":null,
          "continuation":{"kind":"wave","fetching":false},
          "items":[{"uid":"q1a","id":"dQw4w9WgXcQ","title":"Never Gonna Give You Up",
                    "artists":[{"id":"UCuAXFkgsw1L7xaCfnd5JJOw","name":"Rick Astley"}],
                    "album":{"id":"MPREb_x","title":"Whenever You Need Somebody"},"duration":213,
                    "cover":"volna://img/?u=https%3A%2F%2Flh3.googleusercontent.com%2Fabc&s=640",
                    "explicit":false,"kind":"track","origin":"wave","raw":{"id":"dQw4w9WgXcQ"},"ui":null,
                    "unavailable":false}]},
 "settings":{"volume":0.7,"muted":false,"volumeCurve":"linear","quality":"high",
             "crossfade":{"enabled":false,"seconds":5},"gapless":true,"speed":1,"preservePitch":true,
             "replayGain":{"mode":"track","preampDb":3},"autoplay":false,
             "blocklist":{"trackIds":[],"artistIds":["3995478"],"albumIds":[]},"migrated":true},
 "dsp":{"eq":{"enabled":false,"preampDb":0,"bands":[]},"graphicEq":{"enabled":false,"preampDb":0,"bands":[]},
        "mono":false,"binaural":{"enabled":false,"crossfeed":{"enabled":true,"level":"medium"},
        "widening":{"enabled":true,"amount":1}}},
 "sleepTimer":null,"wave":{"active":true},"journalSeq":311}"#;

    /// `1` and `1.0` differ as JSON values; compare numbers as f64.
    fn sorted(v: Value) -> Value {
        match v {
            Value::Number(n) => serde_json::json!(n.as_f64().unwrap()),
            Value::Array(a) => Value::Array(a.into_iter().map(sorted).collect()),
            Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, sorted(v))).collect()),
            other => other,
        }
    }

    #[test]
    fn snapshot_fixture_round_trips() {
        let snap: PlayerSnapshot = serde_json::from_str(SNAPSHOT).unwrap();
        assert_eq!(snap.state.status, Status::Playing);
        assert_eq!(snap.queue.continuation.kind, ContinuationKind::Wave);
        assert_eq!(snap.queue.items[0].origin, Origin::Wave);
        assert_eq!(snap.settings.replay_gain.mode, ReplayGainMode::Track);
        assert_eq!(snap.settings.blocklist.artist_ids, vec!["3995478"]);
        assert_eq!(snap.dsp.binaural.crossfeed.level, CrossfeedLevel::Medium);
        assert_eq!(snap.journal_seq, 311);
        let back = serde_json::to_value(&snap).unwrap();
        let orig: Value = serde_json::from_str(SNAPSHOT).unwrap();
        assert_eq!(sorted(back), sorted(orig));
    }

    #[test]
    fn queue_changed_fixture() {
        let item = |uid: &str| {
            serde_json::json!({"uid": uid, "id": "dQw4w9WgXcQ", "title": "x", "artists": [],
                "album": null, "duration": null, "cover": null, "explicit": false, "kind": "track",
                "origin": "wave", "raw": null, "ui": null, "unavailable": false})
        };
        let raw = serde_json::json!({"rev":13,"order":["q1a","q1b","q1c"],"original":null,"index":0,
            "shuffle":false,"repeat":"off","continuation":{"kind":"wave","fetching":false},
            "added":[item("q1b"), item("q1c")],"updated":[]});
        let q: QueueChanged = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(q.rev, 13);
        assert_eq!(q.added.len(), 2);
        assert_eq!(sorted(serde_json::to_value(&q).unwrap()), sorted(raw));
    }

    #[test]
    fn track_input_defaults() {
        let t: TrackInput = serde_json::from_str(r#"{"id":"dQw4w9WgXcQ","title":"x"}"#).unwrap();
        assert_eq!(t.kind, TrackKind::Track);
        assert!(t.origin.is_none() && t.ui.is_none());
        assert!(is_video_id(&t.id));
        assert!(!is_video_id("local:abc"));
        let p = PlayerTrack::from_input("q1".into(), t);
        assert_eq!(p.origin, Origin::User);
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("raw").unwrap().is_null() && v.get("ui").unwrap().is_null());
    }

    #[test]
    fn journal_entries() {
        let track = serde_json::json!({"uid": "q1", "id": "dQw4w9WgXcQ", "title": "x", "artists": [],
                "album": null, "duration": 200.0, "cover": null, "explicit": false, "kind": "track",
                "origin": "user", "raw": null, "ui": {"a": 1}, "unavailable": false});
        let raw = serde_json::json!({"seq": 4, "at": 1.0, "kind": "end", "track": track,
            "startedAt": 0.5, "position": 12.0, "duration": 200.0, "ended": false, "inWave": true});
        let e: JournalEntry = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(e.seq(), 4);
        assert_eq!(sorted(serde_json::to_value(&e).unwrap()), sorted(raw));
        let h = JournalEntry::History {
            seq: 5,
            at: 2.0,
            track: serde_json::from_value(track).unwrap(),
        };
        let v = serde_json::to_value(&h).unwrap();
        assert_eq!(v["kind"], "history");
        assert_eq!(v["seq"], 5);
    }

    #[test]
    fn settings_patch_merges_one_level() {
        let s = PlayerSettings::default();
        let patch = serde_json::json!({"volume": 3.0, "crossfade": {"enabled": true}});
        let m = s.merged(patch.as_object().unwrap()).unwrap();
        assert_eq!(m.volume, 1.0);
        assert!(m.crossfade.enabled);
        // Replaced whole: `seconds` falls back to its default.
        assert_eq!(m.crossfade.seconds, 5.0);
        assert!(
            s.merged(serde_json::json!({"nope": 1}).as_object().unwrap())
                .is_err()
        );
        let mut c = PlayerSettings {
            volume_curve: VolumeCurve::Cubic,
            volume: 0.5,
            ..Default::default()
        };
        assert!((c.master_gain() - 0.125).abs() < 1e-12);
        c.muted = true;
        assert_eq!(c.master_gain(), 0.0);
    }

    #[test]
    fn blocklist_matches_ids() {
        let mut t = PlayerTrack::from_input(
            "q".into(),
            TrackInput {
                id: "dQw4w9WgXcQ".into(),
                artists: vec![ArtistRef {
                    id: Some("UC1".into()),
                    name: "A".into(),
                }],
                ..Default::default()
            },
        );
        let b = Blocklist {
            artist_ids: vec!["UC1".into()],
            ..Default::default()
        };
        assert!(b.blocks(&t));
        t.artists.clear();
        assert!(!b.blocks(&t));
    }
}
