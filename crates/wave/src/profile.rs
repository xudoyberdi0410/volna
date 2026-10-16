//! The listener's taste at one moment, folded from a [`Snapshot`].

use std::collections::{HashMap, HashSet};

use volna_ytm::Track;

use crate::signal::{PlayOutcome, Weights, decay};
use crate::store::{SeedArtist, Snapshot, artist_key};

/// Seeds should be fresh: their weight fades over two weeks.
const SEED_FRESHNESS_DAYS: f64 = 14.0;
/// With fewer completed plays than this (and no likes or picks) the wave
/// asks for artists first.
const COLD_START_PLAYS: usize = 3;

/// Decayed scores per artist and track plus the hard facts (bans, likes).
#[derive(Debug, Clone, Default)]
pub struct TasteProfile {
    /// Artist key → affinity (positive = liked).
    pub artist: HashMap<String, f64>,
    /// Artist key → number of completed plays or likes (familiarity).
    pub artist_known: HashMap<String, u32>,
    /// Track id → affinity from its own plays and likes.
    pub track: HashMap<String, f64>,
    pub banned: HashSet<String>,
    pub liked: HashSet<String>,
    pub last_started: HashMap<String, i64>,
    pub seed_artists: Vec<SeedArtist>,
    /// Track ids that make good radio seeds, with a weight (fresh likes and
    /// completed plays score highest).
    pub seed_tracks: Vec<(String, f64)>,
    completes: usize,
}

impl TasteProfile {
    pub fn build(snap: &Snapshot, w: &Weights, now: i64) -> Self {
        let mut p = TasteProfile {
            last_started: snap.last_started.clone(),
            ..Default::default()
        };
        let mut seeds: HashMap<String, f64> = HashMap::new();
        let artists_of = |id: &str| snap.track_artists.get(id).map(Vec::as_slice).unwrap_or(&[]);

        for play in &snap.plays {
            let age = now - play.ts;
            let value = w.outcome(play.outcome, play.in_wave) * decay(age, w.play_half_life_days);
            *p.track.entry(play.video_id.clone()).or_default() += value;
            p.add_to_artists(artists_of(&play.video_id), value);
            if play.outcome == PlayOutcome::Complete {
                p.completes += 1;
                for a in artists_of(&play.video_id) {
                    *p.artist_known.entry(artist_key(a)).or_default() += 1;
                }
                *seeds.entry(play.video_id.clone()).or_default() +=
                    w.complete * decay(age, SEED_FRESHNESS_DAYS);
            } else {
                // A skipped track is a poor seed even if it was liked once.
                *seeds.entry(play.video_id.clone()).or_default() +=
                    w.outcome(play.outcome, play.in_wave) * decay(age, SEED_FRESHNESS_DAYS);
            }
        }

        for (id, ts) in &snap.likes {
            let age = now - ts;
            let value = w.like * decay(age, w.like_half_life_days);
            p.liked.insert(id.clone());
            *p.track.entry(id.clone()).or_default() += value;
            p.add_to_artists(artists_of(id), value);
            for a in artists_of(id) {
                *p.artist_known.entry(artist_key(a)).or_default() += 1;
            }
            // Likes stay usable seeds for longer than plays.
            *seeds.entry(id.clone()).or_default() += w.like * decay(age, SEED_FRESHNESS_DAYS * 4.0);
        }

        for (id, ts) in &snap.dislikes {
            p.banned.insert(id.clone());
            let value = w.dislike_artist * decay(now - ts, w.dislike_half_life_days);
            // Only the main artist: a feature on a bad track is not a verdict.
            p.add_to_artists(artists_of(id).get(..1).unwrap_or(&[]), value);
            seeds.remove(id);
        }

        for (a, ts) in &snap.seed_artists {
            *p.artist.entry(a.id.clone()).or_default() +=
                w.seed_artist * decay(now - ts, w.seed_half_life_days);
            *p.artist_known.entry(a.id.clone()).or_default() += 1;
            p.seed_artists.push(a.clone());
        }

        p.seed_tracks = seeds
            .into_iter()
            .filter(|(id, w)| *w > 0.05 && !p.banned.contains(id))
            .collect();
        p.seed_tracks
            .sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        p
    }

