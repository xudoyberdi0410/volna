//! «Моя волна»: Tauri commands over [`volna_wave::Wave`].
//!
//! The database lives in `<app data dir>/wave.sqlite3`. Tracks go to the
//! page in the same hifi-api shape as `volna://api` ([`shape::track`]), so
//! the UI runs them through its usual `prepareTrack`. Player events come
//! back as the UI's track objects and are mapped to [`volna_ytm::Track`].

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::OnceCell;
use volna_wave::{
    ArtistBrief, PlayerEvent, RecordOutcome, SeedArtist, Store, Wave, WaveSettings, WaveStatus,
    YtmSource,
};
use volna_ytm::{ArtistRef, Item, SearchFilter, Track, TrackKind, YtmClient};

use crate::shape::{self, TrackCtx};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Shared by all commands; cheap to clone.
#[derive(Clone)]
pub struct WaveState {
    wave: Arc<Wave<YtmSource>>,
    popular: Arc<OnceCell<Vec<ArtistCard>>>,
}

impl WaveState {
    pub fn open(data_dir: &Path, ytm: YtmClient) -> anyhow::Result<Self> {
        let store = Store::open(&data_dir.join("wave.sqlite3"))?;
        Ok(Self {
            wave: Arc::new(Wave::new(store, YtmSource::new(ytm))),
            popular: Arc::new(OnceCell::new()),
        })
    }

    /// The engine, for the native player's continuation (`player/hooks.rs`).
    pub fn engine(&self) -> Arc<Wave<YtmSource>> {
        self.wave.clone()
    }

    pub fn ytm(&self) -> &YtmClient {
        self.wave.source().client()
    }
}

/// Artists on the first cold-start screen, before the listener types
/// anything: a mix of what is big in Uzbekistan, in Russian and worldwide.
const POPULAR_ARTISTS: &[&str] = &[
    "Shahzoda",
    "Скриптонит",
    "Jony",
    "The Weeknd",
    "Xamdam Sobirov",
    "Miyagi & Andy Panda",
    "Billie Eilish",
    "Rauf & Faik",
    "Ummon",
    "Macan",
    "Dua Lipa",
    "Баста",
    "Imagine Dragons",
    "Kino",
    "Ulug'bek Rahmatullayev",
    "Eminem",
];

/// An artist tile as the UI draws it.
#[derive(Debug, Clone, Serialize)]
pub struct ArtistCard {
    id: String,
    name: String,
    /// `volna://img` URL or null.
    picture: Option<String>,
}

impl From<ArtistBrief> for ArtistCard {
    fn from(a: ArtistBrief) -> Self {
        Self {
            picture: a.thumbnail.as_deref().map(|u| shape::image_url(u, 226)),
            id: a.id,
            name: a.name,
        }
    }
}

fn artist_cards(items: impl IntoIterator<Item = Item>) -> Vec<ArtistCard> {
    items
        .into_iter()
        .filter_map(|i| match i {
            Item::Artist(a) => Some(ArtistCard {
                picture: shape::best_thumbnail(&a.thumbnails).map(|u| shape::image_url(u, 226)),
                id: a.id,
                name: a.name,
            }),
            _ => None,
        })
        .collect()
}

fn shaped(tracks: &[Track]) -> Vec<Value> {
    tracks
        .iter()
        .map(|t| shape::track(t, TrackCtx::default()))
        .collect()
}

/// A track object as the UI holds it (`PreparedTrack`); only what the wave
/// needs is read, the rest is ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UiTrack {
    id: String,
    title: String,
    artists: Vec<UiArtist>,
    artist: Option<UiArtist>,
    duration: Option<f64>,
    explicit: bool,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UiArtist {
    id: Option<Value>,
    name: String,
}

