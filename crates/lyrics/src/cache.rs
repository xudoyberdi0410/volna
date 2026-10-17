//! SQLite cache of chain results by video id (and ISRC when known).
//! An empty result is cached too, for a shorter time, so a track without
//! lyrics does not hit every provider on each play.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::Result;
use crate::model::Lyrics;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS lyrics (
    video_id   TEXT PRIMARY KEY,
    isrc       TEXT,
    sources    TEXT NOT NULL,   -- JSON [Lyrics], best first; [] = none found
    complete   INTEGER NOT NULL, -- 1 when every provider was asked
    fetched_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS lyrics_isrc ON lyrics(isrc);
";

/// How long found lyrics stay.
pub const POSITIVE_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 60);
/// How long "nothing found" stays.
pub const NEGATIVE_TTL: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Best first; empty means no provider had the track.
    pub sources: Vec<Lyrics>,
    /// Every provider was asked (not only the chain until the first hit).
    pub complete: bool,
}

pub struct Cache {
    db: Mutex<Connection>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::with(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(db: Connection) -> Result<Self> {
        db.execute_batch(SCHEMA)?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A fresh entry for the video, else a positive one for the same ISRC.
    pub fn get(&self, video_id: &str, isrc: Option<&str>) -> Result<Option<Entry>> {
        self.get_at(video_id, isrc, now())
    }

    fn get_at(&self, video_id: &str, isrc: Option<&str>, at: i64) -> Result<Option<Entry>> {
        let db = self.conn();
        let row = |sql: &str, key: &str| -> Result<Option<(String, bool, i64)>> {
            Ok(db
                .query_row(sql, params![key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?)
        };
        let mut found = row(
            "SELECT sources, complete, fetched_at FROM lyrics WHERE video_id = ?1",
            video_id,
        )?
        .filter(|r| fresh(r, at));
        if found.is_none()
            && let Some(isrc) = isrc.filter(|s| !s.is_empty())
        {
            found = row(
                "SELECT sources, complete, fetched_at FROM lyrics
                 WHERE isrc = ?1 AND sources != '[]' ORDER BY fetched_at DESC LIMIT 1",
                isrc,
            )?
            .filter(|r| fresh(r, at));
        }
        let Some((json, complete, _)) = found else {
            return Ok(None);
        };
        Ok(Some(Entry {
            sources: serde_json::from_str(&json)?,
            complete,
        }))
    }

    pub fn put(&self, video_id: &str, isrc: Option<&str>, entry: &Entry) -> Result<()> {
        self.put_at(video_id, isrc, entry, now())
    }

    fn put_at(&self, video_id: &str, isrc: Option<&str>, entry: &Entry, at: i64) -> Result<()> {
        let json = serde_json::to_string(&entry.sources)?;
        self.conn().execute(
            "INSERT INTO lyrics (video_id, isrc, sources, complete, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(video_id) DO UPDATE SET
               isrc = COALESCE(excluded.isrc, lyrics.isrc), sources = excluded.sources,
               complete = excluded.complete, fetched_at = excluded.fetched_at",
            params![video_id, isrc, json, entry.complete, at],
        )?;
        Ok(())
    }

    /// Drops expired rows.
    pub fn prune(&self) -> Result<usize> {
        let at = now();
        Ok(self.conn().execute(
            "DELETE FROM lyrics WHERE (sources = '[]' AND fetched_at < ?1) OR fetched_at < ?2",
            params![
                at - NEGATIVE_TTL.as_secs() as i64,
                at - POSITIVE_TTL.as_secs() as i64
            ],
        )?)
    }
}

fn fresh((json, _, fetched_at): &(String, bool, i64), at: i64) -> bool {
    let ttl = if json == "[]" {
        NEGATIVE_TTL
    } else {
        POSITIVE_TTL
    };
    at - fetched_at < ttl.as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> Entry {
        Entry {
            sources: (0..n)
                .map(|i| Lyrics::from_plain(format!("S{i}"), "a\nb", None))
                .collect(),
            complete: false,
        }
    }

    #[test]
    fn round_trip_and_isrc_fallback() {
        let c = Cache::in_memory().unwrap();
        assert!(c.get("v1", None).unwrap().is_none());
        c.put("v1", Some("ISRC1"), &entry(2)).unwrap();
        assert_eq!(c.get("v1", None).unwrap(), Some(entry(2)));
        // Another upload of the same recording.
        assert_eq!(c.get("v2", Some("ISRC1")).unwrap(), Some(entry(2)));
        assert!(c.get("v2", Some("OTHER")).unwrap().is_none());
        // Re-put without an ISRC keeps the known one.
        c.put("v1", None, &entry(1)).unwrap();
        assert_eq!(c.get("v3", Some("ISRC1")).unwrap(), Some(entry(1)));
    }

    #[test]
    fn negative_entries_expire_sooner() {
        let c = Cache::in_memory().unwrap();
        let t0 = 1_000_000_000;
        c.put_at("none", None, &entry(0), t0).unwrap();
        c.put_at("some", None, &entry(1), t0).unwrap();
        let later = t0 + NEGATIVE_TTL.as_secs() as i64 + 1;
        assert!(c.get_at("none", None, t0 + 10).unwrap().is_some());
        assert!(c.get_at("none", None, later).unwrap().is_none());
        assert!(c.get_at("some", None, later).unwrap().is_some());
        let much_later = t0 + POSITIVE_TTL.as_secs() as i64 + 1;
        assert!(c.get_at("some", None, much_later).unwrap().is_none());
    }

    #[test]
    fn opens_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/lyrics.sqlite3");
        Cache::open(&path)
            .unwrap()
            .put("v", None, &entry(1))
            .unwrap();
        assert!(
            Cache::open(&path)
                .unwrap()
                .get("v", None)
                .unwrap()
                .is_some()
        );
    }
}
