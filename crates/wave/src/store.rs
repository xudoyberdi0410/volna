//! SQLite storage of the listener's taste: the event log, likes, dislikes,
//! cold-start artists, track metadata and the wave settings.
//!
//! The database is small (one row per play) and the engine rebuilds its
//! [`crate::profile::TasteProfile`] from a [`Snapshot`] whenever it ranks,
//! so the schema stays a plain log without derived tables.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use volna_ytm::{ArtistRef, Track};

use crate::error::Result;
use crate::settings::WaveSettings;
use crate::signal::PlayOutcome;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tracks (
    video_id   TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    artists    TEXT NOT NULL,   -- JSON [ArtistRef]
    data       TEXT NOT NULL,   -- JSON volna_ytm::Track
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS events (
    id         INTEGER PRIMARY KEY,
    ts         INTEGER NOT NULL,
    video_id   TEXT NOT NULL,
    kind       TEXT NOT NULL,   -- start | complete | early_skip | late_skip
    position_s REAL,
    duration_s REAL,
    in_wave    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS events_ts ON events(ts);
CREATE TABLE IF NOT EXISTS likes (
    video_id TEXT PRIMARY KEY,
    ts       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS dislikes (
    video_id TEXT PRIMARY KEY,
    ts       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS seed_artists (
    artist_id TEXT PRIMARY KEY,
    name      TEXT NOT NULL,
    ts        INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Key that identifies an artist across listings: the channel id when there
/// is one, otherwise the lower-cased name.
pub fn artist_key(a: &ArtistRef) -> String {
    match a.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => id.strip_prefix("MPLA").unwrap_or(id).to_owned(),
        None => format!("name:{}", a.name.trim().to_lowercase()),
    }
}

/// An artist chosen on the cold-start screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedArtist {
    pub id: String,
    pub name: String,
}

/// One finished play.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRow {
    pub ts: i64,
    pub video_id: String,
    pub outcome: PlayOutcome,
    pub in_wave: bool,
}

/// Everything the profile needs, read in one go.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub plays: Vec<PlayRow>,
    /// Last time each track started playing.
    pub last_started: HashMap<String, i64>,
    pub likes: Vec<(String, i64)>,
    pub dislikes: Vec<(String, i64)>,
    pub seed_artists: Vec<(SeedArtist, i64)>,
    /// Artists of every track mentioned above.
    pub track_artists: HashMap<String, Vec<ArtistRef>>,
}

/// Thread-safe handle; every call is one short transaction.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            // Best effort; `Connection::open` reports the real error.
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic mid-statement leaves SQLite consistent; keep going.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Remembers a track's metadata (artists drive the artist affinity).
    pub fn upsert_track(&self, t: &Track, now: i64) -> Result<()> {
        let artists = serde_json::to_string(&t.artists)?;
        let data = serde_json::to_string(t)?;
        self.conn().execute(
            "INSERT INTO tracks (video_id, title, artists, data, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(video_id) DO UPDATE SET
                title = excluded.title, artists = excluded.artists,
                data = excluded.data, updated_at = excluded.updated_at",
            params![t.video_id, t.title, artists, data, now],
        )?;
        Ok(())
    }

    pub fn track(&self, video_id: &str) -> Result<Option<Track>> {
        let data: Option<String> = self
            .conn()
            .query_row(
                "SELECT data FROM tracks WHERE video_id = ?1",
                [video_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match data {
            Some(d) => Some(serde_json::from_str(&d)?),
            None => None,
        })
    }

    pub fn record_start(&self, video_id: &str, now: i64, in_wave: bool) -> Result<()> {
        self.conn().execute(
            "INSERT INTO events (ts, video_id, kind, in_wave) VALUES (?1, ?2, 'start', ?3)",
            params![now, video_id, in_wave],
        )?;
        Ok(())
    }

    pub fn record_outcome(
        &self,
        video_id: &str,
        now: i64,
        outcome: PlayOutcome,
        position_s: f64,
        duration_s: Option<f64>,
        in_wave: bool,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO events (ts, video_id, kind, position_s, duration_s, in_wave)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                now,
                video_id,
                outcome.as_str(),
                position_s,
                duration_s,
                in_wave
            ],
        )?;
        Ok(())
    }

    pub fn set_like(&self, video_id: &str, liked: bool, now: i64) -> Result<()> {
        let conn = self.conn();
        if liked {
            conn.execute(
                "INSERT OR IGNORE INTO likes (video_id, ts) VALUES (?1, ?2)",
                params![video_id, now],
            )?;
            conn.execute("DELETE FROM dislikes WHERE video_id = ?1", [video_id])?;
        } else {
            conn.execute("DELETE FROM likes WHERE video_id = ?1", [video_id])?;
        }
        Ok(())
    }

    /// Makes the likes table equal to the UI's favourites (the UI owns likes).
    /// Existing rows keep their timestamp; tracks are upserted for artists.
    pub fn replace_likes(&self, liked: &[(Track, i64)], now: i64) -> Result<()> {
        for (t, _) in liked {
            self.upsert_track(t, now)?;
        }
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "CREATE TEMP TABLE IF NOT EXISTS keep (video_id TEXT PRIMARY KEY)",
            [],
        )?;
        tx.execute("DELETE FROM keep", [])?;
        for (t, ts) in liked {
            tx.execute(
                "INSERT OR IGNORE INTO keep (video_id) VALUES (?1)",
                [&t.video_id],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO likes (video_id, ts) VALUES (?1, ?2)",
                params![t.video_id, ts],
            )?;
        }
        tx.execute(
            "DELETE FROM likes WHERE video_id NOT IN (SELECT video_id FROM keep)",
            [],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn set_dislike(&self, video_id: &str, disliked: bool, now: i64) -> Result<()> {
        let conn = self.conn();
        if disliked {
            conn.execute(
                "INSERT OR REPLACE INTO dislikes (video_id, ts) VALUES (?1, ?2)",
                params![video_id, now],
            )?;
        } else {
            conn.execute("DELETE FROM dislikes WHERE video_id = ?1", [video_id])?;
        }
        Ok(())
    }

    pub fn is_disliked(&self, video_id: &str) -> Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM dislikes WHERE video_id = ?1",
                [video_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn add_seed_artists(&self, artists: &[SeedArtist], now: i64) -> Result<()> {
        let conn = self.conn();
        for a in artists {
            conn.execute(
                "INSERT OR REPLACE INTO seed_artists (artist_id, name, ts) VALUES (?1, ?2, ?3)",
                params![a.id, a.name, now],
            )?;
        }
        Ok(())
    }

    pub fn settings(&self) -> Result<WaveSettings> {
        let raw: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = 'wave'", [], |r| {
                r.get(0)
            })
            .optional()?;
        // Unknown or stale JSON falls back to defaults instead of failing.
        Ok(raw
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default())
    }

    pub fn set_settings(&self, s: &WaveSettings) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES ('wave', ?1)",
            [serde_json::to_string(s)?],
        )?;
        Ok(())
    }

    /// Plays, likes and so on newer than `since` (likes, dislikes and seed
    /// artists are always read whole: they are few and decay slowly).
    pub fn snapshot(&self, since: i64) -> Result<Snapshot> {
        let conn = self.conn();
        let mut snap = Snapshot::default();

        let mut stmt = conn.prepare_cached(
            "SELECT ts, video_id, kind, in_wave FROM events WHERE ts >= ?1 ORDER BY ts, id",
        )?;
        let rows = stmt.query_map([since], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })?;
        for row in rows {
            let (ts, video_id, kind, in_wave) = row?;
            if kind == "start" {
                snap.last_started.insert(video_id, ts);
            } else if let Some(outcome) = PlayOutcome::parse(&kind) {
                snap.plays.push(PlayRow {
                    ts,
                    video_id,
                    outcome,
                    in_wave,
                });
            }
        }
        drop(stmt);

        let pairs = |sql: &str| -> rusqlite::Result<Vec<(String, i64)>> {
            let mut stmt = conn.prepare_cached(sql)?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        };
        snap.likes = pairs("SELECT video_id, ts FROM likes")?;
        snap.dislikes = pairs("SELECT video_id, ts FROM dislikes")?;

        let mut stmt = conn.prepare_cached("SELECT artist_id, name, ts FROM seed_artists")?;
        snap.seed_artists = stmt
            .query_map([], |r| {
                Ok((
                    SeedArtist {
                        id: r.get(0)?,
                        name: r.get(1)?,
                    },
                    r.get(2)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);

        let mut stmt = conn.prepare_cached(
            "SELECT video_id, artists FROM tracks WHERE video_id IN (
                SELECT video_id FROM events WHERE ts >= ?1
                UNION SELECT video_id FROM likes
                UNION SELECT video_id FROM dislikes)",
        )?;
        let rows = stmt.query_map([since], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, artists) = row?;
            snap.track_artists
                .insert(id, serde_json::from_str(&artists).unwrap_or_default());
        }
        Ok(snap)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use volna_ytm::TrackKind;

    pub fn track(id: &str, artist: &str) -> Track {
        Track {
            video_id: id.into(),
            title: format!("Song {id}"),
            artists: vec![ArtistRef {
                id: Some(format!("UC{artist}")),
                name: artist.into(),
            }],
            album: None,
            duration_s: Some(200),
            thumbnails: vec![],
            explicit: false,
            kind: TrackKind::Song,
            year: None,
            views: None,
            playable: true,
        }
    }

    #[test]
    fn events_likes_and_settings_round_trip() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_track(&track("a1", "A"), 10).unwrap();
        store.record_start("a1", 100, true).unwrap();
        store
            .record_outcome("a1", 300, PlayOutcome::Complete, 200.0, Some(200.0), true)
            .unwrap();
        store.set_like("a1", true, 400).unwrap();
        store.set_dislike("b1", true, 500).unwrap();

        let snap = store.snapshot(0).unwrap();
        assert_eq!(snap.plays.len(), 1);
        assert_eq!(snap.plays[0].outcome, PlayOutcome::Complete);
        assert_eq!(snap.last_started["a1"], 100);
        assert_eq!(snap.likes, vec![("a1".to_owned(), 400)]);
        assert_eq!(snap.dislikes, vec![("b1".to_owned(), 500)]);
        assert_eq!(snap.track_artists["a1"][0].name, "A");
        assert!(store.snapshot(301).unwrap().plays.is_empty());

        // Liking a disliked track lifts the ban.
        store.set_like("b1", true, 600).unwrap();
        assert!(!store.is_disliked("b1").unwrap());

        let mut s = store.settings().unwrap();
        assert_eq!(s, WaveSettings::default());
        s.language = crate::settings::Language::Russian;
        store.set_settings(&s).unwrap();
        assert_eq!(store.settings().unwrap(), s);
        assert_eq!(store.track("a1").unwrap().unwrap().title, "Song a1");
    }

    #[test]
    fn replace_likes_mirrors_the_ui() {
        let store = Store::open_in_memory().unwrap();
        store.set_like("old", true, 1).unwrap();
        store.set_like("kept", true, 2).unwrap();
        store
            .replace_likes(&[(track("kept", "K"), 50), (track("new", "N"), 60)], 70)
            .unwrap();
        let mut likes = store.snapshot(0).unwrap().likes;
        likes.sort();
        // "kept" keeps its original timestamp.
        assert_eq!(likes, vec![("kept".into(), 2), ("new".into(), 60)]);
    }

    #[test]
    fn artist_keys_prefer_ids() {
        let with_id = ArtistRef {
            id: Some("MPLAUCabc".into()),
            name: "X".into(),
        };
        let without = ArtistRef {
            id: None,
            name: " Various Artists ".into(),
        };
        assert_eq!(artist_key(&with_id), "UCabc");
        assert_eq!(artist_key(&without), "name:various artists");
    }

    #[test]
    fn opens_a_file_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/wave.sqlite3");
        Store::open(&path).unwrap().set_like("x", true, 1).unwrap();
        let again = Store::open(&path).unwrap();
        assert_eq!(again.snapshot(0).unwrap().likes.len(), 1);
    }
}
