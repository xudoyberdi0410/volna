//! The user library in the daemon (`docs/flutter-ui-plan.md` 3a): likes,
//! user playlists, history, pinned items and the one-shot import of the
//! webview library.
//!
//! Only SQLite and the wave engine are needed (no GStreamer, no socket), so
//! this moves to `volna-core` as is. The host gives it a listener for
//! `library_changed` events and records history from its player hooks.
//!
//! Commands (camelCase args, results as below):
//! - `library_get{kind}` → `[{item: Item, addedAt}]`, newest first. `kind`:
//!   `track` (videos included), `album`, `artist`, `playlist` (mixes included).
//! - `library_like{kind, item, on}` → `bool` (liked now). `item`: the
//!   `volna_ytm` model of that kind; for `on: false` its id string is enough.
//! - `library_is_liked{kind, ids}` → `{<id>: bool}`.
//! - `playlist_list` → `[PlaylistInfo]`; `playlist_get{id}` → `PlaylistDetail`.
//! - `playlist_create{name, description?, tracks?}` → `PlaylistInfo`.
//! - `playlist_rename{id, name?, description?}` → `PlaylistInfo`.
//! - `playlist_delete{id}` → `bool`.
//! - `playlist_add{id, tracks}` → added count; `playlist_remove{id, videoIds}`
//!   → removed count; `playlist_move{id, from, to}` → `null`.
//! - `history_get{limit?, offset?}` → `[{track, playedAt}]`; `history_clear`.
//! - `pin_set{kind, item, on}` → `bool`; `pin_list` → `[Pin]`.
//! - `legacy_status` → `{imported, importedAt}`;
//!   `legacy_import{dump}` or `legacy_import{path}` → `Report`;
//!   `legacy_export_file{dump}` → the path written.
//!
//! Likes feed the wave as `wave_sync_likes`/`wave_event{like}` did: a track
//! like is a wave `Like` event, an unlike clears the wave like, and an import
//! makes the wave's likes equal to the liked tracks.

pub mod legacy;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::{Value, json};
use volna_wave::{PlayerEvent, Wave, YtmSource};
use volna_ytm::{Item, Track};

use crate::dispatch::{arg, opt, out};
pub use store::{HistoryEntry, LikeKind, Liked, Pin, PinKind, PlaylistDetail, PlaylistInfo, Store};

/// `meta` key set once the webview library was imported (value: unix ms).
const LEGACY_IMPORTED: &str = "legacy_imported";
/// Name of the event the UIs refresh on: `library_changed{kind}`.
pub const EVENT: &str = "library_changed";

type Listener = Box<dyn Fn(&str, Value) + Send + Sync>;