    /// Main artist counts fully, featured ones half.
    fn add_to_artists(&mut self, artists: &[volna_ytm::ArtistRef], value: f64) {
        for (i, a) in artists.iter().enumerate() {
            let share = if i == 0 { 1.0 } else { 0.5 };
            *self.artist.entry(artist_key(a)).or_default() += value * share;
        }
    }

    /// Nothing to build a wave from yet: show the cold-start screen.
    pub fn is_cold(&self) -> bool {
        self.liked.is_empty() && self.seed_artists.is_empty() && self.completes < COLD_START_PLAYS
    }

    /// Affinity of a candidate's artists (main fully, others half).
    pub fn artist_affinity(&self, t: &Track) -> f64 {
        t.artists
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let v = self.artist.get(&artist_key(a)).copied().unwrap_or(0.0);
                if i == 0 { v } else { v * 0.5 }
            })
            .sum()
    }

    /// The lowest affinity among the track's artists (used for bans: one
    /// hated artist on a track is enough).
    pub fn worst_artist(&self, t: &Track) -> f64 {
        t.artists
            .iter()
            .take(1)
            .map(|a| self.artist.get(&artist_key(a)).copied().unwrap_or(0.0))
            .fold(0.0, f64::min)
    }

    /// Whether the listener knows any of the track's artists.
    pub fn knows_artist(&self, t: &Track) -> bool {
        t.artists
            .iter()
            .any(|a| self.artist_known.get(&artist_key(a)).copied().unwrap_or(0) > 0)
    }

    /// Artists ordered by affinity, best first, positive only.
    pub fn top_artists(&self, n: usize) -> Vec<(String, f64)> {
        let mut v: Vec<_> = self
            .artist
            .iter()
            .filter(|(k, v)| **v > 0.0 && !k.starts_with("name:"))
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        v.truncate(n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::DAY;
    use crate::store::{PlayRow, tests::track};

    fn snap_with(plays: &[(&str, &str, PlayOutcome, i64)]) -> Snapshot {
        let mut s = Snapshot::default();
        for (id, artist, outcome, ts) in plays {
            s.track_artists
                .insert((*id).into(), track(id, artist).artists.clone());
            s.plays.push(PlayRow {
                ts: *ts,
                video_id: (*id).into(),
                outcome: *outcome,
                in_wave: true,
            });
        }
        s
    }

    #[test]
    fn plays_skips_likes_and_dislikes_move_artists() {
        let now = 100 * DAY;
        let mut s = snap_with(&[
            ("a1", "A", PlayOutcome::Complete, now),
            ("a2", "A", PlayOutcome::Complete, now),
            ("b1", "B", PlayOutcome::EarlySkip, now),
            ("c1", "C", PlayOutcome::LateSkip, now),
        ]);
        s.likes.push(("a1".into(), now));
        s.track_artists
            .insert("d1".into(), track("d1", "D").artists);
        s.dislikes.push(("d1".into(), now));

        let p = TasteProfile::build(&s, &Weights::default(), now);
        assert!((p.artist["UCA"] - 5.0).abs() < 1e-9);
        assert!((p.artist["UCB"] + 1.5).abs() < 1e-9);
        assert!((p.artist["UCC"] + 0.4).abs() < 1e-9);
        assert!((p.artist["UCD"] + 4.0).abs() < 1e-9);
        assert!(p.banned.contains("d1"));
        assert_eq!(p.seed_tracks[0].0, "a1");
        assert!(p.seed_tracks.iter().all(|(id, _)| id != "b1"));
        assert!(!p.is_cold());
        assert_eq!(p.top_artists(5)[0].0, "UCA");
    }

    #[test]
    fn old_signals_fade() {
        let now = 1000 * DAY;
        let s = snap_with(&[("a1", "A", PlayOutcome::Complete, now - 30 * DAY)]);
        let p = TasteProfile::build(&s, &Weights::default(), now);
        assert!((p.artist["UCA"] - 0.5).abs() < 1e-9);
    }

    #[test]
    fn cold_until_there_is_something_to_go_on() {
        let p = TasteProfile::build(&Snapshot::default(), &Weights::default(), 0);
        assert!(p.is_cold());
        let mut s = Snapshot::default();
        s.seed_artists.push((
            SeedArtist {
                id: "UCX".into(),
                name: "X".into(),
            },
            0,
        ));
        let p = TasteProfile::build(&s, &Weights::default(), 0);
        assert!(!p.is_cold());
        assert!(p.artist["UCX"] > 0.0);
    }
}
