//! What survives a restart: the queue (`player-queue.json` plus a tiny
//! `.pos` sidecar rewritten while playing) and the settings with the DSP
//! config (`player.json`). Paths come from the caller. Writes are atomic
//! (temp file + rename).

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{ContinuationKind, DspConfig, PlayerSettings, PlayerTrack};
use crate::queue::QueueState;

#[derive(Debug, Clone)]
pub struct Paths {
    /// `<app_data_dir>/player-queue.json`.
    pub queue: PathBuf,
    /// `<app_config_dir>/player.json`.
    pub settings: PathBuf,
}

impl Paths {
    fn position(&self) -> PathBuf {
        self.queue.with_extension("pos")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedContinuation {
    pub kind: ContinuationKind,
    /// Radio seeds.
    pub seeds: Vec<PlayerTrack>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedQueue {
    pub version: u32,
    pub queue: QueueState,
    pub continuation: SavedContinuation,
    /// Recently played ids (continuation dedup), newest last.
    pub played: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedPosition {
    pub uid: Option<String>,
    pub position: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SavedSettings {
    pub version: u32,
    pub settings: PlayerSettings,
    pub dsp: DspConfig,
}

/// Everything loaded at startup.
#[derive(Debug, Clone, Default)]
pub struct Persisted {
    pub queue: SavedQueue,
    pub position: SavedPosition,
    pub settings: SavedSettings,
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!("ignoring unreadable {}: {e}", path.display());
            T::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => {
            tracing::warn!("cannot read {}: {e}", path.display());
            T::default()
        }
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_data()?;
    }
    std::fs::rename(&tmp, path)
}

fn write_json<T: Serialize>(path: &Path, value: &T) {
    let result = serde_json::to_vec(value)
        .map_err(std::io::Error::other)
        .and_then(|b| write_atomic(path, &b));
    if let Err(e) = result {
        tracing::warn!("cannot save {}: {e}", path.display());
    }
}

impl Persisted {
    pub fn load(paths: &Paths) -> Self {
        let mut settings: SavedSettings = read_json(&paths.settings);
        settings.settings.clamp();
        settings.dsp = settings.dsp.sanitized();
        Self {
            queue: read_json(&paths.queue),
            position: read_json(&paths.position()),
            settings,
        }
    }
}

pub fn save_queue(paths: &Paths, q: &SavedQueue) {
    write_json(&paths.queue, q);
}

pub fn save_position(paths: &Paths, p: &SavedPosition) {
    write_json(&paths.position(), p);
}

pub fn save_settings(paths: &Paths, s: &SavedSettings) {
    write_json(&paths.settings, s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackInput;

    #[test]
    fn round_trip_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            queue: dir.path().join("data/player-queue.json"),
            settings: dir.path().join("config/player.json"),
        };
        let empty = Persisted::load(&paths);
        assert!(empty.queue.queue.items.is_empty());
        assert_eq!(empty.settings.settings, PlayerSettings::default());

        let track = PlayerTrack::from_input(
            "q1".into(),
            TrackInput {
                id: "dQw4w9WgXcQ".into(),
                ui: Some(serde_json::json!({"x": [1, 2]})),
                ..Default::default()
            },
        );
        let q = SavedQueue {
            version: 1,
            queue: QueueState {
                items: vec![track],
                index: Some(0),
                next_uid: 1,
                ..Default::default()
            },
            continuation: SavedContinuation {
                kind: ContinuationKind::Radio,
                seeds: vec![],
            },
            played: vec!["a".into()],
        };
        save_queue(&paths, &q);
        save_position(
            &paths,
            &SavedPosition {
                uid: Some("q1".into()),
                position: 12.5,
            },
        );
        let mut s = SavedSettings {
            version: 1,
            ..Default::default()
        };
        s.settings.volume = 9.0;
        save_settings(&paths, &s);
        let back = Persisted::load(&paths);
        assert_eq!(back.queue, q);
        assert_eq!(back.position.position, 12.5);
        assert_eq!(back.settings.settings.volume, 1.0);

        std::fs::write(&paths.queue, b"{broken").unwrap();
        assert!(Persisted::load(&paths).queue.queue.items.is_empty());
    }
}
