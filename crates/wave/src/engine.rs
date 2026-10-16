//! The wave session: seeds, the candidate pool, picking and live re-seeding.
//!
//! One [`Wave`] lives for the whole app. [`Wave::start`] opens a session
//! (fresh seeds, empty pool), [`Wave::next`] hands out tracks and refills
//! the pool when it runs low, [`Wave::record`] takes player events, writes
//! them to the [`Store`] and reacts at once: skips push the artist down for
//! the rest of the session, two skips in a row from one seed replace that
//! seed, a like turns the track into a new seed.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use volna_ytm::Track;

use crate::error::{Error, Result};
use crate::lang;
use crate::profile::TasteProfile;
use crate::scoring::{self, Candidate, Origin, ScoreParams, Seed, SessionView};
use crate::settings::{Character, Language, WaveSettings};
use crate::signal::{DAY, PlayOutcome, Weights};
use crate::source::{CandidateSource, MoodQuery};
use crate::store::{SeedArtist, Store, artist_key};

/// Unix seconds; injectable so tests can fast-forward.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    })
}

/// How much history the profile reads.
const HISTORY_DAYS: i64 = 365;
/// Refill when fewer eligible candidates than this are left.
const LOW_POOL: usize = 12;
/// Seeds per session (PLAN: 2–4).
const SEEDS: usize = 3;
const MAX_SEEDS: usize = 4;
/// Skips in a row from one seed that retire it.
const SEED_SKIP_LIMIT: u32 = 2;
/// Artists remembered for spacing.
const RECENT: usize = 10;

/// What the player tells the wave.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlayerEvent {
    /// A track started playing.
    Start {
        track: Track,
        #[serde(default)]
        in_wave: bool,
    },
    /// The player left a track: it ended, was skipped or replaced.
    End {
        track: Track,
        position_s: f64,
        duration_s: Option<f64>,
        #[serde(default)]
        ended: bool,
        #[serde(default)]
        in_wave: bool,
    },
    Like {
        track: Track,
        liked: bool,
    },
    Dislike {
        track: Track,
        disliked: bool,
    },
}

/// What [`Wave::record`] did.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct RecordOutcome {
    /// How the play was classified (for `End`).
    pub outcome: Option<PlayOutcome>,
    /// A seed was replaced or added: tracks already queued from the old
    /// seed are stale, the UI may call [`Wave::requeue`].
    pub reseeded: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaveStatus {
    /// No taste yet: show the cold-start screen before starting.
    pub cold: bool,
    pub active: bool,
    pub settings: WaveSettings,
}

/// One unit of candidate fetching.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Job {
    Radio(Seed, String),
    Related(Seed, String),
    SeedArtist(Seed, String),
    Affine { artist: String, explore: bool },
    Mood(MoodQuery),
}

struct Fetched {
    candidates: Vec<Candidate>,
    /// Extra jobs found while fetching (radio of an artist seed's top track).
    follow_up: Vec<Job>,
}

#[derive(Default)]
struct Session {
    active: bool,
    settings: WaveSettings,
    pool: Vec<Candidate>,
    served: HashSet<String>,
    origin_of: HashMap<String, Origin>,
    /// Handed out but not started yet, in order.
    queued: VecDeque<(String, Vec<String>)>,
    /// Artist keys of the latest started tracks.
    played: VecDeque<Vec<String>>,
    seeds: Vec<Seed>,
    retired: HashSet<Seed>,
    seed_skips: HashMap<Seed, u32>,
    /// Session-only artist adjustments (live reaction to skips/likes).
    session_artist: HashMap<String, f64>,
    mood_artists: HashSet<String>,
    done_jobs: HashSet<Job>,
    /// New seeds or settings: fetch before the next pick even if the pool
    /// is still full, so the change is heard right away.
    force_refill: bool,
    /// Tracks completed in this session: good fresh seeds.
    completed: Vec<String>,
}

impl Session {
    fn recent_artists(&self) -> Vec<Vec<String>> {
        self.played
            .iter()
            .cloned()
            .chain(self.queued.iter().map(|(_, a)| a.clone()))
            .collect()
    }