pub struct Library {
    store: Store,
    /// Likes feed the wave; `None` in tests that do not care.
    wave: Option<Arc<Wave<YtmSource>>>,
    listener: Mutex<Option<Listener>>,
    /// Love on like and the scrobbler keys of `legacy_import`.
    scrobble: std::sync::OnceLock<volna_scrobble::Scrobbling>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn err(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    // Store errors that already carry a code keep it.
    if s.split_once(": ")
        .is_some_and(|(code, _)| matches!(code, "bad_args" | "not_found"))
    {
        s
    } else {
        format!("internal: {s}")
    }
}

/// A UUID v4 string for new playlists.
fn new_id() -> String {
    let mut b = [0u8; 16];
    for x in &mut b {
        *x = fastrand::u8(..);
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyStatus {
    pub imported: bool,
    pub imported_at: Option<i64>,
}

impl Library {
    /// `<data_dir>/library.sqlite3`.
    pub fn open(data_dir: &Path, wave: Option<Arc<Wave<YtmSource>>>) -> anyhow::Result<Self> {
        Ok(Self::new(
            Store::open(&data_dir.join("library.sqlite3"))?,
            wave,
        ))
    }

    pub fn new(store: Store, wave: Option<Arc<Wave<YtmSource>>>) -> Self {
        Self {
            store,
            wave,
            listener: Mutex::new(None),
            scrobble: std::sync::OnceLock::new(),
        }
    }

    /// Connects the scrobblers (`api::scrobble`); set once at startup.
    pub fn set_scrobble(&self, s: volna_scrobble::Scrobbling) {
        let _ = self.scrobble.set(s);
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Where `library_changed` goes (the connected window).
    pub fn set_listener(&self, f: impl Fn(&str, Value) + Send + Sync + 'static) {
        *self.listener.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    fn changed(&self, kind: &str) {
        if let Some(f) = self
            .listener
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            f(EVENT, json!({ "kind": kind }));
        }
    }

    // ---- likes -------------------------------------------------------------

    pub fn likes(&self, kind: LikeKind) -> Result<Vec<Liked>, String> {
        self.store.likes(kind).map_err(err)
    }

    /// Likes or unlikes; returns whether the item is liked now.
    pub async fn like(&self, kind: LikeKind, item: Value, on: bool) -> Result<bool, String> {
        let now = now_ms();
        if on {
            let item = parse_item(kind, item)?;
            self.store.like(&item, now).map_err(err)?;
            if let (Item::Track(t), Some(s)) = (&item, self.scrobble.get()) {
                super::scrobble::love_on_like(s, t, true);
            }
            if let (Item::Track(t), Some(wave)) = (&item, &self.wave)
                && let Err(e) = wave
                    .record(PlayerEvent::Like {
                        track: t.clone(),
                        liked: true,
                    })
                    .await
            {
                tracing::warn!("wave like failed: {e}");
            }
        } else {
            let id = item_id(kind, &item)?;
            if kind == LikeKind::Track
                && let Some(s) = self.scrobble.get()
                && let Some(t) = self.track_for_unlove(&id, &item)
            {
                super::scrobble::love_on_like(s, &t, false);
            }
            self.store.unlike(kind, &id).map_err(err)?;
            // Not `record(Like{liked: false})`: it would overwrite the wave's
            // track metadata with whatever (maybe bare) item came here.
            if kind == LikeKind::Track
                && let Some(wave) = &self.wave
                && let Err(e) = wave.store().set_like(&id, false, now / 1000)
            {
                tracing::warn!("wave unlike failed: {e}");
            }
        }
        self.changed(kind.as_str());
        Ok(on)
    }

    /// The unliked track with a title and artist for the scrobblers: the
    /// item sent, or else the stored like (an unlike may carry only the id).
    fn track_for_unlove(&self, id: &str, item: &Value) -> Option<Track> {
        if let Ok(Item::Track(t)) = parse_item(LikeKind::Track, item.clone())
            && !t.title.is_empty()
            && !t.artists.is_empty()
        {
            return Some(t);
        }
        self.store
            .likes(LikeKind::Track)
            .ok()?
            .into_iter()
            .find_map(|l| match l.item {
                Item::Track(t) if t.video_id == id => Some(t),
                _ => None,
            })
    }

    pub fn is_liked(
        &self,
        kind: LikeKind,
        ids: Vec<String>,
    ) -> Result<HashMap<String, bool>, String> {
        let liked = self.store.liked_ids(kind, &ids).map_err(err)?;
        Ok(ids
            .into_iter()
            .map(|id| {
                let on = liked.contains(&id);
                (id, on)
            })
            .collect())
    }

    /// Makes the wave's likes equal to the liked tracks (`wave_sync_likes`).
    pub fn sync_wave_likes(&self) -> Result<usize, String> {
        let Some(wave) = &self.wave else {
            return Ok(0);
        };
        let liked = self.store.liked_tracks_for_wave().map_err(err)?;
        wave.store()
            .replace_likes(&liked, now_ms() / 1000)
            .map_err(err)?;
        Ok(liked.len())
    }

    // ---- user playlists ------------------------------------------------------

    fn playlist_info(&self, id: &str) -> Result<PlaylistInfo, String> {
        self.store
            .playlist(id)
            .map_err(err)?
            .map(|d| d.info)
            .ok_or_else(|| format!("not_found: no playlist {id}"))
    }

    pub fn playlist_create(
        &self,
        name: String,
        description: Option<String>,
        tracks: Vec<Track>,
    ) -> Result<PlaylistInfo, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("bad_args: empty playlist name".into());
        }
        let (id, now) = (new_id(), now_ms());
        self.store
            .playlist_create(
                &id,
                name,
                description.as_deref().unwrap_or(""),
                None,
                now,
                now,
            )
            .map_err(err)?;
        if !tracks.is_empty() {
            let rows: Vec<(Track, i64)> = tracks.into_iter().map(|t| (t, now)).collect();
            self.store.playlist_add(&id, &rows, now).map_err(err)?;
        }
        self.changed("user_playlist");
        self.playlist_info(&id)
    }

    pub fn playlist_rename(
        &self,
        id: &str,
        name: Option<String>,
        description: Option<String>,
    ) -> Result<PlaylistInfo, String> {
        let name = name.map(|n| n.trim().to_owned());
        if name.as_deref() == Some("") {
            return Err("bad_args: empty playlist name".into());
        }
        if !self
            .store
            .playlist_rename(id, name.as_deref(), description.as_deref(), now_ms())
            .map_err(err)?
        {
            return Err(format!("not_found: no playlist {id}"));
        }
        self.changed("user_playlist");
        self.playlist_info(id)
    }

    pub fn playlist_delete(&self, id: &str) -> Result<bool, String> {
        let gone = self.store.playlist_delete(id).map_err(err)?;
        if gone {
            self.changed("user_playlist");
        }
        Ok(gone)
    }

    pub fn playlist_add(&self, id: &str, tracks: Vec<Track>) -> Result<usize, String> {
        let now = now_ms();
        let rows: Vec<(Track, i64)> = tracks.into_iter().map(|t| (t, now)).collect();
        let n = self.store.playlist_add(id, &rows, now).map_err(err)?;
        self.changed("user_playlist");
        Ok(n)
    }

    pub fn playlist_remove(&self, id: &str, video_ids: Vec<String>) -> Result<usize, String> {
        let n = self
            .store
            .playlist_remove(id, &video_ids, now_ms())
            .map_err(err)?;
        self.changed("user_playlist");
        Ok(n)
    }

    pub fn playlist_move(&self, id: &str, from: usize, to: usize) -> Result<(), String> {
        self.store
            .playlist_move(id, from, to, now_ms())
            .map_err(err)?;
        self.changed("user_playlist");
        Ok(())
    }

    pub fn playlist_get(&self, id: &str) -> Result<PlaylistDetail, String> {
        self.store
            .playlist(id)
            .map_err(err)?
            .ok_or_else(|| format!("not_found: no playlist {id}"))
    }

    // ---- history -------------------------------------------------------------

    /// A play from the player hooks (≥ 10 s played), `at` in unix ms.
    pub fn record_play(&self, track: &Track, at: i64) {
        match self.store.history_add(track, at) {
            Ok(()) => self.changed("history"),
            Err(e) => tracing::warn!("history: {e}"),
        }
    }

    // ---- pins ------------------------------------------------------------------

    pub fn pin_set(&self, kind: PinKind, item: Value, on: bool) -> Result<bool, String> {
        if on {
            let mut pin = pin_of(kind, &item)?;
            if kind == PinKind::UserPlaylist {
                let info = self.playlist_info(&pin.id)?;
                if pin.title.is_empty() {
                    pin.title = info.name;
                }
                if pin.thumbnail.is_none() {
                    pin.thumbnail = info.cover.or_else(|| info.thumbnails.into_iter().next());
                }
            }
            self.store.pin(&pin).map_err(err)?;
        } else {
            let id = pin_id(kind, &item)?;
            self.store.unpin(kind, &id).map_err(err)?;
        }
        self.changed("pinned");
        Ok(on)
    }

    // ---- legacy ----------------------------------------------------------------

    pub fn legacy_status(&self) -> Result<LegacyStatus, String> {
        let at = self.store.meta(LEGACY_IMPORTED).map_err(err)?;
        Ok(LegacyStatus {
            imported: at.is_some(),
            imported_at: at.and_then(|s| s.parse().ok()),
        })
    }

    /// Merges a webview library dump; safe to repeat (items already there
    /// keep their dates, playlists are matched by id).
    pub fn legacy_import(&self, dump: &Value) -> Result<legacy::Report, String> {
        let now = now_ms();
        let m = legacy::map(dump, now)?;
        self.store.like_many(&m.likes).map_err(err)?;
        for p in &m.playlists {
            self.store
                .playlist_create(
                    &p.id,
                    &p.name,
                    &p.description,
                    p.cover.as_deref(),
                    p.created_at,
                    p.updated_at,
                )
                .map_err(err)?;
            self.store
                .playlist_add(&p.id, &p.tracks, p.updated_at)
                .map_err(err)?;
        }
        for (t, at) in &m.history {
            self.store.history_add(t, *at).map_err(err)?;
        }
        for p in &m.pins {
            if p.kind == PinKind::UserPlaylist
                && !m.playlists.iter().any(|pl| pl.id == p.id)
                && self.store.playlist(&p.id).map_err(err)?.is_none()
            {
                continue;
            }
            self.store.pin(p).map_err(err)?;
        }
        self.store.set_legacy(&m.kv).map_err(err)?;
        if let Some(s) = self.scrobble.get() {
            let keys = m.kv.iter().cloned().collect();
            if let Err(e) = super::scrobble::import_legacy(s, &keys) {
                tracing::warn!("scrobbler settings import failed: {e}");
            }
        }
        self.store
            .set_meta(LEGACY_IMPORTED, &now.to_string())
            .map_err(err)?;
        if let Err(e) = self.sync_wave_likes() {
            tracing::warn!("wave like sync after import failed: {e}");
        }
        tracing::info!(report = ?m.report, "legacy library imported");
        self.changed("all");
        Ok(m.report)
    }

    pub fn legacy_import_file(&self, path: &Path) -> Result<legacy::Report, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("bad_args: cannot read {}: {e}", path.display()))?;
        let dump: Value = serde_json::from_str(&text)
            .map_err(|e| format!("bad_args: {} is not JSON: {e}", path.display()))?;
        self.legacy_import(&dump)
    }
}

