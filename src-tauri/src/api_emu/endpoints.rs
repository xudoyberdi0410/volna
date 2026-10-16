//! One method per hifi-api endpoint (contract §3) plus the Volna extras.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use volna_ytm::{
    Album, AlbumKind, AlbumSummary, Artist, ArtistRef, Item, Playlist, SearchFilter, Track,
};

use super::shape::{self, TrackCtx, VERSION};
use super::{ApiEmu, ApiError, ApiResult, Query};

/// Tracks per `/playlist/` page (YouTube's own page size).
const PLAYLIST_PAGE: usize = 100;
/// Hard cap when a playlist's size is unknown and must be loaded up front.
const PLAYLIST_MAX: usize = 2000;
/// `/recommendations/` length (contract §3.3: about 20–25).
const RECOMMENDATIONS: usize = 25;
/// `/album/similar/` length.
const SIMILAR_ALBUMS: usize = 12;

impl ApiEmu {
    pub(super) async fn dispatch(&self, path: &str, q: &Query) -> ApiResult<Value> {
        match path {
            "/" => Ok(json!({ "version": VERSION, "Repo": "volna" })),
            "/search" => self.search(q).await,
            "/search/suggest" => self.suggest(q).await,
            "/info" => self.info(q.id("id")?).await,
            "/track" => self.track_manifest(q.id("id")?, q.get("quality")),
            "/recommendations" => self.recommendations(q.id("id")?).await,
            "/album" => self.album_endpoint(q).await,
            "/album/similar" => self.similar_albums(q.id("id")?).await,
            "/artist" => self.artist_endpoint(q).await,
            "/artist/similar" => self.similar_artists(q.id("id")?).await,
            "/artist/bio" => self.artist_bio(q.id("id")?).await,
            "/playlist" => self.playlist_endpoint(q.id("id")?, q.usize("offset")).await,
            "/mix" => self.mix(q.id("id")?).await,
            "/lyrics" => self.lyrics(q.id("id")?).await,
            "/home" => self.home().await,
            "/explore" => self.explore().await,
            "/explore/genre" => self.explore_genre(q).await,
            // No TIDAL manifests or music videos in v1 (contract §3.11, §4.4).
            "/trackManifests" | "/video" => {
                Err(ApiError::NotFound(format!("{path} is not served")))
            }
            _ => Err(ApiError::NotFound(format!("unknown endpoint {path}"))),
        }
    }

    // ---- cached YouTube Music pages -------------------------------------

    async fn album(&self, id: &str) -> ApiResult<Arc<Album>> {
        if let Some(a) = self.albums.get(&id.to_owned()) {
            return Ok(a);
        }
        let album = Arc::new(self.ytm.album(id).await?);
        self.albums.insert(id.to_owned(), album.clone());
        if album.browse_id != id {
            self.albums.insert(album.browse_id.clone(), album.clone());
        }
        Ok(album)
    }

    async fn artist(&self, id: &str) -> ApiResult<Arc<Artist>> {
        let id = shape::clean_artist_id(id);
        if let Some(a) = self.artists.get(&id.to_owned()) {
            return Ok(a);
        }
        let artist = Arc::new(self.ytm.artist(id).await?);
        self.artists.insert(id.to_owned(), artist.clone());
        Ok(artist)
    }

    /// The tracks of a radio/watch queue: the first `next` page plus one
    /// continuation (about 50 tracks). `RDAMVM<v>` radios start with `v`.
    async fn radio(&self, mix_id: &str) -> ApiResult<Arc<Vec<Track>>> {
        if let Some(r) = self.radios.get(&mix_id.to_owned()) {
            return Ok(r);
        }
        let queue = match mix_id.strip_prefix("RDAMVM") {
            Some(video_id) => self.ytm.radio(video_id).await?,
            None => self.ytm.watch_playlist(mix_id).await?,
        };
        let mut tracks = queue.tracks;
        if let Some(token) = queue.continuation {
            match self.ytm.queue_more(&token).await {
                Ok(page) => tracks.extend(page.tracks),
                Err(e) => tracing::debug!(mix_id, "radio continuation failed: {e}"),
            }
        }
        dedupe_tracks(&mut tracks);
        if tracks.is_empty() {
            return Err(ApiError::NotFound(format!("radio {mix_id}")));
        }
        let tracks = Arc::new(tracks);
        self.radios.insert(mix_id.to_owned(), tracks.clone());
        Ok(tracks)
    }

