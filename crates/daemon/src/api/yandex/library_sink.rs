//! [`LibrarySink`] over the local library (`api/library.rs`).
//!
//! Likes go straight to the store (one wave sync and one `library_changed`
//! per kind at the end, not one per track); playlists go through
//! [`Library`]'s own methods. Which playlist an import made from which Yandex
//! playlist is kept in the store's `meta` table.

use volna_ytm::{AlbumSummary, ArtistSummary, Item, Track};

use super::import::LibrarySink;
use crate::api::library::{Library, LikeKind};

/// `meta` key of the playlist imported from `source` under `title`.
fn meta_key(source: &str, title: &str) -> String {
    format!("imported_playlist:{source}:{title}")
}

fn err(e: String) -> anyhow::Error {
    anyhow::anyhow!(e)
}

impl Library {
    /// Likes unless already liked; true when new.
    fn like_new(&self, kind: LikeKind, id: &str, item: Item, at: i64) -> anyhow::Result<bool> {
        if self.store().liked(kind, id)?.is_some() {
            return Ok(false);
        }
        self.store().like(&item, at)?;
        Ok(true)
    }
}

impl LibrarySink for Library {
    fn like_track(&self, track: &Track, added_at_ms: i64) -> anyhow::Result<bool> {
        self.like_new(
            LikeKind::Track,
            &track.video_id,
            Item::Track(track.clone()),
            added_at_ms,
        )
    }

    fn like_album(&self, album: &AlbumSummary) -> anyhow::Result<bool> {
        self.like_new(
            LikeKind::Album,
            &album.browse_id,
            Item::Album(album.clone()),
            crate::api::library::now_ms(),
        )
    }

    fn like_artist(&self, artist: &ArtistSummary) -> anyhow::Result<bool> {
        self.like_new(
            LikeKind::Artist,
            &artist.id,
            Item::Artist(artist.clone()),
            crate::api::library::now_ms(),
        )
    }

    fn imported_playlist(&self, source: &str, title: &str) -> anyhow::Result<Option<String>> {
        let Some(id) = self.store().meta(&meta_key(source, title))? else {
            return Ok(None);
        };
        // Deleted by the user since: the next import makes a new one.
        Ok(self.store().playlist(&id)?.map(|_| id))
    }

    fn create_playlist(
        &self,
        title: &str,
        description: &str,
        source: &str,
    ) -> anyhow::Result<String> {
        let description = (!description.is_empty()).then(|| description.to_owned());
        let info = self
            .playlist_create(title.to_owned(), description, Vec::new())
            .map_err(err)?;
        self.store().set_meta(&meta_key(source, title), &info.id)?;
        Ok(info.id)
    }

    fn add_to_playlist(&self, playlist: &str, tracks: &[Track]) -> anyhow::Result<()> {
        self.playlist_add(playlist, tracks.to_vec()).map_err(err)?;
        Ok(())
    }

    fn playlist_track_ids(&self, playlist: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .playlist_get(playlist)
            .map_err(err)?
            .tracks
            .into_iter()
            .map(|e| e.track.video_id)
            .collect())
    }

    fn reorder_playlist(&self, playlist: &str, video_ids: &[String]) -> anyhow::Result<()> {
        let mut current = self.playlist_track_ids(playlist)?;
        for (to, id) in video_ids.iter().enumerate() {
            let Some(from) = current.iter().position(|c| c == id) else {
                continue;
            };
            if from != to {
                self.playlist_move(playlist, from, to).map_err(err)?;
                let v = current.remove(from);
                current.insert(to.min(current.len()), v);
            }
        }
        Ok(())
    }

    fn finish(&self) -> anyhow::Result<()> {
        if let Err(e) = self.sync_wave_likes() {
            tracing::warn!("yandex import: wave likes: {e}");
        }
        for kind in [LikeKind::Track, LikeKind::Album, LikeKind::Artist] {
            self.changed(kind.as_str());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::library::Store;
    use crate::api::yandex::client::{Library as YLibrary, LikedTrack, Playlist};
    use crate::api::yandex::import::{Matched, save};
    use crate::api::yandex::matching::tests::{track, ytrack};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    fn yandex() -> YLibrary {
        YLibrary {
            login: None,
            liked_tracks: vec![LikedTrack {
                track: ytrack("One", None, &["A"], None),
                liked_at: Some("2024-01-02T00:00:00Z".into()),
            }],
            playlists: vec![Playlist {
                title: "Mix".into(),
                description: Some("d".into()),
                own: true,
                tracks: vec![
                    ytrack("One", None, &["A"], None),
                    ytrack("Two", None, &["A"], None),
                ],
            }],
            albums: vec![],
            artists: vec![],
        }
    }

    fn matched(ids: &[(&str, &str)]) -> Matched {
        Matched {
            tracks: ids
                .iter()
                .map(|(y, v)| (format!("y-{y}"), track(v, y, &["A"], None)))
                .collect::<HashMap<_, _>>(),
            total: 2,
            ..Matched::default()
        }
    }

    #[test]
    fn saves_into_the_sqlite_library_and_updates_on_reimport() {
        let lib = Library::new(Store::open_in_memory().unwrap(), None);
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        lib.set_listener(move |_, p| {
            sink.lock()
                .unwrap()
                .push(p["kind"].as_str().unwrap().to_owned())
        });

        let y = yandex();
        let first = save(&lib, &y, &matched(&[("One", "v1")])).unwrap();
        assert_eq!((first.liked_tracks, first.playlists), (1, 1));
        let liked = lib.likes(LikeKind::Track).unwrap();
        assert_eq!(liked[0].added_at, 1_704_153_600_000);
        assert!(events.lock().unwrap().contains(&"track".to_owned()));

        // A hand-added track, then a second import that finds "Two" as well.
        let id = lib.imported_playlist("yandex", "Mix").unwrap().unwrap();
        lib.playlist_add(&id, vec![track("hand", "Mine", &["Me"], None)])
            .unwrap();
        let second = save(&lib, &y, &matched(&[("One", "v1"), ("Two", "v2")])).unwrap();
        assert_eq!((second.liked_tracks, second.playlists), (0, 0));
        assert_eq!(lib.playlist_track_ids(&id).unwrap(), ["v1", "v2", "hand"]);
        assert_eq!(lib.store().playlists().unwrap().len(), 1);
        assert_eq!(lib.playlist_get(&id).unwrap().info.description, "d");

        // Deleted by the user: the next import makes it again.
        lib.playlist_delete(&id).unwrap();
        let third = save(&lib, &y, &matched(&[("One", "v1")])).unwrap();
        assert_eq!(third.playlists, 1);
    }
}