/// Writes a dump to `dir/volna-library-<unix ms>.json`.
pub fn legacy_export_file(dir: &Path, dump: &Value) -> Result<PathBuf, String> {
    legacy::map(dump, 0)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("internal: {e}"))?;
    let path = dir.join(format!("volna-library-{}.json", now_ms()));
    let text = serde_json::to_string_pretty(dump).map_err(|e| format!("internal: {e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("internal: {}: {e}", path.display()))?;
    Ok(path)
}

/// The model of `kind` (a tagged `Item` is accepted too).
fn parse_item(kind: LikeKind, v: Value) -> Result<Item, String> {
    let bad = |e: serde_json::Error| format!("bad_args: invalid {} item: {e}", kind.as_str());
    if let Ok(item) = serde_json::from_value::<Item>(v.clone()) {
        if LikeKind::of(&item) == kind {
            return Ok(item);
        }
        return Err(format!("bad_args: item is not a {}", kind.as_str()));
    }
    Ok(match kind {
        LikeKind::Track => Item::Track(serde_json::from_value(v).map_err(bad)?),
        LikeKind::Album => Item::Album(serde_json::from_value(v).map_err(bad)?),
        LikeKind::Artist => Item::Artist(serde_json::from_value(v).map_err(bad)?),
        LikeKind::Playlist => Item::Playlist(serde_json::from_value(v).map_err(bad)?),
    })
}

/// The first non-empty string among `keys` (snake_case and camelCase model
/// fields), or the value itself when it is a string.
fn id_field(v: &Value, keys: &[&str]) -> Option<String> {
    if let Value::String(s) = v {
        return Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    }
    keys.iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn item_id(kind: LikeKind, v: &Value) -> Result<String, String> {
    id_field(v, &["video_id", "videoId", "browse_id", "browseId", "id"])
        .map(|id| store::canonical_id(kind, &id))
        .ok_or_else(|| format!("bad_args: {} item without an id", kind.as_str()))
}

fn pin_id(kind: PinKind, v: &Value) -> Result<String, String> {
    let id = id_field(v, &["browse_id", "browseId", "id"])
        .ok_or_else(|| format!("bad_args: {} pin without an id", kind.as_str()))?;
    Ok(match kind {
        PinKind::Artist => store::canonical_id(LikeKind::Artist, &id),
        PinKind::Playlist => store::canonical_id(LikeKind::Playlist, &id),
        _ => id,
    })
}

/// A pin from a model (`AlbumSummary`, `ArtistSummary`, `PlaylistSummary`,
/// `PlaylistInfo`) or `{id, title, thumbnail}`.
fn pin_of(kind: PinKind, v: &Value) -> Result<Pin, String> {
    let id = pin_id(kind, v)?;
    let title = ["title", "name"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or_default()
        .to_owned();
    let thumbnail = v
        .get("thumbnail")
        .or_else(|| v.get("cover"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let thumbs: Vec<volna_ytm::Thumbnail> =
                serde_json::from_value(v.get("thumbnails")?.clone()).ok()?;
            store::best_thumbnail(&thumbs).map(str::to_owned)
        });
    Ok(Pin {
        kind,
        id,
        title,
        thumbnail,
        pinned_at: now_ms(),
    })
}

/// Library commands; `None` when `cmd` is not one of them.
pub async fn call(lib: &Library, cmd: &str, a: &Value) -> Option<Result<Value, String>> {
    Some(match cmd {
        "library_get" => match arg::<LikeKind>(a, "kind") {
            Ok(kind) => out(lib.likes(kind)),
            Err(e) => Err(e),
        },
        "library_like" => match (arg(a, "kind"), arg(a, "on")) {
            (Ok(kind), Ok(on)) => {
                let item = a.get("item").cloned().unwrap_or(Value::Null);
                out(lib.like(kind, item, on).await)
            }
            (Err(e), _) | (_, Err(e)) => Err(e),
        },
        "library_is_liked" => (|| out(lib.is_liked(arg(a, "kind")?, arg(a, "ids")?)))(),
        "playlist_list" => out(lib.store.playlists().map_err(err)),
        "playlist_get" => (|| out(lib.playlist_get(&arg::<String>(a, "id")?)))(),
        "playlist_create" => (|| {
            out(lib.playlist_create(
                arg(a, "name")?,
                opt(a, "description")?,
                opt(a, "tracks")?.unwrap_or_default(),
            ))
        })(),
        "playlist_rename" => (|| {
            out(lib.playlist_rename(
                &arg::<String>(a, "id")?,
                opt(a, "name")?,
                opt(a, "description")?,
            ))
        })(),
        "playlist_delete" => (|| out(lib.playlist_delete(&arg::<String>(a, "id")?)))(),
        "playlist_add" => (|| out(lib.playlist_add(&arg::<String>(a, "id")?, arg(a, "tracks")?)))(),
        "playlist_remove" => {
            (|| out(lib.playlist_remove(&arg::<String>(a, "id")?, arg(a, "videoIds")?)))()
        }
        "playlist_move" => {
            (|| out(lib.playlist_move(&arg::<String>(a, "id")?, arg(a, "from")?, arg(a, "to")?)))()
        }
        "history_get" => (|| {
            let limit = opt::<usize>(a, "limit")?
                .unwrap_or(100)
                .min(store::HISTORY_CAP);
            let offset = opt::<usize>(a, "offset")?.unwrap_or(0);
            out(lib.store.history(limit, offset).map_err(err))
        })(),
        "history_clear" => {
            let r = lib.store.history_clear().map_err(err);
            if r.is_ok() {
                lib.changed("history");
            }
            out(r)
        }
        "pin_set" => (|| {
            out(lib.pin_set(
                arg(a, "kind")?,
                a.get("item").cloned().unwrap_or(Value::Null),
                arg(a, "on")?,
            ))
        })(),
        "pin_list" => out(lib.store.pins().map_err(err)),
        "legacy_status" => out(lib.legacy_status()),
        "legacy_import" => (|| match (a.get("dump"), opt::<String>(a, "path")?) {
            (Some(dump), _) if !dump.is_null() => out(lib.legacy_import(dump)),
            (_, Some(path)) => out(lib.legacy_import_file(Path::new(&path))),
            _ => Err("bad_args: legacy_import needs `dump` or `path`".into()),
        })(),
        "legacy_export_file" => (|| {
            let dump = a
                .get("dump")
                .filter(|d| !d.is_null())
                .ok_or("bad_args: legacy_export_file needs `dump`")?;
            let dir = dirs::download_dir()
                .or_else(dirs::home_dir)
                .ok_or("internal: no Downloads or home directory")?;
            out(legacy_export_file(&dir, dump).map(|p| p.display().to_string()))
        })(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use store::tests::track;

    fn wave() -> Arc<Wave<YtmSource>> {
        let ytm = volna_ytm::YtmClient::new(reqwest::Client::new());
        Arc::new(Wave::new(
            volna_wave::Store::open_in_memory().unwrap(),
            YtmSource::new(ytm),
        ))
    }

    type Events = Arc<Mutex<Vec<Value>>>;

    fn lib() -> (Library, Arc<Wave<YtmSource>>, Events) {
        let w = wave();
        let lib = Library::new(Store::open_in_memory().unwrap(), Some(w.clone()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        lib.set_listener(move |name, p| {
            assert_eq!(name, EVENT);
            ev.lock().unwrap().push(p);
        });
        (lib, w, events)
    }

    fn wave_likes(w: &Wave<YtmSource>) -> Vec<String> {
        let mut ids: Vec<String> = w
            .store()
            .snapshot(0)
            .unwrap()
            .likes
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids
    }

    async fn run(lib: &Library, cmd: &str, a: Value) -> Result<Value, String> {
        call(lib, cmd, &a).await.expect("a library command")
    }

    #[tokio::test]
    async fn likes_go_to_the_library_and_the_wave() {
        let (lib, w, events) = lib();
        let t = serde_json::to_value(track("dQw4w9WgXcQ", "Song")).unwrap();
        let r = run(
            &lib,
            "library_like",
            json!({"kind": "track", "item": t, "on": true}),
        )
        .await
        .unwrap();
        assert_eq!(r, json!(true));
        assert_eq!(wave_likes(&w), ["dQw4w9WgXcQ"]);
        let got = run(&lib, "library_get", json!({"kind": "track"}))
            .await
            .unwrap();
        assert_eq!(got[0]["item"]["type"], "track");
        assert_eq!(got[0]["item"]["title"], "Song");
        assert!(got[0]["addedAt"].as_i64().unwrap() > 0);
        let liked = run(
            &lib,
            "library_is_liked",
            json!({"kind": "track", "ids": ["dQw4w9WgXcQ", "zzzzzzzzzzz"]}),
        )
        .await
        .unwrap();
        assert_eq!(liked, json!({"dQw4w9WgXcQ": true, "zzzzzzzzzzz": false}));

        run(
            &lib,
            "library_like",
            json!({"kind": "track", "item": "dQw4w9WgXcQ", "on": false}),
        )
        .await
        .unwrap();
        assert!(wave_likes(&w).is_empty());
        assert_eq!(
            run(&lib, "library_get", json!({"kind": "track"}))
                .await
                .unwrap(),
            json!([])
        );
        assert_eq!(
            *events.lock().unwrap(),
            [json!({"kind": "track"}), json!({"kind": "track"})]
        );
    }

    #[tokio::test]
    async fn albums_take_their_model_or_a_tagged_item() {
        let (lib, w, _) = lib();
        let album = json!({
            "browse_id": "MPREb_1", "playlist_id": null, "title": "LP", "kind": "album",
            "artists": [], "year": 2020, "thumbnails": [], "explicit": false
        });
        run(
            &lib,
            "library_like",
            json!({"kind": "album", "item": album, "on": true}),
        )
        .await
        .unwrap();
        let mut tagged = album.clone();
        tagged["type"] = json!("album");
        tagged["browse_id"] = json!("MPREb_2");
        run(
            &lib,
            "library_like",
            json!({"kind": "album", "item": tagged, "on": true}),
        )
        .await
        .unwrap();
        assert_eq!(lib.likes(LikeKind::Album).unwrap().len(), 2);
        assert!(wave_likes(&w).is_empty(), "only tracks feed the wave");
        let e = run(
            &lib,
            "library_like",
            json!({"kind": "artist", "item": tagged, "on": true}),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("bad_args"), "{e}");
    }

    #[tokio::test]
    async fn playlist_commands() {
        let (lib, _, _) = lib();
        let t = |id: &str| serde_json::to_value(track(id, id)).unwrap();
        let info = run(
            &lib,
            "playlist_create",
            json!({"name": " Road ", "tracks": [t("aaaaaaaaaaa")]}),
        )
        .await
        .unwrap();
        let id = info["id"].as_str().unwrap().to_owned();
        assert_eq!(id.len(), 36);
        assert_eq!(
            (info["name"].clone(), info["trackCount"].clone()),
            (json!("Road"), json!(1))
        );
        let n = run(
            &lib,
            "playlist_add",
            json!({"id": id, "tracks": [t("bbbbbbbbbbb"), t("aaaaaaaaaaa")]}),
        )
        .await
        .unwrap();
        assert_eq!(n, json!(1));
        run(&lib, "playlist_move", json!({"id": id, "from": 1, "to": 0}))
            .await
            .unwrap();
        let got = run(&lib, "playlist_get", json!({"id": id})).await.unwrap();
        assert_eq!(got["tracks"][0]["track"]["video_id"], "bbbbbbbbbbb");
        assert!(got["tracks"][0]["addedAt"].is_i64());
        run(
            &lib,
            "playlist_rename",
            json!({"id": id, "description": "d"}),
        )
        .await
        .unwrap();
        run(
            &lib,
            "pin_set",
            json!({"kind": "user_playlist", "item": id, "on": true}),
        )
        .await
        .unwrap();
        let pins = run(&lib, "pin_list", json!({})).await.unwrap();
        assert_eq!(pins[0]["title"], "Road");
        assert_eq!(pins[0]["kind"], "user_playlist");
        assert_eq!(
            run(
                &lib,
                "playlist_remove",
                json!({"id": id, "videoIds": ["aaaaaaaaaaa"]})
            )
            .await
            .unwrap(),
            json!(1)
        );
        assert_eq!(
            run(&lib, "playlist_list", json!({})).await.unwrap()[0]["description"],
            "d"
        );
        assert_eq!(
            run(&lib, "playlist_delete", json!({"id": id}))
                .await
                .unwrap(),
            json!(true)
        );
        let e = run(&lib, "playlist_get", json!({"id": id}))
            .await
            .unwrap_err();
        assert!(e.starts_with("not_found"), "{e}");
        assert!(call(&lib, "wave_status", &json!({})).await.is_none());
    }

    #[tokio::test]
    async fn history_from_plays() {
        let (lib, _, events) = lib();
        lib.record_play(&track("aaaaaaaaaaa", "A"), 10);
        lib.record_play(&track("bbbbbbbbbbb", "B"), 20);
        let h = run(&lib, "history_get", json!({"limit": 1})).await.unwrap();
        assert_eq!(h.as_array().unwrap().len(), 1);
        assert_eq!(h[0]["track"]["video_id"], "bbbbbbbbbbb");
        assert_eq!(h[0]["playedAt"], 20);
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn legacy_import_is_one_shot_and_idempotent() {
        let (lib, w, events) = lib();
        assert_eq!(
            run(&lib, "legacy_status", json!({})).await.unwrap()["imported"],
            json!(false)
        );
        let dump = legacy::tests::sample();
        let r = run(&lib, "legacy_import", json!({"dump": dump}))
            .await
            .unwrap();
        assert_eq!(r["tracks"], 2);
        let st = run(&lib, "legacy_status", json!({})).await.unwrap();
        assert_eq!(st["imported"], json!(true));
        assert!(st["importedAt"].is_i64());

        // A second run (e.g. the same file from Settings) changes nothing.
        run(&lib, "legacy_import", json!({"dump": dump}))
            .await
            .unwrap();
        assert_eq!(lib.likes(LikeKind::Track).unwrap().len(), 2);
        assert_eq!(lib.likes(LikeKind::Playlist).unwrap().len(), 2);
        let pls = lib.store.playlists().unwrap();
        assert_eq!(pls.len(), 1);
        assert_eq!(pls[0].track_count, 2);
        assert_eq!(lib.store.history(10, 0).unwrap().len(), 2);
        assert_eq!(lib.store.pins().unwrap().len(), 2);
        assert_eq!(
            lib.store.legacy("listenbrainz-token").unwrap().as_deref(),
            Some("tok")
        );
        assert_eq!(wave_likes(&w), ["9bZkp7q19f0", "dQw4w9WgXcQ"]);
        assert_eq!(events.lock().unwrap().last(), Some(&json!({"kind": "all"})));

        // From a file, as the Settings fallback.
        let dir = tempfile::tempdir().unwrap();
        let path = legacy_export_file(dir.path(), &dump).unwrap();
        let fresh = Library::new(Store::open_in_memory().unwrap(), None);
        let r = run(
            &fresh,
            "legacy_import",
            json!({"path": path.display().to_string()}),
        )
        .await
        .unwrap();
        assert_eq!(r["userPlaylists"], 1);
        assert!(legacy_export_file(dir.path(), &json!({"x": 1})).is_err());
    }
}