    /// A playlist with at least `want` tracks loaded (or all of them).
    async fn playlist(&self, id: &str, want: usize) -> ApiResult<Playlist> {
        let id = id.strip_prefix("VL").unwrap_or(id).to_owned();
        let cell = match self.playlists.get(&id) {
            Some(c) => c,
            None => {
                let first = self.ytm.playlist(&id).await?;
                let cell = Arc::new(tokio::sync::Mutex::new(first));
                self.playlists.insert(id.clone(), cell.clone());
                cell
            }
        };
        let mut pl = cell.lock().await;
        // Unknown size: load everything so numberOfTracks is right on the
        // first page (the UI reads it only once).
        let want = if pl.track_count.is_none() {
            PLAYLIST_MAX
        } else {
            want.min(PLAYLIST_MAX)
        };
        while pl.tracks.len() < want
            && let Some(token) = pl.continuation.clone()
        {
            let page = self.ytm.playlist_more(&token).await?;
            pl.continuation = page.continuation;
            if page.tracks.is_empty() {
                pl.continuation = None;
            }
            pl.tracks.extend(page.tracks);
        }
        Ok(pl.clone())
    }

    // ---- endpoints -------------------------------------------------------

    /// `/search/?q|s|a|al|p|v|i=` (§3.1). Every bucket is always present,
    /// because the UI falls back to "any bucket" when the asked one is missing.
    async fn search(&self, q: &Query) -> ApiResult<Value> {
        let mut buckets = SearchBuckets::default();
        if let Some(term) = q.get("q").filter(|t| !t.trim().is_empty()) {
            // Four filtered searches instead of the unfiltered one: those
            // carry album and duration on songs and ~20 items per type.
            let (songs, artists, albums, playlists) = tokio::join!(
                self.ytm.search(term, SearchFilter::Songs),
                self.ytm.search(term, SearchFilter::Artists),
                self.ytm.search(term, SearchFilter::Albums),
                self.ytm.search(term, SearchFilter::Playlists),
            );
            let mut failures = Vec::new();
            for page in [songs, artists, albums, playlists] {
                match page {
                    Ok(p) => buckets.add(p.items),
                    Err(e) => failures.push(e),
                }
            }
            // The UI disables combined search for the session on an error,
            // so only fail when nothing at all came back.
            if failures.len() == 4 {
                return Err(failures.remove(0).into());
            }
        } else {
            let scoped = [
                ("s", SearchFilter::Songs),
                ("a", SearchFilter::Artists),
                ("al", SearchFilter::Albums),
                ("p", SearchFilter::Playlists),
            ];
            if let Some((term, filter)) = scoped
                .iter()
                .find_map(|(k, f)| q.get(k).map(|t| (t, *f)))
                .filter(|(t, _)| !t.trim().is_empty())
            {
                buckets.add(self.ytm.search(term, filter).await?.items);
            }
            // `v` (videos) and `i` (ISRC) have no YouTube Music equivalent.
        }
        Ok(json!({ "version": VERSION, "data": buckets.into_json() }))
    }

    /// `/search/suggest/?q=` → completed queries only (`kind: "term"`).
    async fn suggest(&self, q: &Query) -> ApiResult<Value> {
        let term = q.get("q").unwrap_or_default();
        if term.trim().is_empty() {
            return Ok(json!({ "version": VERSION, "data": [] }));
        }
        let s = self.ytm.search_suggestions(term).await?;
        let data: Vec<Value> = s
            .queries
            .iter()
            .map(|t| json!({ "kind": "term", "searchTerm": t, "displayTerm": t }))
            .collect();
        Ok(json!({ "version": VERSION, "data": data }))
    }

    /// `/info/?id=<videoId>` (§3.2).
    async fn info(&self, id: &str) -> ApiResult<Value> {
        let details = self.ytm.track(id).await?;
        Ok(json!({ "version": VERSION, "data": shape::track(&details.track, TrackCtx::default()) }))
    }

