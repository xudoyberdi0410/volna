//! Public endpoints of [`YtmClient`].

use serde_json::json;

use crate::client::YtmClient;
use crate::error::{Error, Result};
use crate::models::{
    Album, Artist, Lyrics, MoodCategory, Playlist, Related, SearchFilter, SearchPage,
    SearchSuggestions, Section, TrackDetails, TrackPage, WatchQueue,
};
use crate::parse::{browse, items::strip_vl, pages, search, watch};

impl YtmClient {
    /// Searches the YouTube Music catalogue.
    ///
    /// [`SearchFilter::All`] returns the top result plus a handful of each
    /// type; filtered searches return ~20 items and a continuation.
    pub async fn search(&self, query: &str, filter: SearchFilter) -> Result<SearchPage> {
        let mut body = json!({ "query": query });
        if let Some(p) = search::filter_params(filter) {
            body["params"] = json!(p);
        }
        let resp = self.post("search", body).await?;
        Ok(search::parse_search(&resp, filter))
    }

    /// Next page of a filtered search.
    pub async fn search_more(
        &self,
        continuation: &str,
        filter: SearchFilter,
    ) -> Result<SearchPage> {
        let resp = self
            .post("search", json!({ "continuation": continuation }))
            .await?;
        Ok(search::parse_search_continuation(&resp, filter))
    }

    /// Autocomplete for a partial query.
    pub async fn search_suggestions(&self, input: &str) -> Result<SearchSuggestions> {
        let resp = self
            .post("music/get_search_suggestions", json!({ "input": input }))
            .await?;
        Ok(search::parse_suggestions(&resp))
    }

    /// Metadata of one track, taken from the watch-next panel (the `player`
    /// endpoint asks logged-out web clients to prove they are not a bot).
    pub async fn track(&self, video_id: &str) -> Result<TrackDetails> {
        let resp = self.next(video_id, None).await?;
        let queue = watch::parse_watch(&resp);
        let track = queue
            .tracks
            .into_iter()
            .find(|t| t.video_id == video_id)
            .ok_or_else(|| Error::NotFound(format!("track {video_id}")))?;
        Ok(TrackDetails {
            track,
            lyrics_browse_id: queue.lyrics_browse_id,
            related_browse_id: queue.related_browse_id,
        })
    }

    /// Endless song radio (`RDAMVM<videoId>`), starting with the seed.
    pub async fn radio(&self, video_id: &str) -> Result<WatchQueue> {
        let playlist = format!("RDAMVM{video_id}");
        let resp = self.next(video_id, Some(&playlist)).await?;
        Ok(watch::parse_watch(&resp))
    }

    /// Watch queue of any playlist id: an album (`OLAK5uy_…`), a mix
    /// (`RDCLAK…`), an artist radio (`RDEM…`), or a user playlist.
    pub async fn watch_playlist(&self, playlist_id: &str) -> Result<WatchQueue> {
        let body = json!({
            "playlistId": strip_vl(playlist_id),
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        });
        let resp = self.post("next", body).await?;
        Ok(watch::parse_watch(&resp))
    }

    /// More tracks for a queue returned by [`Self::radio`] / [`Self::watch_playlist`].
    pub async fn queue_more(&self, continuation: &str) -> Result<TrackPage> {
        let body = json!({
            "continuation": continuation,
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
        });
        let resp = self.post("next", body).await?;
        Ok(watch::parse_watch_continuation(&resp))
    }

    async fn next(&self, video_id: &str, playlist_id: Option<&str>) -> Result<serde_json::Value> {
        let mut body = json!({
            "videoId": video_id,
            "isAudioOnly": true,
            "enablePersistentPlaylistPanel": true,
            "tunerSettingValue": "AUTOMIX_SETTING_NORMAL",
        });
        if let Some(p) = playlist_id {
            body["playlistId"] = json!(p);
        }
        self.post("next", body).await
    }

    async fn browse(&self, browse_id: &str, params: Option<&str>) -> Result<serde_json::Value> {
        let mut body = json!({ "browseId": browse_id });
        if let Some(p) = params {
            body["params"] = json!(p);
        }
        self.post("browse", body).await
    }

    async fn browse_continuation(&self, token: &str) -> Result<serde_json::Value> {
        self.post("browse", json!({ "continuation": token })).await
    }

    /// Album page. Accepts an album browse id (`MPREb_…`) or the album's
    /// audio playlist id (`OLAK5uy_…`), which is resolved first.
    pub async fn album(&self, id: &str) -> Result<Album> {
        let browse_id = if id.starts_with("MPRE") {
            id.to_owned()
        } else {
            self.album_browse_id(id).await?
        };
        let resp = self.browse(&browse_id, None).await?;
        pages::parse_album(&resp, &browse_id)
            .ok_or_else(|| Error::NotFound(format!("album {browse_id}")))
    }

