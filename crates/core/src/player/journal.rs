//! Plays made while no window was open, kept for the UI (§3.6): history
//! and scrobbles are written from it when a window attaches.
//!
//! JSONL at `<app_data_dir>/player-journal.jsonl`. `seq` only grows, also
//! across restarts and acks: the first line of a rewritten file is
//! `{"meta":{"seq":N}}`. At most [`CAP`] entries are kept.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Deserialize;
use volna_player::{JournalEntry, PlayerTrack};

pub const CAP: usize = 5000;
const PAGE: usize = 1000;

type Listener = Box<dyn Fn(&JournalEntry) + Send + Sync>;

pub struct Journal {
    path: PathBuf,
    inner: Mutex<Inner>,
    listener: Mutex<Option<Listener>>,
}

struct Inner {
    entries: VecDeque<JournalEntry>,
    seq: u64,
}

#[derive(Deserialize)]
struct Meta {
    meta: MetaSeq,
}

#[derive(Deserialize)]
struct MetaSeq {
    seq: u64,
}

/// What [`Journal::append`] writes (seq and time are filled in).
pub enum Record {
    History {
        at: f64,
        track: PlayerTrack,
    },
    End {
        at: f64,
        track: PlayerTrack,
        started_at: f64,
        position: f64,
        duration: Option<f64>,
        ended: bool,
        in_wave: bool,
    },
}

impl Journal {
    pub fn open(path: &Path) -> Self {
        let mut entries = VecDeque::new();
        let mut seq = 0;
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                if let Ok(m) = serde_json::from_str::<Meta>(line) {
                    seq = seq.max(m.meta.seq);
                } else if let Ok(e) = serde_json::from_str::<JournalEntry>(line) {
                    seq = seq.max(e.seq());
                    entries.push_back(e);
                } else {
                    tracing::warn!("skipping a broken journal line");
                }
            }
        }
        while entries.len() > CAP {
            entries.pop_front();
        }
        Self {
            path: path.to_owned(),
            inner: Mutex::new(Inner { entries, seq }),
            listener: Mutex::new(None),
        }
    }

    /// Called for every append (the live `player:journal` mirror).
    pub fn set_listener(&self, f: impl Fn(&JournalEntry) + Send + Sync + 'static) {
        *self.listener.lock().expect("journal listener") = Some(Box::new(f));
    }

    pub fn seq(&self) -> u64 {
        self.inner.lock().expect("journal").seq
    }

    pub fn append(&self, r: Record) -> JournalEntry {
        let entry = {
            let mut inner = self.inner.lock().expect("journal");
            inner.seq += 1;
            let seq = inner.seq;
            let entry = match r {
                Record::History { at, track } => JournalEntry::History { seq, at, track },
                Record::End {
                    at,
                    track,
                    started_at,
                    position,
                    duration,
                    ended,
                    in_wave,
                } => JournalEntry::End {
                    seq,
                    at,
                    track,
                    started_at,
                    position,
                    duration,
                    ended,
                    in_wave,
                },
            };
            inner.entries.push_back(entry.clone());
            if inner.entries.len() > CAP {
                inner.entries.pop_front();
                // Keep the file bounded too.
                self.rewrite(&inner);
            } else {
                self.append_line(&entry);
            }
            entry
        };
        if let Some(f) = self.listener.lock().expect("journal listener").as_ref() {
            f(&entry);
        }
        entry
    }

    /// Entries with `seq > after`, ascending, at most 1000.
    pub fn after(&self, after: u64) -> Vec<JournalEntry> {
        let inner = self.inner.lock().expect("journal");
        inner
            .entries
            .iter()
            .filter(|e| e.seq() > after)
            .take(PAGE)
            .cloned()
            .collect()
    }

    /// Forgets entries with `seq <= up_to`.
    pub fn ack(&self, up_to: u64) {
        let mut inner = self.inner.lock().expect("journal");
        let before = inner.entries.len();
        inner.entries.retain(|e| e.seq() > up_to);
        if inner.entries.len() != before {
            self.rewrite(&inner);
        }
    }

    fn append_line(&self, e: &JournalEntry) {
        let result = (|| -> std::io::Result<()> {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            let mut line = serde_json::to_vec(e).map_err(std::io::Error::other)?;
            line.push(b'\n');
            f.write_all(&line)
        })();
        if let Err(e) = result {
            tracing::warn!("cannot append to the journal: {e}");
        }
    }

    fn rewrite(&self, inner: &Inner) {
        let mut out = format!("{{\"meta\":{{\"seq\":{}}}}}\n", inner.seq).into_bytes();
        for e in &inner.entries {
            if let Ok(line) = serde_json::to_vec(e) {
                out.extend_from_slice(&line);
                out.push(b'\n');
            }
        }
        if let Err(e) = volna_player::persist::write_atomic(&self.path, &out) {
            tracing::warn!("cannot rewrite the journal: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_player::TrackInput;

    fn track() -> PlayerTrack {
        PlayerTrack::from_input(
            "q1".into(),
            TrackInput {
                id: "dQw4w9WgXcQ".into(),
                ..Default::default()
            },
        )
    }

    #[test]
    fn seq_survives_ack_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("player-journal.jsonl");
        let j = Journal::open(&path);
        let seen = std::sync::Arc::new(Mutex::new(0));
        let s2 = seen.clone();
        j.set_listener(move |_| *s2.lock().unwrap() += 1);
        for i in 0..3 {
            j.append(Record::History {
                at: f64::from(i),
                track: track(),
            });
        }
        j.append(Record::End {
            at: 9.0,
            track: track(),
            started_at: 1.0,
            position: 30.0,
            duration: Some(200.0),
            ended: false,
            in_wave: true,
        });
        assert_eq!(*seen.lock().unwrap(), 4);
        assert_eq!(j.after(0).len(), 4);
        assert_eq!(
            j.after(2).iter().map(JournalEntry::seq).collect::<Vec<_>>(),
            [3, 4]
        );
        drop(j);

        let j = Journal::open(&path);
        assert_eq!(j.seq(), 4);
        assert_eq!(j.after(0).len(), 4);
        j.ack(4);
        assert!(j.after(0).is_empty());
        drop(j);

        let j = Journal::open(&path);
        assert_eq!(j.seq(), 4, "seq must not restart after an ack");
        let e = j.append(Record::History {
            at: 1.0,
            track: track(),
        });
        assert_eq!(e.seq(), 5);
    }
}