    /// `/track/?id=` as a TIDAL "BTS" manifest pointing at the stream (§4.4).
    /// The patched UI plays without it; kept for code paths that still ask.
    fn track_manifest(&self, id: &str, quality: Option<&str>) -> ApiResult<Value> {
        let low = quality == Some("LOW");
        let url = match self.stream_base.get() {
            // `<audio>` cannot play `volna://` on WebKitGTK (docs/streams.md).
            Some(base) => format!("{base}/{id}{}", if low { "?q=low" } else { "" }),
            None => shape::volna_url(
                "stream",
                &format!("/{id}"),
                if low { &[("q", "low")] } else { &[] },
            ),
        };
        let manifest = json!({
            "mimeType": "audio/webm",
            "codecs": "opus",
            "encryptionType": "NONE",
            "urls": [url],
        });
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(manifest.to_string());
        Ok(json!({ "version": VERSION, "data": {
            "trackId": id,
            "assetPresentation": "FULL",
            "audioMode": "STEREO",
            "audioQuality": if low { "LOW" } else { "HIGH" },
            "manifestMimeType": "application/vnd.tidal.bts",
            "manifest": encoded,
        }}))
    }

    /// `/recommendations/?id=<videoId>`: the song radio without the seed (§3.3).
    async fn recommendations(&self, id: &str) -> ApiResult<Value> {
        let radio = self.radio(&format!("RDAMVM{id}")).await?;
        let items: Vec<Value> = radio
            .iter()
            .filter(|t| t.video_id != id)
            .take(RECOMMENDATIONS)
            .map(|t| shape::track_item(t, TrackCtx::default()))
            .collect();
        let n = items.len();
        Ok(json!({ "version": VERSION, "data": { "items": items, "totalNumberOfItems": n } }))
    }

    /// `/album/?id=<MPREb_…|OLAK5uy_…>[&offset&limit]` (§3.4). Everything is
    /// returned on the first page, so `numberOfTracks == items.len()`.
    async fn album_endpoint(&self, q: &Query) -> ApiResult<Value> {
        let album = self.album(q.id("id")?).await?;
        let offset = q.usize("offset").unwrap_or(0);
        let limit = q.usize("limit").unwrap_or(usize::MAX);
        Ok(json!({ "version": VERSION, "data": shape::album_page(&album, offset, limit) }))
    }

    /// `/album/similar/?id=` (§3.8). YouTube Music has no such surface:
    /// use the albums of the first track's "Related" tab, then the albums
    /// of its radio (video-based radios carry no albums at all).
    async fn similar_albums(&self, id: &str) -> ApiResult<Value> {
        let album = self.album(id).await?;
        let Some(seed) = album.tracks.iter().find(|t| t.playable) else {
            return Ok(json!({ "version": VERSION, "albums": [] }));
        };
        let radio_id = format!("RDAMVM{}", seed.video_id);
        let (related, radio) =
            tokio::join!(self.ytm.related(&seed.video_id), self.radio(&radio_id));
        let mut candidates = match related {
            Ok(r) => r.albums,
            Err(e) => {
                tracing::debug!(id, "related tab failed: {e}");
                Vec::new()
            }
        };
        if let Ok(radio) = radio {
            candidates.extend(albums_of(&radio, &album.browse_id));
        }
        let mut seen = HashSet::from([album.browse_id.clone()]);
        let albums: Vec<Value> = candidates
            .iter()
            .filter(|a| seen.insert(a.browse_id.clone()))
            .take(SIMILAR_ALBUMS)
            .map(|a| shape::album_summary(a, None))
            .collect();
        Ok(json!({ "version": VERSION, "albums": albums }))
    }

