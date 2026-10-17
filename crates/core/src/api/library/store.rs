//! SQLite storage of the user library: likes, user playlists, history,
//! pinned items and what the legacy (webview) library left behind.
//!
//! `<app data dir>/library.sqlite3`, next to the wave's `wave.sqlite3`.
//! A separate file and connection: the wave database is a taste log the
//! engine owns and may rebuild; the library is user data that must survive
//! that. Both only need a data directory, so they move to `volna-core`
//! together.
//!
//! Every row keeps the YouTube Music model it was liked as (`data`, JSON)
//! plus the fields a list needs (title, artists, cover, duration) as plain
//! columns, so lists render offline and a row whose `data` no longer parses
//! (a model changed shape) still comes back.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use volna_ytm::{
    AlbumKind, AlbumRef, AlbumSummary, ArtistRef, ArtistSummary, Item, PlaylistSummary, Thumbnail,
    Track, TrackKind,
};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS likes (
    kind       TEXT NOT NULL,              -- track | album | artist | playlist
    id         TEXT NOT NULL,              -- videoId | MPREb_… | UC… | PL…/RD…/OLAK…
    title      TEXT NOT NULL,
    artists    TEXT NOT NULL DEFAULT '[]', -- JSON [ArtistRef]
    album      TEXT,                       -- JSON AlbumRef (tracks)
    thumbnail  TEXT,                       -- https URL of the largest thumbnail
    duration_s INTEGER,
    data       TEXT NOT NULL,              -- JSON volna_ytm::Item
    added_at   INTEGER NOT NULL,           -- unix ms
    PRIMARY KEY (kind, id)
);
CREATE INDEX IF NOT EXISTS likes_added ON likes(kind, added_at);
CREATE TABLE IF NOT EXISTS playlists (
    id          TEXT PRIMARY KEY,          -- UUID
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    cover       TEXT,                      -- custom cover URL, if any
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS playlist_items (
    playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    pos         INTEGER NOT NULL,          -- 0-based, dense
    video_id    TEXT NOT NULL,
    title       TEXT NOT NULL,
    artists     TEXT NOT NULL DEFAULT '[]',
    album       TEXT,
    thumbnail   TEXT,
    duration_s  INTEGER,
    data        TEXT NOT NULL,             -- JSON volna_ytm::Track
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, video_id)
);
CREATE INDEX IF NOT EXISTS playlist_items_pos ON playlist_items(playlist_id, pos);
CREATE TABLE IF NOT EXISTS history (
    video_id   TEXT PRIMARY KEY,           -- one row per track: its latest play
    title      TEXT NOT NULL,
    artists    TEXT NOT NULL DEFAULT '[]',
    album      TEXT,
    thumbnail  TEXT,
    duration_s INTEGER,
    data       TEXT NOT NULL,              -- JSON volna_ytm::Track
    played_at  INTEGER NOT NULL            -- unix ms
);
CREATE INDEX IF NOT EXISTS history_played ON history(played_at);
CREATE TABLE IF NOT EXISTS pinned (
    kind      TEXT NOT NULL,               -- album | artist | playlist | user_playlist
    id        TEXT NOT NULL,
    title     TEXT NOT NULL,
    thumbnail TEXT,
    pinned_at INTEGER NOT NULL,
    PRIMARY KEY (kind, id)
);
-- Raw values from the webview library nobody maps yet (scrobbler keys from
-- localStorage under their own names, `idb:settings`, `idb:user_folders`).
CREATE TABLE IF NOT EXISTS legacy_kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// History keeps this many tracks.
pub const HISTORY_CAP: usize = 5000;

/// What can be liked. Videos are tracks (`TrackKind::Video`); YouTube Music
/// mixes (`RD…`) are playlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LikeKind {
    #[serde(alias = "video")]
    Track,
    Album,
    Artist,
    #[serde(alias = "mix")]
    Playlist,
}

