//! Where candidates come from. The engine only sees [`CandidateSource`];
//! [`YtmSource`] is the real one, tests plug in a scripted catalogue.

use std::collections::HashMap;
use std::future::Future;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use volna_ytm::{Item, Track, YtmClient};

use crate::error::{Error, Result};

/// An artist with what the wave needs from their page.
#[derive(Debug, Clone, Default)]
pub struct ArtistInfo {
    pub top_tracks: Vec<Track>,
    /// "Fans might also like".
    pub related: Vec<ArtistBrief>,
}

/// Minimal artist card (cold-start grid, related artists).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistBrief {
    pub id: String,
    pub name: String,
    /// Picture URL as YouTube serves it (the app proxies it).
    #[serde(default)]
    pub thumbnail: Option<String>,
}

/// Which mood playlists to draw from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MoodQuery {
    /// A "Moods & moments" button by its YouTube Music title ("Chill").
    Title(String),
    /// Raw `params` of any mood/genre button (activities).
    Params(String),
}

/// Candidate generators. All lists may contain tracks the wave will drop
/// (repeats, bans); sources need not filter.
pub trait CandidateSource: Send + Sync {
    /// YouTube Music radio (`RDAMVM<id>`) of a seed track.
    fn radio(&self, video_id: &str) -> impl Future<Output = Result<Vec<Track>>> + Send;
    /// The "Related" tab of a track.
    fn related(&self, video_id: &str) -> impl Future<Output = Result<Vec<Track>>> + Send;
    /// Top tracks and related artists of an artist.
    fn artist(&self, artist_id: &str) -> impl Future<Output = Result<ArtistInfo>> + Send;
    /// Tracks from playlists of a mood or activity.
    fn mood_tracks(&self, mood: &MoodQuery) -> impl Future<Output = Result<Vec<Track>>> + Send;
}

/// [`CandidateSource`] over the app's YouTube Music client.
pub struct YtmSource {
    ytm: YtmClient,
    /// Mood title → params, loaded once.
    moods: Mutex<Option<HashMap<String, String>>>,
    rng: std::sync::Mutex<fastrand::Rng>,
}

impl YtmSource {
    pub fn new(ytm: YtmClient) -> Self {
        Self {
            ytm,
            moods: Mutex::new(None),
            rng: std::sync::Mutex::new(fastrand::Rng::new()),
        }
    }

    pub fn client(&self) -> &YtmClient {
        &self.ytm
    }

    async fn mood_params(&self, title: &str) -> Result<String> {
        let mut moods = self.moods.lock().await;
        if moods.is_none() {
            let map = self
                .ytm
                .moods_and_genres()
                .await?
                .into_iter()
                .flat_map(|c| c.moods)
                .map(|m| (m.title.to_lowercase(), m.params))
                .collect();
            *moods = Some(map);
        }
        moods
            .as_ref()
            .and_then(|m| m.get(&title.to_lowercase()).cloned())
            .ok_or_else(|| Error::Source(volna_ytm::Error::NotFound(format!("mood {title}"))))
    }
}

impl CandidateSource for YtmSource {
    async fn radio(&self, video_id: &str) -> Result<Vec<Track>> {
        Ok(self.ytm.radio(video_id).await?.tracks)
    }

    async fn related(&self, video_id: &str) -> Result<Vec<Track>> {
        Ok(self.ytm.related(video_id).await?.tracks)
    }

    async fn artist(&self, artist_id: &str) -> Result<ArtistInfo> {
        let a = self.ytm.artist(artist_id).await?;
        Ok(ArtistInfo {
            top_tracks: a.top_songs,
            related: a
                .related
                .into_iter()
                .map(|r| ArtistBrief {
                    thumbnail: r.thumbnails.last().map(|t| t.url.clone()),
                    id: r.id,
                    name: r.name,
                })
                .collect(),
        })
    }

    async fn mood_tracks(&self, mood: &MoodQuery) -> Result<Vec<Track>> {
        let params = match mood {
            MoodQuery::Title(t) => self.mood_params(t).await?,
            MoodQuery::Params(p) => p.clone(),
        };
        let mut playlists: Vec<String> = self
            .ytm
            .mood_playlists(&params)
            .await?
            .into_iter()
            .flat_map(|s| s.items)
            .filter_map(|i| match i {
                Item::Playlist(p) => Some(p.id),
                _ => None,
            })
            .collect();
        {
            let mut rng = self.rng.lock().unwrap_or_else(|e| e.into_inner());
            rng.shuffle(&mut playlists);
        }
        // Two playlists give ~100–200 tracks: plenty, and only two requests.
        let pages = futures_util::future::join_all(
            playlists.iter().take(2).map(|id| self.ytm.playlist(id)),
        )
        .await;
        let mut tracks = Vec::new();
        for page in pages {
            match page {
                Ok(p) => tracks.extend(p.tracks),
                Err(e) => tracing::debug!("mood playlist failed: {e}"),
            }
        }
        Ok(tracks)
    }
}