    /// `/artist/?id=` / `?f=` (§3.5). With `offset`/`limit` it pages the
    /// full "Top songs" list instead.
    async fn artist_endpoint(&self, q: &Query) -> ApiResult<Value> {
        let id = q.id("id").or_else(|_| q.id("f"))?;
        let artist = self.artist(id).await?;
        let owner = ArtistRef {
            id: Some(shape::clean_artist_id(&artist.id).to_owned()),
            name: artist.name.clone(),
        };

        if q.get("offset").is_some() || q.get("limit").is_some() {
            let offset = q.usize("offset").unwrap_or(0);
            let limit = q.usize("limit").unwrap_or(15).clamp(1, 100);
            let all = match &artist.top_songs_playlist_id {
                Some(pid) => match self.playlist(pid, offset + limit).await {
                    Ok(pl) => pl.tracks,
                    Err(e) => {
                        tracing::debug!(pid, "top songs playlist failed: {e}");
                        artist.top_songs.clone()
                    }
                },
                None => artist.top_songs.clone(),
            };
            let tracks: Vec<Value> = all
                .iter()
                .enumerate()
                .skip(offset)
                .take(limit)
                .map(|(i, t)| top_track(t, &owner, 10_000 - i as u32))
                .collect();
            return Ok(json!({ "version": VERSION, "tracks": tracks }));
        }

        let owner_pair = (owner.id.as_deref().unwrap_or_default(), owner.name.as_str());
        let albums: Vec<Value> = artist
            .albums
            .iter()
            .chain(&artist.singles)
            .map(|a| shape::album_summary(a, Some(owner_pair)))
            .collect();
        let tracks: Vec<Value> = artist
            .top_songs
            .iter()
            .enumerate()
            .map(|(i, t)| top_track(t, &owner, 100u32.saturating_sub(i as u32)))
            .collect();
        Ok(json!({
            "version": VERSION,
            "artist": shape::artist_profile(&artist),
            "cover": null,
            "albums": { "items": albums },
            "tracks": tracks,
        }))
    }

    /// `/artist/similar/?id=` (§3.9): "Fans might also like".
    async fn similar_artists(&self, id: &str) -> ApiResult<Value> {
        let artist = self.artist(id).await?;
        let artists: Vec<Value> = artist.related.iter().map(shape::artist_summary).collect();
        Ok(json!({ "version": VERSION, "artists": artists }))
    }