impl LikeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
        }
    }

    pub fn of(item: &Item) -> Self {
        match item {
            Item::Track(_) => Self::Track,
            Item::Album(_) => Self::Album,
            Item::Artist(_) => Self::Artist,
            Item::Playlist(_) => Self::Playlist,
        }
    }
}

/// What can be pinned to the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    Album,
    Artist,
    Playlist,
    #[serde(alias = "user-playlist", alias = "userplaylist")]
    UserPlaylist,
}

impl PinKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Album => "album",
            Self::Artist => "artist",
            Self::Playlist => "playlist",
            Self::UserPlaylist => "user_playlist",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
    }
}

/// A liked item, newest first in lists.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Liked {
    pub item: Item,
    pub added_at: i64,
}

/// A user playlist without its tracks.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Custom cover; otherwise draw a collage of `thumbnails`.
    pub cover: Option<String>,
    /// Up to four distinct track covers, in playlist order.
    pub thumbnails: Vec<String>,
    pub track_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistEntry {
    pub track: Track,
    pub added_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistDetail {
    #[serde(flatten)]
    pub info: PlaylistInfo,
    pub tracks: Vec<PlaylistEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub track: Track,
    pub played_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub kind: PinKind,
    pub id: String,
    pub title: String,
    pub thumbnail: Option<String>,
    pub pinned_at: i64,
}

/// The list columns of a row.
struct Cols {
    id: String,
    title: String,
    artists: Vec<ArtistRef>,
    album: Option<AlbumRef>,
    thumbnail: Option<String>,
    duration_s: Option<u32>,
}

/// The largest `https://` thumbnail.
pub fn best_thumbnail(thumbs: &[Thumbnail]) -> Option<&str> {
    thumbs
        .iter()
        .enumerate()
        .max_by_key(|(i, t)| (u64::from(t.width) * u64::from(t.height), *i))
        .map(|(_, t)| t.url.as_str())
        .filter(|u| u.starts_with("https://"))
}

fn thumbs_of(url: Option<String>) -> Vec<Thumbnail> {
    url.map(|url| Thumbnail {
        url,
        width: 0,
        height: 0,
    })
    .into_iter()
    .collect()
}

/// Canonical library id: artists without `MPLA`, playlists without `VL`.
pub fn canonical_id(kind: LikeKind, id: &str) -> String {
    let id = id.trim();
    match kind {
        LikeKind::Artist => id.strip_prefix("MPLA").unwrap_or(id),
        LikeKind::Playlist => id.strip_prefix("VL").unwrap_or(id),
        _ => id,
    }
    .to_owned()
}

fn track_cols(t: &Track) -> Cols {
    Cols {
        id: t.video_id.clone(),
        title: t.title.clone(),
        artists: t.artists.clone(),
        album: t.album.clone(),
        thumbnail: best_thumbnail(&t.thumbnails).map(str::to_owned),
        duration_s: t.duration_s,
    }
}

fn cols(item: &Item) -> Cols {
    match item {
        Item::Track(t) => track_cols(t),
        Item::Album(a) => Cols {
            id: a.browse_id.clone(),
            title: a.title.clone(),
            artists: a.artists.clone(),
            album: None,
            thumbnail: best_thumbnail(&a.thumbnails).map(str::to_owned),
            duration_s: None,
        },
        Item::Artist(a) => Cols {
            id: canonical_id(LikeKind::Artist, &a.id),
            title: a.name.clone(),
            artists: vec![],
            album: None,
            thumbnail: best_thumbnail(&a.thumbnails).map(str::to_owned),
            duration_s: None,
        },
        Item::Playlist(p) => Cols {
            id: canonical_id(LikeKind::Playlist, &p.id),
            title: p.title.clone(),
            artists: p.author.clone().into_iter().collect(),
            album: None,
            thumbnail: best_thumbnail(&p.thumbnails).map(str::to_owned),
            duration_s: None,
        },
    }
}

/// A model from its columns, when `data` no longer parses.
fn rebuild(kind: LikeKind, c: Cols) -> Item {
    match kind {
        LikeKind::Track => Item::Track(rebuild_track(c)),
        LikeKind::Album => Item::Album(AlbumSummary {
            browse_id: c.id,
            playlist_id: None,
            title: c.title,
            kind: AlbumKind::Album,
            artists: c.artists,
            year: None,
            thumbnails: thumbs_of(c.thumbnail),
            explicit: false,
        }),
        LikeKind::Artist => Item::Artist(ArtistSummary {
            id: c.id,
            name: c.title,
            thumbnails: thumbs_of(c.thumbnail),
            audience: None,
        }),
        LikeKind::Playlist => Item::Playlist(PlaylistSummary {
            id: c.id,
            title: c.title,
            author: c.artists.into_iter().next(),
            thumbnails: thumbs_of(c.thumbnail),
            track_count: None,
            subtitle: None,
        }),
    }
}

fn rebuild_track(c: Cols) -> Track {
    Track {
        video_id: c.id,
        title: c.title,
        artists: c.artists,
        album: c.album,
        duration_s: c.duration_s,
        thumbnails: thumbs_of(c.thumbnail),
        explicit: false,
        kind: TrackKind::Song,
        year: None,
        views: None,
        playable: true,
    }
}

/// Reads `id, title, artists, album, thumbnail, duration_s, data` at 0..=6.
fn read_cols(r: &Row<'_>) -> rusqlite::Result<(Cols, String)> {
    let artists: String = r.get(2)?;
    let album: Option<String> = r.get(3)?;
    Ok((
        Cols {
            id: r.get(0)?,
            title: r.get(1)?,
            artists: serde_json::from_str(&artists).unwrap_or_default(),
            album: album.and_then(|a| serde_json::from_str(&a).ok()),
            thumbnail: r.get(4)?,
            duration_s: r.get(5)?,
        },
        r.get(6)?,
    ))
}

fn track_from(c: Cols, data: &str) -> Track {
    serde_json::from_str(data).unwrap_or_else(|_| rebuild_track(c))
}

/// Thread-safe handle; every call is one short transaction.
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ---- likes -------------------------------------------------------------

    /// Likes `item`. An item liked before keeps its `added_at` and gets the
    /// fresh metadata.
    pub fn like(&self, item: &Item, added_at: i64) -> Result<()> {
        like_in(&self.conn(), item, added_at)
    }

    /// Likes many items in one transaction (imports).
    pub fn like_many(&self, items: &[(Item, i64)]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (item, at) in items {
            like_in(&tx, item, *at)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// True when something was removed.
    pub fn unlike(&self, kind: LikeKind, id: &str) -> Result<bool> {
        let n = self.conn().execute(
            "DELETE FROM likes WHERE kind = ?1 AND id = ?2",
            params![kind.as_str(), canonical_id(kind, id)],
        )?;
        Ok(n > 0)
    }

    pub fn liked(&self, kind: LikeKind, id: &str) -> Result<Option<Liked>> {
        let conn = self.conn();
        let row = conn
            .query_row(
                "SELECT id, title, artists, album, thumbnail, duration_s, data, added_at
                 FROM likes WHERE kind = ?1 AND id = ?2",
                params![kind.as_str(), canonical_id(kind, id)],
                |r| Ok((read_cols(r)?, r.get::<_, i64>(7)?)),
            )
            .optional()?;
        Ok(row.map(|((c, data), added_at)| liked_from(kind, c, &data, added_at)))
    }

    /// Newest first.
    pub fn likes(&self, kind: LikeKind) -> Result<Vec<Liked>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT id, title, artists, album, thumbnail, duration_s, data, added_at
             FROM likes WHERE kind = ?1 ORDER BY added_at DESC, rowid DESC",
        )?;
        let rows = st.query_map([kind.as_str()], |r| {
            Ok((read_cols(r)?, r.get::<_, i64>(7)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let ((c, data), added_at) = row?;
            out.push(liked_from(kind, c, &data, added_at));
        }
        Ok(out)
    }

    /// Which of `ids` are liked.
    pub fn liked_ids(&self, kind: LikeKind, ids: &[String]) -> Result<HashSet<String>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT 1 FROM likes WHERE kind = ?1 AND id = ?2")?;
        let mut out = HashSet::new();
        for id in ids {
            if st.exists(params![kind.as_str(), canonical_id(kind, id)])? {
                out.insert(id.clone());
            }
        }
        Ok(out)
    }

    /// Liked tracks with their `added_at` in seconds (the wave's unit).
    pub fn liked_tracks_for_wave(&self) -> Result<Vec<(Track, i64)>> {
        Ok(self
            .likes(LikeKind::Track)?
            .into_iter()
            .filter_map(|l| match l.item {
                Item::Track(t) => Some((t, l.added_at / 1000)),
                _ => None,
            })
            .collect())
    }

    // ---- user playlists ------------------------------------------------------

    pub fn playlist_create(
        &self,
        id: &str,
        name: &str,
        description: &str,
        cover: Option<&str>,
        created_at: i64,
        updated_at: i64,
    ) -> Result<()> {
        self.conn().execute(
            "INSERT INTO playlists (id, name, description, cover, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name,
                description = excluded.description, cover = excluded.cover,
                updated_at = max(updated_at, excluded.updated_at)",
            params![id, name, description, cover, created_at, updated_at],
        )?;
        Ok(())
    }

    /// False when there is no such playlist.
    pub fn playlist_rename(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        now: i64,
    ) -> Result<bool> {
        let n = self.conn().execute(
            "UPDATE playlists SET name = coalesce(?2, name),
                description = coalesce(?3, description), updated_at = ?4
             WHERE id = ?1",
            params![id, name, description, now],
        )?;
        Ok(n > 0)
    }

    pub fn playlist_delete(&self, id: &str) -> Result<bool> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM playlist_items WHERE playlist_id = ?1", [id])?;
        let n = tx.execute("DELETE FROM playlists WHERE id = ?1", [id])?;
        tx.execute(
            "DELETE FROM pinned WHERE kind = 'user_playlist' AND id = ?1",
            [id],
        )?;
        tx.commit()?;
        Ok(n > 0)
    }

    fn playlist_exists(conn: &Connection, id: &str) -> Result<bool> {
        Ok(conn
            .prepare_cached("SELECT 1 FROM playlists WHERE id = ?1")?
            .exists([id])?)
    }

    /// Appends the tracks not in the playlist yet; returns how many.
    pub fn playlist_add(&self, id: &str, tracks: &[(Track, i64)], now: i64) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if !Self::playlist_exists(&tx, id)? {
            anyhow::bail!("not_found: no playlist {id}");
        }
        let mut pos: i64 = tx.query_row(
            "SELECT coalesce(max(pos) + 1, 0) FROM playlist_items WHERE playlist_id = ?1",
            [id],
            |r| r.get(0),
        )?;
        let mut added = 0;
        for (t, at) in tracks {
            if t.video_id.is_empty() {
                continue;
            }
            let c = track_cols(t);
            let n = tx.execute(
                "INSERT OR IGNORE INTO playlist_items
                    (playlist_id, pos, video_id, title, artists, album, thumbnail, duration_s, data, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    pos,
                    c.id,
                    c.title,
                    serde_json::to_string(&c.artists)?,
                    c.album.as_ref().map(serde_json::to_string).transpose()?,
                    c.thumbnail,
                    c.duration_s,
                    serde_json::to_string(t)?,
                    at,
                ],
            )?;
            if n > 0 {
                pos += 1;
                added += 1;
            }
        }
        if added > 0 {
            tx.execute(
                "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
        }
        tx.commit()?;
        Ok(added)
    }

    /// Removes the tracks; returns how many were there.
    pub fn playlist_remove(&self, id: &str, video_ids: &[String], now: i64) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        if !Self::playlist_exists(&tx, id)? {
            anyhow::bail!("not_found: no playlist {id}");
        }
        let mut removed = 0;
        for v in video_ids {
            removed += tx.execute(
                "DELETE FROM playlist_items WHERE playlist_id = ?1 AND video_id = ?2",
                params![id, v],
            )?;
        }
        if removed > 0 {
            renumber(&tx, id, None)?;
            tx.execute(
                "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
        }
        tx.commit()?;
        Ok(removed)
    }

    /// Moves the track at `from` to `to` (0-based, `to` clamped).
    pub fn playlist_move(&self, id: &str, from: usize, to: usize, now: i64) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut order = item_order(&tx, id)?;
        if from >= order.len() {
            anyhow::bail!("bad_args: no track at {from}");
        }
        let v = order.remove(from);
        order.insert(to.min(order.len()), v);
        renumber(&tx, id, Some(order))?;
        tx.execute(
            "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn playlist(&self, id: &str) -> Result<Option<PlaylistDetail>> {
        let conn = self.conn();
        let Some(info) = playlist_info(&conn, id)? else {
            return Ok(None);
        };
        let mut st = conn.prepare(
            "SELECT video_id, title, artists, album, thumbnail, duration_s, data, added_at
             FROM playlist_items WHERE playlist_id = ?1 ORDER BY pos",
        )?;
        let rows = st.query_map([id], |r| Ok((read_cols(r)?, r.get::<_, i64>(7)?)))?;
        let mut tracks = Vec::new();
        for row in rows {
            let ((c, data), added_at) = row?;
            tracks.push(PlaylistEntry {
                track: track_from(c, &data),
                added_at,
            });
        }
        Ok(Some(PlaylistDetail { info, tracks }))
    }

    /// Newest first.
    pub fn playlists(&self) -> Result<Vec<PlaylistInfo>> {
        let conn = self.conn();
        let ids: Vec<String> = conn
            .prepare("SELECT id FROM playlists ORDER BY created_at DESC, rowid DESC")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(info) = playlist_info(&conn, &id)? {
                out.push(info);
            }
        }
        Ok(out)
    }

    // ---- history -------------------------------------------------------------

    /// Records a play; each track keeps only its latest one.
    pub fn history_add(&self, t: &Track, played_at: i64) -> Result<()> {
        if t.video_id.is_empty() {
            return Ok(());
        }
        let c = track_cols(t);
        let conn = self.conn();
        conn.execute(
            "INSERT INTO history (video_id, title, artists, album, thumbnail, duration_s, data, played_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(video_id) DO UPDATE SET
                title = excluded.title, artists = excluded.artists,
                album = coalesce(excluded.album, album),
                thumbnail = coalesce(excluded.thumbnail, thumbnail),
                duration_s = coalesce(excluded.duration_s, duration_s),
                data = excluded.data,
                played_at = max(played_at, excluded.played_at)",
            params![
                c.id,
                c.title,
                serde_json::to_string(&c.artists)?,
                c.album.as_ref().map(serde_json::to_string).transpose()?,
                c.thumbnail,
                c.duration_s,
                serde_json::to_string(t)?,
                played_at,
            ],
        )?;
        conn.execute(
            "DELETE FROM history WHERE video_id IN (
                SELECT video_id FROM history ORDER BY played_at DESC LIMIT -1 OFFSET ?1)",
            [HISTORY_CAP as i64],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn history(&self, limit: usize, offset: usize) -> Result<Vec<HistoryEntry>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT video_id, title, artists, album, thumbnail, duration_s, data, played_at
             FROM history ORDER BY played_at DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = st.query_map(params![limit as i64, offset as i64], |r| {
            Ok((read_cols(r)?, r.get::<_, i64>(7)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let ((c, data), played_at) = row?;
            out.push(HistoryEntry {
                track: track_from(c, &data),
                played_at,
            });
        }
        Ok(out)
    }

    pub fn history_clear(&self) -> Result<()> {
        self.conn().execute("DELETE FROM history", [])?;
        Ok(())
    }

    // ---- pinned --------------------------------------------------------------

    pub fn pin(&self, p: &Pin) -> Result<()> {
        self.conn().execute(
            "INSERT INTO pinned (kind, id, title, thumbnail, pinned_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(kind, id) DO UPDATE SET title = excluded.title,
                thumbnail = excluded.thumbnail",
            params![p.kind.as_str(), p.id, p.title, p.thumbnail, p.pinned_at],
        )?;
        Ok(())
    }

    pub fn unpin(&self, kind: PinKind, id: &str) -> Result<bool> {
        let n = self.conn().execute(
            "DELETE FROM pinned WHERE kind = ?1 AND id = ?2",
            params![kind.as_str(), id],
        )?;
        Ok(n > 0)
    }

    /// Newest first.
    pub fn pins(&self) -> Result<Vec<Pin>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT kind, id, title, thumbnail, pinned_at FROM pinned
             ORDER BY pinned_at DESC, rowid DESC",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, id, title, thumbnail, pinned_at) = row?;
            if let Some(kind) = PinKind::parse(&kind) {
                out.push(Pin {
                    kind,
                    id,
                    title,
                    thumbnail,
                    pinned_at,
                });
            }
        }
        Ok(out)
    }

    // ---- key/value -----------------------------------------------------------

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Raw values left by the webview library (see `legacy.rs`).
    pub fn set_legacy(&self, entries: &[(String, String)]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (k, v) in entries {
            tx.execute(
                "INSERT OR REPLACE INTO legacy_kv (key, value) VALUES (?1, ?2)",
                params![k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn legacy(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row("SELECT value FROM legacy_kv WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// All legacy entries whose key starts with `prefix`.
    pub fn legacy_prefixed(&self, prefix: &str) -> Result<Vec<(String, String)>> {
        let conn = self.conn();
        let mut st = conn.prepare(
            "SELECT key, value FROM legacy_kv WHERE substr(key, 1, length(?1)) = ?1 ORDER BY key",
        )?;
        let rows = st.query_map([prefix], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}

fn liked_from(kind: LikeKind, c: Cols, data: &str, added_at: i64) -> Liked {
    let item = match serde_json::from_str::<Item>(data) {
        Ok(item) if LikeKind::of(&item) == kind => item,
        _ => rebuild(kind, c),
    };
    Liked { item, added_at }
}

fn like_in(conn: &Connection, item: &Item, added_at: i64) -> Result<()> {
    let kind = LikeKind::of(item);
    let c = cols(item);
    if c.id.is_empty() {
        anyhow::bail!("bad_args: {} without an id", kind.as_str());
    }
    conn.execute(
        "INSERT INTO likes (kind, id, title, artists, album, thumbnail, duration_s, data, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(kind, id) DO UPDATE SET
            title = excluded.title, artists = excluded.artists, album = excluded.album,
            thumbnail = coalesce(excluded.thumbnail, thumbnail),
            duration_s = coalesce(excluded.duration_s, duration_s),
            data = excluded.data",
        params![
            kind.as_str(),
            c.id,
            c.title,
            serde_json::to_string(&c.artists)?,
            c.album.as_ref().map(serde_json::to_string).transpose()?,
            c.thumbnail,
            c.duration_s,
            serde_json::to_string(item)?,
            added_at,
        ],
    )?;
    Ok(())
}

fn item_order(conn: &Connection, id: &str) -> Result<Vec<String>> {
    if !Store::playlist_exists(conn, id)? {
        anyhow::bail!("not_found: no playlist {id}");
    }
    Ok(conn
        .prepare("SELECT video_id FROM playlist_items WHERE playlist_id = ?1 ORDER BY pos")?
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Rewrites `pos` as 0..n in `order` (or the current order).
fn renumber(conn: &Connection, id: &str, order: Option<Vec<String>>) -> Result<()> {
    let order = match order {
        Some(o) => o,
        None => item_order(conn, id)?,
    };
    let mut st = conn.prepare_cached(
        "UPDATE playlist_items SET pos = ?3 WHERE playlist_id = ?1 AND video_id = ?2",
    )?;
    for (i, v) in order.iter().enumerate() {
        st.execute(params![id, v, i as i64])?;
    }
    Ok(())
}

fn playlist_info(conn: &Connection, id: &str) -> Result<Option<PlaylistInfo>> {
    let row = conn
        .query_row(
            "SELECT name, description, cover, created_at, updated_at,
                (SELECT count(*) FROM playlist_items WHERE playlist_id = playlists.id)
             FROM playlists WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?;
    let Some((name, description, cover, created_at, updated_at, count)) = row else {
        return Ok(None);
    };
    let mut thumbnails: Vec<String> = Vec::new();
    let mut st = conn.prepare_cached(
        "SELECT thumbnail FROM playlist_items
         WHERE playlist_id = ?1 AND thumbnail IS NOT NULL ORDER BY pos",
    )?;
    let rows = st.query_map([id], |r| r.get::<_, String>(0))?;
    for t in rows {
        let t = t?;
        if !thumbnails.contains(&t) {
            thumbnails.push(t);
            if thumbnails.len() == 4 {
                break;
            }
        }
    }
    Ok(Some(PlaylistInfo {
        id: id.to_owned(),
        name,
        description,
        cover,
        thumbnails,
        track_count: count as u32,
        created_at,
        updated_at,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn track(id: &str, title: &str) -> Track {
        Track {
            video_id: id.into(),
            title: title.into(),
            artists: vec![ArtistRef {
                id: Some("UCa".into()),
                name: "A".into(),
            }],
            album: None,
            duration_s: Some(200),
            thumbnails: vec![Thumbnail {
                url: format!("https://i.ytimg.com/{id}.jpg"),
                width: 120,
                height: 120,
            }],
            explicit: false,
            kind: TrackKind::Song,
            year: None,
            views: None,
            playable: true,
        }
    }

    #[test]
    fn likes_keep_first_added_at_and_newest_first() {
        let s = Store::open_in_memory().unwrap();
        s.like(&Item::Track(track("a", "A")), 1000).unwrap();
        s.like(&Item::Track(track("b", "B")), 2000).unwrap();
        s.like(&Item::Track(track("a", "A2")), 3000).unwrap();
        let likes = s.likes(LikeKind::Track).unwrap();
        assert_eq!(likes.len(), 2);
        assert_eq!(likes[0].added_at, 2000);
        assert_eq!(likes[1].added_at, 1000);
        match &likes[1].item {
            Item::Track(t) => assert_eq!(t.title, "A2"),
            other => panic!("{other:?}"),
        }
        let ids = s
            .liked_ids(LikeKind::Track, &["a".into(), "z".into()])
            .unwrap();
        assert!(ids.contains("a") && !ids.contains("z"));
        assert!(s.unlike(LikeKind::Track, "a").unwrap());
        assert!(!s.unlike(LikeKind::Track, "a").unwrap());
        assert_eq!(s.liked_tracks_for_wave().unwrap()[0].1, 2);
    }

    #[test]
    fn artists_and_playlists_use_canonical_ids() {
        let s = Store::open_in_memory().unwrap();
        s.like(
            &Item::Artist(ArtistSummary {
                id: "MPLAUCx".into(),
                name: "X".into(),
                thumbnails: vec![],
                audience: None,
            }),
            1,
        )
        .unwrap();
        assert!(s.liked(LikeKind::Artist, "UCx").unwrap().is_some());
        assert!(s.liked(LikeKind::Artist, "MPLAUCx").unwrap().is_some());
        s.like(
            &Item::Playlist(PlaylistSummary {
                id: "VLPLy".into(),
                title: "Y".into(),
                author: None,
                thumbnails: vec![],
                track_count: None,
                subtitle: None,
            }),
            1,
        )
        .unwrap();
        assert!(s.unlike(LikeKind::Playlist, "PLy").unwrap());
    }

    #[test]
    fn broken_data_falls_back_to_columns() {
        let s = Store::open_in_memory().unwrap();
        s.like(&Item::Track(track("a", "A")), 1).unwrap();
        s.conn()
            .execute("UPDATE likes SET data = '{\"old\":1}'", [])
            .unwrap();
        let l = s.liked(LikeKind::Track, "a").unwrap().unwrap();
        match l.item {
            Item::Track(t) => {
                assert_eq!(t.title, "A");
                assert_eq!(t.artists[0].name, "A");
                assert_eq!(t.duration_s, Some(200));
                assert_eq!(t.thumbnails[0].url, "https://i.ytimg.com/a.jpg");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn playlist_lifecycle() {
        let s = Store::open_in_memory().unwrap();
        s.playlist_create("p", "Mine", "", None, 10, 10).unwrap();
        let tracks: Vec<(Track, i64)> = ["a", "b", "c", "a"]
            .iter()
            .map(|id| (track(id, id), 5))
            .collect();
        assert_eq!(s.playlist_add("p", &tracks, 20).unwrap(), 3);
        s.playlist_move("p", 0, 2, 30).unwrap();
        let ids = |s: &Store| -> Vec<String> {
            s.playlist("p")
                .unwrap()
                .unwrap()
                .tracks
                .into_iter()
                .map(|e| e.track.video_id)
                .collect()
        };
        assert_eq!(ids(&s), ["b", "c", "a"]);
        assert_eq!(s.playlist_remove("p", &["c".into()], 40).unwrap(), 1);
        assert_eq!(ids(&s), ["b", "a"]);
        assert_eq!(s.playlist_add("p", &[(track("d", "d"), 1)], 50).unwrap(), 1);
        assert_eq!(ids(&s), ["b", "a", "d"]);
        assert!(s.playlist_rename("p", Some("Ours"), None, 60).unwrap());
        let info = &s.playlists().unwrap()[0];
        assert_eq!(info.name, "Ours");
        assert_eq!(info.track_count, 3);
        assert_eq!(info.updated_at, 60);
        assert_eq!(info.thumbnails.len(), 3);
        s.pin(&Pin {
            kind: PinKind::UserPlaylist,
            id: "p".into(),
            title: "Ours".into(),
            thumbnail: None,
            pinned_at: 1,
        })
        .unwrap();
        assert!(s.playlist_delete("p").unwrap());
        assert!(s.playlist("p").unwrap().is_none());
        assert!(s.pins().unwrap().is_empty());
        assert!(s.playlist_add("p", &tracks, 1).is_err());
    }

    #[test]
    fn history_keeps_latest_play_per_track() {
        let s = Store::open_in_memory().unwrap();
        s.history_add(&track("a", "A"), 100).unwrap();
        s.history_add(&track("b", "B"), 200).unwrap();
        s.history_add(&track("a", "A"), 300).unwrap();
        s.history_add(&track("b", "B"), 50).unwrap();
        let h = s.history(10, 0).unwrap();
        let got: Vec<(&str, i64)> = h
            .iter()
            .map(|e| (e.track.video_id.as_str(), e.played_at))
            .collect();
        assert_eq!(got, [("a", 300), ("b", 200)]);
        assert_eq!(s.history(1, 1).unwrap()[0].track.video_id, "b");
    }

    #[test]
    fn pins_and_kv() {
        let s = Store::open_in_memory().unwrap();
        for (i, id) in ["x", "y"].iter().enumerate() {
            s.pin(&Pin {
                kind: PinKind::Album,
                id: (*id).into(),
                title: "T".into(),
                thumbnail: None,
                pinned_at: i as i64,
            })
            .unwrap();
        }
        assert_eq!(s.pins().unwrap()[0].id, "y");
        assert!(s.unpin(PinKind::Album, "y").unwrap());
        s.set_legacy(&[
            ("lastfm-session".into(), "{}".into()),
            ("maloja-token".into(), "t".into()),
        ])
        .unwrap();
        assert_eq!(s.legacy_prefixed("lastfm-").unwrap().len(), 1);
        assert_eq!(s.legacy("maloja-token").unwrap().as_deref(), Some("t"));
        assert_eq!(s.meta("k").unwrap(), None);
        s.set_meta("k", "v").unwrap();
        assert_eq!(s.meta("k").unwrap().as_deref(), Some("v"));
    }
}