    /// `OLAK5uy_…` → `MPREb_…`.
    pub async fn album_browse_id(&self, playlist_id: &str) -> Result<String> {
        let id = strip_vl(playlist_id);
        let resp = self.browse(&format!("VL{id}"), None).await?;
        pages::album_id_from_playlist_page(&resp)
            .ok_or_else(|| Error::NotFound(format!("album for playlist {id}")))
    }

    /// Artist page: top songs, albums, singles, videos, related artists.
    pub async fn artist(&self, channel_id: &str) -> Result<Artist> {
        let resp = self.browse(channel_id, None).await?;
        pages::parse_artist(&resp, channel_id)
            .ok_or_else(|| Error::NotFound(format!("artist {channel_id}")))
    }

    /// First page of a playlist (accepts ids with or without `VL`).
    /// Page further with [`Self::playlist_more`] or use [`Self::playlist_all`].
    pub async fn playlist(&self, playlist_id: &str) -> Result<Playlist> {
        let id = strip_vl(playlist_id);
        let resp = self.browse(&format!("VL{id}"), None).await?;
        pages::parse_playlist(&resp, id).ok_or_else(|| Error::NotFound(format!("playlist {id}")))
    }

    pub async fn playlist_more(&self, continuation: &str) -> Result<TrackPage> {
        let resp = self.browse_continuation(continuation).await?;
        Ok(pages::parse_playlist_continuation(&resp))
    }

    /// Playlist with up to `limit` tracks (all of them when `None`).
    pub async fn playlist_all(&self, playlist_id: &str, limit: Option<usize>) -> Result<Playlist> {
        let mut pl = self.playlist(playlist_id).await?;
        let limit = limit.unwrap_or(usize::MAX);
        while pl.tracks.len() < limit
            && let Some(token) = pl.continuation.take()
        {
            let page = self.playlist_more(&token).await?;
            if page.tracks.is_empty() {
                break;
            }
            pl.tracks.extend(page.tracks);
            pl.continuation = page.continuation;
        }
        pl.tracks.truncate(limit);
        Ok(pl)
    }

    /// The "Related" tab of a song: similar tracks, artists, playlists.
    pub async fn related(&self, video_id: &str) -> Result<Related> {
        let resp = self.next(video_id, None).await?;
        let id = watch::related_browse_id(&resp)
            .ok_or_else(|| Error::NotFound(format!("related tab of {video_id}")))?;
        self.related_by_browse_id(&id).await
    }

    /// Related tab by its `MPTR…` id (from [`TrackDetails`] / [`WatchQueue`]).
    pub async fn related_by_browse_id(&self, browse_id: &str) -> Result<Related> {
        let resp = self.browse(browse_id, None).await?;
        Ok(browse::parse_related(&resp))
    }

    /// Plain-text lyrics, `None` when YouTube has none for the song.
    pub async fn lyrics(&self, video_id: &str) -> Result<Option<Lyrics>> {
        let resp = self.next(video_id, None).await?;
        match watch::lyrics_browse_id(&resp) {
            Some(id) => self.lyrics_by_browse_id(&id).await,
            None => Ok(None),
        }
    }

    /// Lyrics by their `MPLY…` id.
    pub async fn lyrics_by_browse_id(&self, browse_id: &str) -> Result<Option<Lyrics>> {
        let resp = self.browse(browse_id, None).await?;
        Ok(browse::parse_lyrics(&resp))
    }

    /// "Moods & moments" and "Genres" grids.
    pub async fn moods_and_genres(&self) -> Result<Vec<MoodCategory>> {
        let resp = self.browse("FEmusic_moods_and_genres", None).await?;
        Ok(browse::parse_moods(&resp))
    }

    /// Playlist shelves of one mood/genre (`params` from [`crate::Mood`]).
    pub async fn mood_playlists(&self, params: &str) -> Result<Vec<Section>> {
        let resp = self
            .browse("FEmusic_moods_and_genres_category", Some(params))
            .await?;
        Ok(browse::parse_feed(&resp).0)
    }

    /// Logged-out home feed, following continuations until at least
    /// `min_sections` sections are loaded (or the feed ends).
    pub async fn home(&self, min_sections: usize) -> Result<Vec<Section>> {
        let resp = self.browse("FEmusic_home", None).await?;
        let (mut sections, mut cont) = browse::parse_feed(&resp);
        while sections.len() < min_sections
            && let Some(token) = cont.take()
        {
            let resp = self.browse_continuation(&token).await?;
            let (more, next) = browse::parse_feed(&resp);
            if more.is_empty() {
                break;
            }
            sections.extend(more);
            cont = next;
        }
        Ok(sections)
    }
}