    /// `/artist/bio/?id=` (§3.10).
    async fn artist_bio(&self, id: &str) -> ApiResult<Value> {
        let artist = self.artist(id).await?;
        let text = artist
            .description
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| ApiError::NotFound(format!("biography of {id}")))?;
        Ok(json!({ "version": VERSION, "data": { "text": text, "source": "YouTube Music" } }))
    }

    /// `/playlist/?id=[&offset=]` (§3.6). Radio ids are served as a
    /// one-page playlist made from the mix (§9.4).
    async fn playlist_endpoint(&self, id: &str, offset: Option<usize>) -> ApiResult<Value> {
        let id = id.strip_prefix("VL").unwrap_or(id);
        let offset = offset.unwrap_or(0);
        if shape::is_radio_id(id) {
            let mix = self.mix(id).await?;
            let tracks = mix["items"].as_array().cloned().unwrap_or_default();
            let mut header = shape::playlist_header(
                id,
                mix["mix"]["title"].as_str().unwrap_or("Radio"),
                None,
                None,
                &[],
                Some(tracks.len() as u32),
            );
            header["squareImage"] = mix["mix"]["images"]["LARGE"]["url"].clone();
            header["image"] = header["squareImage"].clone();
            let items: Vec<Value> = if offset == 0 {
                tracks
                    .into_iter()
                    .map(|t| json!({ "item": t, "type": "track", "cut": null }))
                    .collect()
            } else {
                Vec::new()
            };
            return Ok(json!({ "version": VERSION, "playlist": header, "items": items }));
        }

        let pl = self.playlist(id, offset + PLAYLIST_PAGE).await?;
        let total = match (pl.track_count, &pl.continuation) {
            (_, None) => pl.tracks.len(),
            (Some(n), Some(_)) => (n as usize).max(pl.tracks.len()),
            (None, Some(_)) => pl.tracks.len(),
        };
        let items: Vec<Value> = pl
            .tracks
            .iter()
            .skip(offset)
            .take(PLAYLIST_PAGE)
            .map(|t| {
                let mut v = shape::track_item(t, TrackCtx::default());
                v["cut"] = Value::Null;
                v
            })
            .collect();
        let header = shape::playlist_header(
            &pl.id,
            &pl.title,
            pl.author.as_ref(),
            pl.description.as_deref(),
            &pl.thumbnails,
            Some(total as u32),
        );
        Ok(json!({ "version": VERSION, "playlist": header, "items": items }))
    }

    /// `/mix/?id=` (§3.7): a song radio, artist radio or any watch queue.
    /// No `data` envelope.
    async fn mix(&self, id: &str) -> ApiResult<Value> {
        let tracks = self.radio(id).await?;
        let seed = &tracks[0];
        let artists = seed
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let title = if id.starts_with("RDAMVM") {
            format!("{} Radio", seed.title)
        } else if id.starts_with("RDEM") || id.starts_with("RDAO") {
            format!(
                "{} Radio",
                seed.artists.first().map_or("Artist", |a| &a.name)
            )
        } else {
            "Mix".to_owned()
        };
        let items: Vec<Value> = tracks
            .iter()
            .map(|t| shape::track(t, TrackCtx::default()))
            .collect();
        Ok(json!({
            "version": VERSION,
            "mix": shape::mix_header(id, &title, &artists, shape::cover(&seed.thumbnails)),
            "items": items,
        }))
    }

    /// `/lyrics/?id=<videoId>` → `{data: {text, source}}`, 404 when none.
    async fn lyrics(&self, id: &str) -> ApiResult<Value> {
        let lyrics = self
            .ytm
            .lyrics(id)
            .await?
            .ok_or_else(|| ApiError::NotFound(format!("lyrics of {id}")))?;
        Ok(json!({ "version": VERSION, "data": {
            "text": lyrics.text,
            "source": lyrics.source.unwrap_or_else(|| "YouTube Music".into()),
        }}))
    }

    /// `/home/`: the logged-out YouTube Music home feed as explore sections.
    async fn home(&self) -> ApiResult<Value> {
        let sections: Vec<Value> = self
            .ytm
            .home(6)
            .await?
            .iter()
            .filter_map(shape::section)
            .collect();
        Ok(json!({ "version": VERSION, "sections": sections }))
    }

    /// `/explore/`: the "Moods & moments" and "Genres" grids. Each button id
    /// is the `params` for `/explore/genre/`.
    async fn explore(&self) -> ApiResult<Value> {
        let categories: Vec<Value> = self
            .ytm
            .moods_and_genres()
            .await?
            .iter()
            .map(|c| {
                let genres: Vec<Value> = c
                    .moods
                    .iter()
                    .map(|m| {
                        json!({
                            "id": m.params,
                            "name": m.title,
                            "color": m.color.map(|argb| format!("#{:06x}", argb & 0x00ff_ffff)),
                        })
                    })
                    .collect();
                json!({ "title": c.title, "genres": genres })
            })
            .collect();
        Ok(json!({ "version": VERSION, "categories": categories, "sections": [] }))
    }

    /// `/explore/genre/?id=<params>`: the playlist shelves of one mood/genre.
    async fn explore_genre(&self, q: &Query) -> ApiResult<Value> {
        // Mood params are base64-ish (`ggMPOg1uX…`), not a plain id.
        let params = q
            .get("id")
            .filter(|p| !p.is_empty() && p.len() < 512)
            .ok_or_else(|| ApiError::BadRequest("missing id=".into()))?;
        let sections: Vec<Value> = self
            .ytm
            .mood_playlists(params)
            .await?
            .iter()
            .filter_map(shape::section)
            .collect();
        Ok(json!({ "version": VERSION, "sections": sections }))
    }
}

/// Top tracks must name the page artist, or the UI's "belongs to this
/// artist" filter drops them (contract §3.5 step 3).
fn top_track(t: &Track, owner: &ArtistRef, popularity: u32) -> Value {
    let mut t = t.clone();
    if !t.artists.iter().any(|a| a.id == owner.id) {
        t.artists.push(owner.clone());
    }
    shape::track(
        &t,
        TrackCtx {
            popularity: Some(popularity),
            ..TrackCtx::default()
        },
    )
}