impl From<UiTrack> for Track {
    fn from(t: UiTrack) -> Self {
        let mut artists: Vec<UiArtist> = t.artists;
        if artists.is_empty()
            && let Some(a) = t.artist
        {
            artists.push(a);
        }
        Track {
            video_id: t.id,
            title: t.title,
            artists: artists
                .into_iter()
                .filter(|a| !a.name.is_empty() && a.name != "Unknown artist")
                .map(|a| ArtistRef {
                    // Ids are strings in Volna; tolerate numbers from old data.
                    id: match a.id {
                        Some(Value::String(s)) if !s.is_empty() => Some(s),
                        Some(Value::Number(n)) => Some(n.to_string()),
                        _ => None,
                    },
                    name: a.name,
                })
                .collect(),
            album: None,
            duration_s: t.duration.filter(|d| *d > 0.0).map(|d| d.round() as u32),
            thumbnails: vec![],
            explicit: t.explicit,
            kind: if t.kind.as_deref() == Some("video") {
                TrackKind::Video
            } else {
                TrackKind::Song
            },
            year: None,
            views: None,
            playable: true,
        }
    }
}

/// Events the page sends (`camelCase`, tagged by `kind`).
#[derive(Debug, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum UiEvent {
    Start {
        track: UiTrack,
        #[serde(default)]
        in_wave: bool,
    },
    End {
        track: UiTrack,
        position: f64,
        duration: Option<f64>,
        #[serde(default)]
        ended: bool,
        #[serde(default)]
        in_wave: bool,
    },
    Like {
        track: UiTrack,
        liked: bool,
    },
    Dislike {
        track: UiTrack,
        disliked: bool,
    },
}

impl From<UiEvent> for PlayerEvent {
    fn from(e: UiEvent) -> Self {
        match e {
            UiEvent::Start { track, in_wave } => PlayerEvent::Start {
                track: track.into(),
                in_wave,
            },
            UiEvent::End {
                track,
                position,
                duration,
                ended,
                in_wave,
            } => PlayerEvent::End {
                track: track.into(),
                position_s: position.max(0.0),
                duration_s: duration.filter(|d| d.is_finite() && *d > 0.0),
                ended,
                in_wave,
            },
            UiEvent::Like { track, liked } => PlayerEvent::Like {
                track: track.into(),
                liked,
            },
            UiEvent::Dislike { track, disliked } => PlayerEvent::Dislike {
                track: track.into(),
                disliked,
            },
        }
    }
}

/// A liked track from the UI library with its `addedAt` (ms).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiLike {
    #[serde(flatten)]
    track: UiTrack,
    #[serde(default)]
    added_at: Option<f64>,
}

/// "Занятие" choices and the genre list, from YouTube Music.
#[derive(Debug, Serialize)]
pub struct ActivityOption {
    title: String,
    label: String,
    params: String,
}

#[derive(Debug, Serialize)]
pub struct ActivityList {
    activities: Vec<ActivityOption>,
    genres: Vec<ActivityOption>,
}

/// YouTube Music titles already covered by the mood buttons.
const MOOD_TITLES: &[&str] = &["Energize", "Feel good", "Chill", "Sad"];

