//! The scrobble database: the [`Config`] row and the offline queue.
//!
//! Every finished play and every love goes into `queue` first (one row per
//! service), and leaves it only once the service took it or refused it for
//! good. Failed rows wait [`backoff`] before the next try; rows older than
//! [`MAX_AGE_S`](crate::MAX_AGE_S) are dropped since the services ignore
//! them anyway.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::config::{Config, Service};
use crate::track::Track;

/// What a queued row asks the service to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Item {
    Scrobble { track: Track, started_at: i64 },
    Love { track: Track, on: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Queued {
    pub id: i64,
    pub item: Item,
    pub attempts: u32,
}

/// Wait before retry number `attempts + 1`: 30 s doubling, at most 6 h.
pub fn backoff(attempts: u32) -> i64 {
    const BASE: i64 = 30;
    const CAP: i64 = 6 * 3600;
    let shift = attempts.saturating_sub(1).min(20);
    (BASE << shift).min(CAP)
}

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueInfo {
    pub pending: u32,
    /// Unix seconds of the next retry, when something is waiting.
    pub next_retry_at: Option<i64>,
    pub last_error: Option<String>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> rusqlite::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS config (
                 id INTEGER PRIMARY KEY CHECK (id = 1),
                 json TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS queue (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 service TEXT NOT NULL,
                 item TEXT NOT NULL,
                 created_at INTEGER NOT NULL,
                 attempts INTEGER NOT NULL DEFAULT 0,
                 next_at INTEGER NOT NULL DEFAULT 0,
                 last_error TEXT
             );
             CREATE INDEX IF NOT EXISTS queue_due ON queue (service, next_at, id);",
        )?;
        Ok(Self { conn })
    }

    pub fn config(&self) -> Config {
        self.conn
            .query_row("SELECT json FROM config WHERE id = 1", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
            .and_then(|j| {
                serde_json::from_str(&j)
                    .inspect_err(|e| tracing::warn!("broken scrobble config, using defaults: {e}"))
                    .ok()
            })
            .unwrap_or_default()
    }

    pub fn save_config(&self, c: &Config) -> rusqlite::Result<()> {
        let json = serde_json::to_string(c).expect("config serialises");
        self.conn.execute(
            "INSERT INTO config (id, json) VALUES (1, ?1)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            [json],
        )?;
        Ok(())
    }

    pub fn push(&self, service: Service, item: &Item, now: i64) -> rusqlite::Result<i64> {
        let json = serde_json::to_string(item).expect("item serialises");
        self.conn.execute(
            "INSERT INTO queue (service, item, created_at, next_at) VALUES (?1, ?2, ?3, 0)",
            params![service.as_str(), json, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Rows of `service` due at `now`, oldest first. Rows past the age limit
    /// are deleted on the way.
    pub fn due(&self, service: Service, now: i64, limit: usize) -> rusqlite::Result<Vec<Queued>> {
        self.conn.execute(
            "DELETE FROM queue WHERE created_at < ?1",
            [now - crate::MAX_AGE_S],
        )?;
        let mut st = self.conn.prepare(
            "SELECT id, item, attempts FROM queue
             WHERE service = ?1 AND next_at <= ?2 ORDER BY id LIMIT ?3",
        )?;
        let rows = st.query_map(params![service.as_str(), now, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })?;
        let mut out = vec![];
        let mut broken = vec![];
        for row in rows {
            let (id, json, attempts) = row?;
            match serde_json::from_str(&json) {
                Ok(item) => out.push(Queued { id, item, attempts }),
                Err(_) => broken.push(id),
            }
        }
        self.done(&broken)?;
        Ok(out)
    }

    /// Sent or refused for good.
    pub fn done(&self, ids: &[i64]) -> rusqlite::Result<()> {
        for id in ids {
            self.conn.execute("DELETE FROM queue WHERE id = ?1", [id])?;
        }
        Ok(())
    }

    /// One more failed attempt: the rows wait [`backoff`].
    pub fn failed(&self, ids: &[i64], now: i64, error: &str) -> rusqlite::Result<()> {
        for id in ids {
            let attempts: u32 = self.conn.query_row(
                "UPDATE queue SET attempts = attempts + 1 WHERE id = ?1 RETURNING attempts",
                [id],
                |r| r.get(0),
            )?;
            self.conn.execute(
                "UPDATE queue SET next_at = ?2, last_error = ?3 WHERE id = ?1",
                params![id, now + backoff(attempts), error],
            )?;
        }
        Ok(())
    }

    /// Makes every waiting row of `service` due now (after a new login).
    pub fn retry_now(&self, service: Service) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE queue SET next_at = 0 WHERE service = ?1",
            [service.as_str()],
        )?;
        Ok(())
    }

    pub fn clear(&self, service: Service) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM queue WHERE service = ?1", [service.as_str()])?;
        Ok(())
    }

    pub fn info(&self, service: Service) -> rusqlite::Result<QueueInfo> {
        let (pending, next): (u32, Option<i64>) = self.conn.query_row(
            "SELECT COUNT(*), MIN(next_at) FROM queue WHERE service = ?1",
            [service.as_str()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let last_error = self
            .conn
            .query_row(
                "SELECT last_error FROM queue WHERE service = ?1 AND last_error IS NOT NULL
                 ORDER BY next_at DESC LIMIT 1",
                [service.as_str()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(QueueInfo {
            pending,
            next_retry_at: next.filter(|n| *n > 0),
            last_error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn love(title: &str) -> Item {
        Item::Love {
            track: Track {
                title: title.into(),
                ..Default::default()
            },
            on: true,
        }
    }

    #[test]
    fn backoff_doubles_to_a_cap() {
        assert_eq!(backoff(0), 30);
        assert_eq!(backoff(1), 30);
        assert_eq!(backoff(2), 60);
        assert_eq!(backoff(3), 120);
        assert_eq!(backoff(10), 30 << 9);
        assert_eq!(backoff(11), 6 * 3600);
        assert_eq!(backoff(u32::MAX), 6 * 3600);
    }

    #[test]
    fn queue_is_per_service_with_backoff() {
        let s = Store::in_memory().unwrap();
        let now = 1_000_000;
        let a = s.push(Service::LastFm, &love("a"), now).unwrap();
        let b = s.push(Service::LastFm, &love("b"), now).unwrap();
        s.push(Service::Maloja, &love("m"), now).unwrap();

        let due = s.due(Service::LastFm, now, 10).unwrap();
        assert_eq!(due.iter().map(|q| q.id).collect::<Vec<_>>(), [a, b]);
        assert_eq!(due[0].item, love("a"));

        s.failed(&[a], now, "offline").unwrap();
        let due = s.due(Service::LastFm, now, 10).unwrap();
        assert_eq!(due.iter().map(|q| q.id).collect::<Vec<_>>(), [b]);
        let due = s.due(Service::LastFm, now + 30, 10).unwrap();
        assert_eq!(due[0].id, a);
        assert_eq!(due[0].attempts, 1);

        s.failed(&[a], now + 30, "offline").unwrap();
        assert!(
            s.due(Service::LastFm, now + 89, 10)
                .unwrap()
                .iter()
                .all(|q| q.id != a)
        );
        assert!(
            s.due(Service::LastFm, now + 90, 10)
                .unwrap()
                .iter()
                .any(|q| q.id == a)
        );

        let info = s.info(Service::LastFm).unwrap();
        assert_eq!(info.pending, 2);
        assert_eq!(info.last_error.as_deref(), Some("offline"));

        s.retry_now(Service::LastFm).unwrap();
        assert_eq!(s.due(Service::LastFm, now, 10).unwrap().len(), 2);
        s.done(&[a, b]).unwrap();
        assert_eq!(s.info(Service::LastFm).unwrap().pending, 0);
        assert_eq!(s.info(Service::Maloja).unwrap().pending, 1);
        s.clear(Service::Maloja).unwrap();
        assert_eq!(s.info(Service::Maloja).unwrap().pending, 0);
    }

    #[test]
    fn old_rows_expire() {
        let s = Store::in_memory().unwrap();
        s.push(Service::ListenBrainz, &love("old"), 0).unwrap();
        assert!(
            s.due(Service::ListenBrainz, crate::MAX_AGE_S + 1, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(s.info(Service::ListenBrainz).unwrap().pending, 0);
    }

    #[test]
    fn config_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scrobble.sqlite3");
        let s = Store::open(&path).unwrap();
        assert_eq!(s.config(), Config::default());
        let mut c = Config::default();
        c.maloja.enabled = true;
        c.percentage = 70;
        s.save_config(&c).unwrap();
        drop(s);
        assert_eq!(Store::open(&path).unwrap().config(), c);
    }
}