/// Drops repeated videoIds, keeping the first occurrence.
fn dedupe_tracks(tracks: &mut Vec<Track>) {
    let mut seen = HashSet::new();
    tracks.retain(|t| seen.insert(t.video_id.clone()));
}

/// Distinct albums of a track list, excluding `exclude`, as summaries.
fn albums_of(tracks: &[Track], exclude: &str) -> Vec<AlbumSummary> {
    let mut seen = HashSet::from([exclude.to_owned()]);
    tracks
        .iter()
        .filter_map(|t| {
            let album = t.album.as_ref()?;
            let id = album.id.clone()?;
            seen.insert(id.clone()).then(|| AlbumSummary {
                browse_id: id,
                playlist_id: None,
                title: album.name.clone(),
                kind: AlbumKind::Album,
                artists: t.artists.clone(),
                year: t.year,
                thumbnails: t.thumbnails.clone(),
                explicit: false,
            })
        })
        .collect()
}

/// The five search buckets, filled from any mix of items.
#[derive(Default)]
struct SearchBuckets {
    tracks: Vec<Value>,
    artists: Vec<Value>,
    albums: Vec<Value>,
    playlists: Vec<Value>,
}

impl SearchBuckets {
    fn add(&mut self, items: Vec<Item>) {
        for item in items {
            match item {
                Item::Track(t) => self.tracks.push(shape::track(&t, TrackCtx::default())),
                Item::Artist(a) => self.artists.push(shape::artist_summary(&a)),
                Item::Album(a) => self.albums.push(shape::album_summary(&a, None)),
                Item::Playlist(p) => self.playlists.push(shape::playlist_summary(&p)),
            }
        }
    }

    fn into_json(self) -> Value {
        json!({
            "tracks": shape::bucket(self.tracks),
            "artists": shape::bucket(self.artists),
            "albums": shape::bucket(self.albums),
            "playlists": shape::bucket(self.playlists),
            "videos": shape::bucket(Vec::new()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use volna_ytm::{AlbumRef, TrackKind};

    fn t(id: &str, album: Option<&str>) -> Track {
        Track {
            video_id: id.into(),
            title: id.into(),
            artists: vec![ArtistRef {
                id: Some("UCa".into()),
                name: "A".into(),
            }],
            album: album.map(|a| AlbumRef {
                id: Some(a.into()),
                name: a.into(),
            }),
            duration_s: Some(1),
            thumbnails: vec![],
            explicit: false,
            kind: TrackKind::Song,
            year: None,
            views: None,
            playable: true,
        }
    }

    #[test]
    fn dedupe_keeps_first() {
        let mut v = vec![t("a", None), t("b", None), t("a", None)];
        dedupe_tracks(&mut v);
        let ids: Vec<_> = v.iter().map(|t| t.video_id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn albums_of_skips_source_and_duplicates() {
        let tracks = [
            t("1", Some("MPREb_self")),
            t("2", Some("MPREb_x")),
            t("3", None),
            t("4", Some("MPREb_x")),
            t("5", Some("MPREb_y")),
        ];
        let ids: Vec<_> = albums_of(&tracks, "MPREb_self")
            .into_iter()
            .map(|a| a.browse_id)
            .collect();
        assert_eq!(ids, ["MPREb_x", "MPREb_y"]);
    }

    #[test]
    fn top_tracks_name_the_page_artist() {
        let owner = ArtistRef {
            id: Some("UCowner".into()),
            name: "Owner".into(),
        };
        let v = top_track(&t("x", None), &owner, 99);
        assert_eq!(v["popularity"], 99);
        let ids: Vec<_> = v["artists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(ids, ["UCa", "UCowner"]);
    }

    #[test]
    fn search_buckets_are_all_present() {
        let mut b = SearchBuckets::default();
        b.add(vec![Item::Track(t("x", None))]);
        let v = b.into_json();
        for key in ["tracks", "artists", "albums", "playlists", "videos"] {
            assert!(v[key]["items"].is_array(), "{key}");
        }
        assert_eq!(v["tracks"]["totalNumberOfItems"], 1);
    }
}