    fn retire(&mut self, seed: &Seed) {
        self.retired.insert(seed.clone());
        self.seeds.retain(|s| s != seed);
        self.pool.retain(|c| c.origin.seed() != Some(seed));
    }
}

pub struct Wave<S> {
    store: Store,
    source: S,
    weights: Weights,
    params: ScoreParams,
    clock: Clock,
    session: Mutex<Session>,
    rng: std::sync::Mutex<fastrand::Rng>,
}

impl<S: CandidateSource> Wave<S> {
    pub fn new(store: Store, source: S) -> Self {
        Self {
            store,
            source,
            weights: Weights::default(),
            params: ScoreParams::default(),
            clock: system_clock(),
            session: Mutex::new(Session::default()),
            rng: std::sync::Mutex::new(fastrand::Rng::new()),
        }
    }

    /// Deterministic randomness and time, for tests and simulations.
    pub fn with_test_hooks(mut self, seed: u64, clock: Clock) -> Self {
        self.rng = std::sync::Mutex::new(fastrand::Rng::with_seed(seed));
        self.clock = clock;
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn source(&self) -> &S {
        &self.source
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    fn rng(&self) -> std::sync::MutexGuard<'_, fastrand::Rng> {
        self.rng.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn profile(&self) -> Result<TasteProfile> {
        let now = self.now();
        let snap = self.store.snapshot(now - HISTORY_DAYS * DAY)?;
        Ok(TasteProfile::build(&snap, &self.weights, now))
    }

    pub async fn status(&self) -> Result<WaveStatus> {
        let active = self.session.lock().await.active;
        Ok(WaveStatus {
            cold: self.profile()?.is_cold(),
            active,
            settings: self.store.settings()?,
        })
    }

    /// Saves settings; a running session switches to them (mood candidates
    /// of the old mood are dropped, history of the session is kept).
    pub async fn set_settings(&self, settings: WaveSettings) -> Result<()> {
        self.store.set_settings(&settings)?;
        let mut s = self.session.lock().await;
        if s.active && s.settings != settings {
            s.pool.retain(|c| c.origin != Origin::Mood);
            s.mood_artists.clear();
            s.done_jobs
                .retain(|j| !matches!(j, Job::Mood(_) | Job::Affine { .. }));
            s.settings = settings;
            s.force_refill = true;
        }
        Ok(())
    }

    /// Cold start: the artists picked on the first screen become seeds.
    pub fn add_seed_artists(&self, artists: &[SeedArtist]) -> Result<()> {
        self.store.add_seed_artists(artists, self.now())
    }

    /// Opens a new session and returns its first `n` tracks.
    pub async fn start(&self, n: usize) -> Result<Vec<Track>> {
        self.open_session().await?;
        self.fill(n).await
    }

    async fn open_session(&self) -> Result<()> {
        let settings = self.store.settings()?;
        let profile = self.profile()?;
        {
            let mut s = self.session.lock().await;
            *s = Session {
                active: true,
                settings: settings.clone(),
                ..Default::default()
            };
            s.seeds = self.pick_seeds(&profile, &s, SEEDS);
            if s.seeds.is_empty() {
                s.active = false;
                return Err(Error::NoCandidates);
            }
            tracing::info!(seeds = ?s.seeds, "wave started");
        }
        Ok(())
    }

    pub async fn stop(&self) {
        self.session.lock().await.active = false;
    }

    /// Next `n` tracks of the running session (starting one if needed).
    pub async fn next(&self, n: usize) -> Result<Vec<Track>> {
        if !self.session.lock().await.active {
            self.open_session().await?;
        }
        self.fill(n).await
    }

    async fn fill(&self, n: usize) -> Result<Vec<Track>> {
        let mut out = Vec::with_capacity(n);
        let mut refills = 0;
        let forced = std::mem::take(&mut self.session.lock().await.force_refill);
        if forced {
            let profile = self.profile()?;
            self.refill(&profile).await?;
            refills += 1;
        }
        while out.len() < n {
            let profile = self.profile()?;
            let picked = {
                let mut s = self.session.lock().await;
                let eligible = self.count_eligible(&s, &profile);
                if eligible < LOW_POOL && refills < 3 {
                    None
                } else {
                    self.pick_one(&mut s, &profile)
                }
            };
            match picked {
                Some(t) => out.push(t),
                None if refills < 3 => {
                    refills += 1;
                    if !self.refill(&profile).await? {
                        // Nothing new to fetch: serve what is left.
                        refills = 3;
                    }
                }
                None => break,
            }
        }
        if out.is_empty() {
            return Err(Error::NoCandidates);
        }
        Ok(out)
    }

    /// Takes back tracks that were queued but not played (after a re-seed or
    /// a settings change) and returns `n` fresh ones.
    pub async fn requeue(&self, dropped: &[String], n: usize) -> Result<Vec<Track>> {
        {
            let mut s = self.session.lock().await;
            for id in dropped {
                s.served.remove(id);
                s.origin_of.remove(id);
                s.queued.retain(|(q, _)| q != id);
            }
        }
        self.next(n).await
    }

    fn view<'a>(&self, s: &'a Session, recent: &'a [Vec<String>]) -> SessionView<'a> {
        SessionView {
            served: Some(&s.served),
            recent_artists: recent,
            session_artist: Some(&s.session_artist),
            mood_artists: Some(&s.mood_artists),
        }
    }

    fn count_eligible(&self, s: &Session, p: &TasteProfile) -> usize {
        let recent = s.recent_artists();
        let view = self.view(s, &recent);
        let now = self.now();
        s.pool
            .iter()
            .filter(|c| scoring::eligible(c, p, &view, &s.settings, &self.params, now))
            .count()
    }

    fn pick_one(&self, s: &mut Session, p: &TasteProfile) -> Option<Track> {
        let recent = s.recent_artists();
        let idx = {
            let view = self.view(s, &recent);
            scoring::pick(
                &s.pool,
                p,
                &view,
                &s.settings,
                &self.params,
                self.now(),
                &mut self.rng(),
            )?
        };
        let c = s.pool.swap_remove(idx);
        let id = c.track.video_id.clone();
        s.served.insert(id.clone());
        s.origin_of.insert(id.clone(), c.origin);
        s.queued
            .push_back((id, c.track.artists.iter().map(artist_key).collect()));
        Some(c.track)
    }

    /// Chooses up to `k` seeds: fresh likes and completed plays, this
    /// session's completions and cold-start artists, sampled by weight.
    fn pick_seeds(&self, p: &TasteProfile, s: &Session, k: usize) -> Vec<Seed> {
        let mut pool: Vec<(Seed, f64)> = Vec::new();
        for id in s.completed.iter().rev().take(5) {
            pool.push((Seed::Track(id.clone()), 4.0));
        }
        for (id, w) in p.seed_tracks.iter().take(30) {
            let mut w = *w;
            if s.settings.character == Character::Favorite && p.liked.contains(id) {
                w *= 2.0;
            }
            if s.settings.language != Language::Any
                && let Ok(Some(t)) = self.store.track(id)
            {
                let names: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
                if lang::detect(&t.title, &names) != s.settings.language {
                    w *= 0.3;
                }
            }
            pool.push((Seed::Track(id.clone()), w));
        }
        for a in &p.seed_artists {
            // Artists the listener has since soured on stop seeding.
            if p.artist.get(&a.id).copied().unwrap_or(0.0) > 0.0 {
                pool.push((Seed::Artist(a.id.clone()), 1.5));
            }
        }
        pool.retain(|(seed, w)| *w > 0.0 && !s.retired.contains(seed) && !s.seeds.contains(seed));
        pool.dedup_by(|a, b| a.0 == b.0);

        let mut rng = self.rng();
        let mut out = Vec::new();
        while out.len() < k && !pool.is_empty() {
            let total: f64 = pool.iter().map(|(_, w)| w).sum();
            let mut roll = rng.f64() * total;
            let mut idx = pool.len() - 1;
            for (i, (_, w)) in pool.iter().enumerate() {
                if roll < *w {
                    idx = i;
                    break;
                }
                roll -= w;
            }
            let (seed, _) = pool.swap_remove(idx);
            pool.retain(|(s, _)| *s != seed);
            out.push(seed);
        }
        out
    }

    /// Plans and runs one round of fetches. `false` when there was nothing
    /// left to fetch.
    async fn refill(&self, profile: &TasteProfile) -> Result<bool> {
        let jobs = {
            let mut s = self.session.lock().await;
            if s.seeds.len() < SEEDS {
                let want = SEEDS - s.seeds.len();
                let more = self.pick_seeds(profile, &s, want);
                s.seeds.extend(more);
            }
            let jobs = self.plan(&s, profile);
            s.done_jobs.extend(jobs.iter().cloned());
            jobs
        };
        if jobs.is_empty() {
            return Ok(false);
        }
        tracing::debug!(?jobs, "wave refill");

        let mut results = futures_util::future::join_all(jobs.iter().map(|j| self.run(j))).await;
        // Follow-ups (radio of an artist seed's top track) in a second round.
        let follow: Vec<Job> = {
            let s = self.session.lock().await;
            results
                .iter()
                .flat_map(|r| r.follow_up.iter().cloned())
                .filter(|j| !s.done_jobs.contains(j))
                .collect()
        };
        if !follow.is_empty() {
            results
                .extend(futures_util::future::join_all(follow.iter().map(|j| self.run(j))).await);
        }

        let mut s = self.session.lock().await;
        s.done_jobs.extend(follow);
        let mut known: HashSet<String> = s.pool.iter().map(|c| c.track.video_id.clone()).collect();
        let now = self.now();
        let mut added = 0;
        for c in results.into_iter().flat_map(|r| r.candidates) {
            if c.origin == Origin::Mood {
                s.mood_artists
                    .extend(c.track.artists.iter().map(artist_key));
            }
            if s.served.contains(&c.track.video_id) || !known.insert(c.track.video_id.clone()) {
                continue;
            }
            // Retired seeds' late results are not wanted any more.
            if c.origin.seed().is_some_and(|seed| s.retired.contains(seed)) {
                continue;
            }
            if let Err(e) = self.store.upsert_track(&c.track, now) {
                tracing::warn!("wave: cannot store track: {e}");
            }
            s.pool.push(c);
            added += 1;
        }
        tracing::debug!(added, pool = s.pool.len(), "wave refill done");
        Ok(true)
    }

    fn plan(&self, s: &Session, profile: &TasteProfile) -> Vec<Job> {
        let mut jobs = Vec::new();
        let push = |jobs: &mut Vec<Job>, j: Job| {
            if !s.done_jobs.contains(&j) && !jobs.contains(&j) {
                jobs.push(j);
            }
        };
        for (i, seed) in s.seeds.iter().enumerate() {
            match seed {
                Seed::Track(id) => {
                    push(&mut jobs, Job::Radio(seed.clone(), id.clone()));
                    if i == 0 {
                        push(&mut jobs, Job::Related(seed.clone(), id.clone()));
                    }
                }
                Seed::Artist(id) => push(&mut jobs, Job::SeedArtist(seed.clone(), id.clone())),
            }
        }

        // Close artists: two of the favourites (or their neighbours when
        // exploring), skipping ones already used.
        let explore = s.settings.character == Character::Unfamiliar;
        let mut top: Vec<String> = profile
            .top_artists(8)
            .into_iter()
            .map(|(a, _)| a)
            .filter(|a| {
                !s.done_jobs.contains(&Job::Affine {
                    artist: a.clone(),
                    explore,
                })
            })
            .collect();
        self.rng().shuffle(&mut top);
        for artist in top.into_iter().take(2) {
            push(&mut jobs, Job::Affine { artist, explore });
        }

        if let Some(m) = s.settings.mood {
            jobs.push(Job::Mood(MoodQuery::Title(m.ytm_title().to_owned())));
        }
        if let Some(a) = &s.settings.activity {
            jobs.push(Job::Mood(MoodQuery::Params(a.params.clone())));
        }
        jobs
    }

    async fn run(&self, job: &Job) -> Fetched {
        let tag = |tracks: Vec<Track>, origin: Origin| -> Vec<Candidate> {
            tracks
                .into_iter()
                .enumerate()
                .map(|(rank, track)| Candidate {
                    track,
                    origin: origin.clone(),
                    rank,
                })
                .collect()
        };
        let mut follow_up = Vec::new();
        let result: Result<Vec<Candidate>> = async {
            Ok(match job {
                Job::Radio(seed, id) => {
                    let mut tracks = self.source.radio(id).await?;
                    // The radio starts with the seed itself.
                    tracks.retain(|t| &t.video_id != id);
                    tag(tracks, Origin::Seed(seed.clone()))
                }
                Job::Related(seed, id) => {
                    tag(self.source.related(id).await?, Origin::Seed(seed.clone()))
                }
                Job::SeedArtist(seed, id) => {
                    let info = self.source.artist(id).await?;
                    if let Some(first) = info.top_tracks.first() {
                        follow_up.push(Job::Radio(seed.clone(), first.video_id.clone()));
                    }
                    tag(info.top_tracks, Origin::Seed(seed.clone()))
                }
                Job::Affine { artist, explore } => {
                    let info = self.source.artist(artist).await?;
                    let mut out = if *explore {
                        Vec::new()
                    } else {
                        tag(info.top_tracks, Origin::Artist(artist.clone()))
                    };
                    let neighbour = {
                        let mut rng = self.rng();
                        (!info.related.is_empty())
                            .then(|| info.related[rng.usize(..info.related.len().min(6))].clone())
                    };
                    if let Some(n) = neighbour
                        && let Ok(info) = self.source.artist(&n.id).await
                    {
                        out.extend(tag(info.top_tracks, Origin::Artist(n.id)));
                    }
                    out
                }
                Job::Mood(q) => tag(self.source.mood_tracks(q).await?, Origin::Mood),
            })
        }
        .await;
        match result {
            Ok(candidates) => Fetched {
                candidates,
                follow_up,
            },
            Err(e) => {
                tracing::warn!(?job, "wave source failed: {e}");
                Fetched {
                    candidates: Vec::new(),
                    follow_up,
                }
            }
        }
    }

    /// Stores a player event and adapts the running session.
    pub async fn record(&self, event: PlayerEvent) -> Result<RecordOutcome> {
        let now = self.now();
        let mut out = RecordOutcome::default();
        match event {
            PlayerEvent::Start { track, in_wave } => {
                self.store.upsert_track(&track, now)?;
                self.store.record_start(&track.video_id, now, in_wave)?;
                let mut s = self.session.lock().await;
                let artists: Vec<String> = track.artists.iter().map(artist_key).collect();
                s.queued.retain(|(id, _)| *id != track.video_id);
                s.played.push_back(artists);
                while s.played.len() > RECENT {
                    s.played.pop_front();
                }
                // Played outside the wave: still never offered again this session.
                s.served.insert(track.video_id);
            }
            PlayerEvent::End {
                track,
                position_s,
                duration_s,
                ended,
                in_wave,
            } => {
                let duration = duration_s.or(track.duration_s.map(f64::from));
                let outcome = PlayOutcome::classify(position_s, duration, ended);
                out.outcome = Some(outcome);
                self.store.upsert_track(&track, now)?;
                self.store.record_outcome(
                    &track.video_id,
                    now,
                    outcome,
                    position_s,
                    duration,
                    in_wave,
                )?;
                let mut s = self.session.lock().await;
                if s.active && in_wave {
                    out.reseeded = self.react_to_play(&mut s, &track, outcome)?;
                }
            }
            PlayerEvent::Like { track, liked } => {
                self.store.upsert_track(&track, now)?;
                self.store.set_like(&track.video_id, liked, now)?;
                let mut s = self.session.lock().await;
                if s.active && liked {
                    adjust(&mut s, &track, 0.5);
                    let seed = Seed::Track(track.video_id.clone());
                    if !s.seeds.contains(&seed) {
                        s.seeds.insert(0, seed);
                        s.seeds.truncate(MAX_SEEDS);
                        s.force_refill = true;
                        out.reseeded = true;
                    }
                }
            }
            PlayerEvent::Dislike { track, disliked } => {
                self.store.upsert_track(&track, now)?;
                self.store.set_dislike(&track.video_id, disliked, now)?;
                if disliked {
                    self.store.set_like(&track.video_id, false, now)?;
                }
                let mut s = self.session.lock().await;
                if s.active && disliked {
                    adjust(&mut s, &track, -3.0);
                    let seed = Seed::Track(track.video_id.clone());
                    if s.seeds.contains(&seed) {
                        s.retire(&seed);
                    }
                    if let Some(seed) = s
                        .origin_of
                        .get(&track.video_id)
                        .and_then(Origin::seed)
                        .cloned()
                    {
                        // A dislike counts as two skips for its seed.
                        *s.seed_skips.entry(seed).or_default() += SEED_SKIP_LIMIT;
                    }
                    out.reseeded = self.maybe_reseed(&mut s)?;
                }
            }
        }
        Ok(out)
    }

    fn react_to_play(&self, s: &mut Session, t: &Track, outcome: PlayOutcome) -> Result<bool> {
        match outcome {
            PlayOutcome::Complete => adjust(s, t, 0.2),
            PlayOutcome::EarlySkip => adjust(s, t, -1.0),
            PlayOutcome::LateSkip => adjust(s, t, -0.3),
        }
        if outcome == PlayOutcome::Complete {
            s.completed.push(t.video_id.clone());
        }
        let Some(seed) = s.origin_of.get(&t.video_id).and_then(Origin::seed).cloned() else {
            return Ok(false);
        };
        if outcome.is_skip() {
            *s.seed_skips.entry(seed).or_default() += 1;
        } else {
            s.seed_skips.insert(seed, 0);
        }
        self.maybe_reseed(s)
    }

    /// Retires seeds with too many skips in a row and picks replacements.
    fn maybe_reseed(&self, s: &mut Session) -> Result<bool> {
        let worn: Vec<Seed> = s
            .seed_skips
            .iter()
            .filter(|(seed, n)| **n >= SEED_SKIP_LIMIT && !s.retired.contains(*seed))
            .map(|(seed, _)| seed.clone())
            .collect();
        if worn.is_empty() {
            return Ok(false);
        }
        for seed in &worn {
            tracing::info!(?seed, "wave: retiring seed after skips");
            s.retire(seed);
        }
        let profile = self.profile()?;
        let want = SEEDS.saturating_sub(s.seeds.len()).max(1);
        let fresh = self.pick_seeds(&profile, s, want);
        tracing::info!(?fresh, "wave: new seeds");
        s.seeds.extend(fresh);
        s.force_refill = true;
        Ok(true)
    }
}

fn adjust(s: &mut Session, t: &Track, delta: f64) {
    if let Some(a) = t.artists.first() {
        *s.session_artist.entry(artist_key(a)).or_default() += delta;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ArtistInfo;
    use crate::store::tests::track;
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Every radio is the same three-artist mix; artist pages are empty.
    struct Fixed;

    impl CandidateSource for Fixed {
        async fn radio(&self, seed: &str) -> Result<Vec<Track>> {
            Ok((0..30)
                .map(|i| track(&format!("{seed}-{i}"), ["A", "B", "C"][i % 3]))
                .collect())
        }
        async fn related(&self, _: &str) -> Result<Vec<Track>> {
            Ok(vec![])
        }
        async fn artist(&self, _: &str) -> Result<ArtistInfo> {
            Ok(ArtistInfo::default())
        }
        async fn mood_tracks(&self, _: &MoodQuery) -> Result<Vec<Track>> {
            Ok((0..10).map(|i| track(&format!("mood-{i}"), "M")).collect())
        }
    }

    fn wave() -> (Wave<Fixed>, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(1_000 * DAY));
        let clock = t.clone();
        let w = Wave::new(Store::open_in_memory().unwrap(), Fixed)
            .with_test_hooks(7, Arc::new(move || clock.load(Ordering::SeqCst)));
        (w, t)
    }

    async fn play(w: &Wave<Fixed>, t: &Track, position: f64) -> RecordOutcome {
        w.record(PlayerEvent::Start {
            track: t.clone(),
            in_wave: true,
        })
        .await
        .unwrap();
        w.record(PlayerEvent::End {
            track: t.clone(),
            position_s: position,
            duration_s: Some(200.0),
            ended: position >= 200.0,
            in_wave: true,
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn cold_wave_refuses_to_start() {
        let (w, _) = wave();
        assert!(w.status().await.unwrap().cold);
        assert!(matches!(w.start(3).await, Err(Error::NoCandidates)));
    }

    #[tokio::test]
    async fn no_repeats_and_two_skips_retire_a_seed() {
        let (w, _) = wave();
        w.store().upsert_track(&track("s1", "A"), 0).unwrap();
        w.store().set_like("s1", true, 1_000 * DAY).unwrap();

        let first = w.start(5).await.unwrap();
        assert_eq!(first.len(), 5);
        let more = w.next(10).await.unwrap();
        let mut ids: Vec<_> = first
            .iter()
            .chain(&more)
            .map(|t| t.video_id.clone())
            .collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "a track was served twice");
        // Spacing: no artist twice in a row.
        for pair in first.iter().chain(&more).collect::<Vec<_>>().windows(2) {
            assert_ne!(pair[0].artists[0].name, pair[1].artists[0].name);
        }

        // Two early skips of tracks from seed s1 retire it.
        let from_seed: Vec<&Track> = first
            .iter()
            .filter(|t| t.video_id.starts_with("s1-"))
            .collect();
        assert!(from_seed.len() >= 2);
        assert!(!play(&w, from_seed[0], 5.0).await.reseeded);
        assert!(play(&w, from_seed[1], 5.0).await.reseeded);
        let s = w.session.lock().await;
        assert!(s.retired.contains(&Seed::Track("s1".into())));
        assert!(
            s.pool
                .iter()
                .all(|c| c.origin.seed() != Some(&Seed::Track("s1".into())))
        );
    }

    #[tokio::test]
    async fn completed_plays_reset_the_skip_streak_and_likes_add_seeds() {
        let (w, _) = wave();
        w.store().upsert_track(&track("s1", "A"), 0).unwrap();
        w.store().set_like("s1", true, 1_000 * DAY).unwrap();
        let batch = w.start(6).await.unwrap();
        let a: Vec<&Track> = batch
            .iter()
            .filter(|t| t.video_id.starts_with("s1-"))
            .collect();
        play(&w, a[0], 5.0).await;
        play(&w, a[1], 200.0).await;
        assert!(!play(&w, a[2], 5.0).await.reseeded);

        let liked = track("other", "Z");
        let out = w
            .record(PlayerEvent::Like {
                track: liked,
                liked: true,
            })
            .await
            .unwrap();
        assert!(out.reseeded);
        assert_eq!(w.session.lock().await.seeds[0], Seed::Track("other".into()));
    }

    #[tokio::test]
    async fn mood_settings_bring_in_mood_tracks() {
        let (w, _) = wave();
        w.store().upsert_track(&track("s1", "A"), 0).unwrap();
        w.store().set_like("s1", true, 1_000 * DAY).unwrap();
        w.set_settings(WaveSettings {
            mood: Some(crate::settings::Mood::Calm),
            ..Default::default()
        })
        .await
        .unwrap();
        let batch = w.start(10).await.unwrap();
        assert!(batch.iter().any(|t| t.artists[0].name == "M"));
    }

    #[tokio::test]
    async fn changing_the_mood_mid_session_is_heard_at_once() {
        let (w, _) = wave();
        w.store().upsert_track(&track("s1", "A"), 0).unwrap();
        w.store().set_like("s1", true, 1_000 * DAY).unwrap();
        let first = w.start(3).await.unwrap();
        assert!(first.iter().all(|t| t.artists[0].name != "M"));
        w.set_settings(WaveSettings {
            mood: Some(crate::settings::Mood::Calm),
            ..Default::default()
        })
        .await
        .unwrap();
        let next = w.next(4).await.unwrap();
        assert!(
            next.iter().filter(|t| t.artists[0].name == "M").count() >= 2,
            "{next:?}"
        );
    }

    #[tokio::test]
    async fn requeue_returns_dropped_tracks_to_the_pool_side() {
        let (w, _) = wave();
        w.store().upsert_track(&track("s1", "A"), 0).unwrap();
        w.store().set_like("s1", true, 1_000 * DAY).unwrap();
        let batch = w.start(3).await.unwrap();
        let dropped: Vec<String> = batch.iter().map(|t| t.video_id.clone()).collect();
        let fresh = w.requeue(&dropped, 3).await.unwrap();
        assert_eq!(fresh.len(), 3);
        let s = w.session.lock().await;
        assert_eq!(s.queued.len(), 3);
    }
}