fn russian_label(title: &str) -> String {
    match title {
        "Workout" => "Тренировка",
        "Focus" => "Работа и учёба",
        "Commute" => "В дороге",
        "Sleep" => "Сон",
        "Party" => "Вечеринка",
        "Romance" => "Романтика",
        "Gaming" => "Игры",
        "Christmas" => "Новый год",
        "Hip-hop" => "Хип-хоп",
        "Pop" => "Поп",
        "Rock" => "Рок",
        "Jazz" => "Джаз",
        "Classical" => "Классика",
        "Metal" => "Метал",
        "Dance & electronic" => "Электроника",
        "Indie & alternative" => "Инди",
        "R&B & soul" => "R&B и соул",
        "Folk & acoustic" => "Фолк и акустика",
        "Soundtracks & musicals" => "Саундтреки",
        "Latin" => "Латино",
        "Arabic" => "Арабская",
        "Decades" => "Десятилетия",
        "Family" => "Для всей семьи",
        "Blues" => "Блюз",
        "Country & Americana" => "Кантри",
        "Reggae & caribbean" => "Регги",
        "African" => "Африканская",
        "Autumn" => "Осень",
        "Bollywood & Indian" => "Индийская",
        "Mandopop & cantopop" => "Китайская",
        "Korean hip-hop" => "Корейский хип-хоп",
        other => other,
    }
    .to_owned()
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub async fn wave_status(wave: &WaveState) -> CmdResult<WaveStatus> {
    wave.wave.status().await.map_err(err)
}

pub async fn wave_set_settings(wave: &WaveState, settings: WaveSettings) -> CmdResult<WaveStatus> {
    wave.wave.set_settings(settings).await.map_err(err)?;
    wave.wave.status().await.map_err(err)
}

pub async fn wave_activities(wave: &WaveState) -> CmdResult<ActivityList> {
    let categories = wave.ytm().moods_and_genres().await.map_err(err)?;
    let mut list = ActivityList {
        activities: vec![],
        genres: vec![],
    };
    for (i, c) in categories.into_iter().enumerate() {
        let is_moods = c.title.to_lowercase().contains("mood") || i == 0;
        for m in c.moods {
            if is_moods && MOOD_TITLES.contains(&m.title.as_str()) {
                continue;
            }
            let opt = ActivityOption {
                label: russian_label(&m.title),
                title: m.title,
                params: m.params,
            };
            if is_moods {
                list.activities.push(opt);
            } else {
                list.genres.push(opt);
            }
        }
    }
    Ok(list)
}

/// Artist search for the cold-start screen; an empty query gives the
/// default grid.
pub async fn wave_artist_search(wave: &WaveState, query: String) -> CmdResult<Vec<ArtistCard>> {
    let query = query.trim();
    if query.is_empty() {
        let ytm = wave.ytm().clone();
        let cards = wave
            .popular
            .get_or_try_init(|| async move {
                let mut tasks = tokio::task::JoinSet::new();
                for (i, name) in POPULAR_ARTISTS.iter().enumerate() {
                    let ytm = ytm.clone();
                    tasks.spawn(async move { (i, ytm.search(name, SearchFilter::Artists).await) });
                }
                let mut found = Vec::new();
                while let Some(joined) = tasks.join_next().await {
                    if let Ok((i, Ok(page))) = joined
                        && let Some(card) = artist_cards(page.items).into_iter().next()
                    {
                        found.push((i, card));
                    }
                }
                found.sort_by_key(|(i, _)| *i);
                let cards: Vec<ArtistCard> = found.into_iter().map(|(_, c)| c).collect();
                if cards.is_empty() {
                    Err("YouTube Music did not answer".to_owned())
                } else {
                    Ok(cards)
                }
            })
            .await?;
        return Ok(dedup_cards(cards.clone()));
    }
    let page = wave
        .ytm()
        .search(query, SearchFilter::Artists)
        .await
        .map_err(err)?;
    Ok(dedup_cards(artist_cards(page.items))
        .into_iter()
        .take(18)
        .collect())
}

/// One tile per artist: YouTube often lists an artist's channel and its
/// "- Topic" twin under the same name.
fn dedup_cards(mut cards: Vec<ArtistCard>) -> Vec<ArtistCard> {
    let mut seen = std::collections::HashSet::new();
    cards.retain(|c| {
        seen.insert(c.id.clone()) && seen.insert(format!("name:{}", c.name.to_lowercase()))
    });
    cards
}

/// "Fans might also like" of a picked artist, to grow the grid.
pub async fn wave_related_artists(wave: &WaveState, id: String) -> CmdResult<Vec<ArtistCard>> {
    let artist = wave.ytm().artist(&id).await.map_err(err)?;
    Ok(artist_cards(artist.related.into_iter().map(Item::Artist)))
}

pub async fn wave_cold_start(wave: &WaveState, artists: Vec<SeedArtist>) -> CmdResult<WaveStatus> {
    wave.wave.add_seed_artists(&artists).map_err(err)?;
    wave.wave.status().await.map_err(err)
}

pub async fn wave_start(wave: &WaveState, count: Option<usize>) -> CmdResult<Vec<Value>> {
    let tracks = wave.wave.start(count.unwrap_or(5)).await.map_err(err)?;
    Ok(shaped(&tracks))
}

pub async fn wave_next(wave: &WaveState, count: Option<usize>) -> CmdResult<Vec<Value>> {
    let tracks = wave.wave.next(count.unwrap_or(5)).await.map_err(err)?;
    Ok(shaped(&tracks))
}

/// After a re-seed: the queued `dropped` tracks are stale, give fresh ones.
pub async fn wave_requeue(
    wave: &WaveState,
    dropped: Vec<String>,
    count: Option<usize>,
) -> CmdResult<Vec<Value>> {
    let tracks = wave
        .wave
        .requeue(&dropped, count.unwrap_or(3))
        .await
        .map_err(err)?;
    Ok(shaped(&tracks))
}

pub async fn wave_stop(wave: &WaveState) -> CmdResult<()> {
    wave.wave.stop().await;
    Ok(())
}

pub async fn wave_event(wave: &WaveState, event: UiEvent) -> CmdResult<RecordOutcome> {
    let event = PlayerEvent::from(event);
    let id = match &event {
        PlayerEvent::Start { track, .. }
        | PlayerEvent::End { track, .. }
        | PlayerEvent::Like { track, .. }
        | PlayerEvent::Dislike { track, .. } => track.video_id.clone(),
    };
    if id.is_empty() {
        return Err("track without id".into());
    }
    let out = wave.wave.record(event).await.map_err(err)?;
    tracing::debug!(%id, ?out, "wave event");
    Ok(out)
}

/// Mirrors the UI's liked tracks (IndexedDB favourites) into the wave.
pub async fn wave_sync_likes(wave: &WaveState, tracks: Vec<UiLike>) -> CmdResult<usize> {
    let now = now_s();
    let liked: Vec<(Track, i64)> = tracks
        .into_iter()
        .filter(|l| !l.track.id.is_empty())
        .map(|l| {
            let ts = l.added_at.map_or(now, |ms| (ms / 1000.0) as i64);
            (Track::from(l.track), ts)
        })
        .collect();
    wave.wave.store().replace_likes(&liked, now).map_err(err)?;
    Ok(liked.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_events_map_to_player_events() {
        let raw = serde_json::json!({
            "kind": "end",
            "track": {
                "id": "abc",
                "title": "Привычка",
                "artists": [{"id": "UC1", "name": "Скриптонит", "type": "MAIN"}],
                "duration": 201,
                "album": {"id": null, "title": "", "cover": "volna://img/?u=x"},
                "type": "track"
            },
            "position": 12.5,
            "duration": 201.3,
            "inWave": true
        });
        let ev: UiEvent = serde_json::from_value(raw).unwrap();
        match PlayerEvent::from(ev) {
            PlayerEvent::End {
                track,
                position_s,
                duration_s,
                ended,
                in_wave,
            } => {
                assert_eq!(track.video_id, "abc");
                assert_eq!(track.artists[0].id.as_deref(), Some("UC1"));
                assert_eq!(track.duration_s, Some(201));
                assert_eq!(position_s, 12.5);
                assert_eq!(duration_s, Some(201.3));
                assert!(!ended && in_wave);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn likes_tolerate_sparse_tracks() {
        let likes: Vec<UiLike> = serde_json::from_value(serde_json::json!([
            {"id": "a", "title": "x", "artist": {"id": null, "name": "Unknown artist"}, "addedAt": 1.7e12},
            {"id": "b", "artist": {"id": "UC2", "name": "B"}}
        ]))
        .unwrap();
        let a = Track::from(likes[0].track.clone());
        assert!(a.artists.is_empty());
        let b = Track::from(likes[1].track.clone());
        assert_eq!(b.artists[0].name, "B");
        assert_eq!(likes[0].added_at, Some(1.7e12));
    }
}
