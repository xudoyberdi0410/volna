//! Ranking candidates: pure functions over a taste profile, the session and
//! the settings, so they can be tested without any I/O.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use volna_ytm::{Track, TrackKind};

use crate::lang;
use crate::profile::TasteProfile;
use crate::settings::{Character, Language, WaveSettings};
use crate::signal::DAY;
use crate::store::artist_key;

/// What a radio was started from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Seed {
    Track(String),
    Artist(String),
}

/// Which generator produced a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// Radio or related tracks of a seed (also artist seeds' top tracks).
    Seed(Seed),
    /// Top tracks of an artist the listener likes, or of one related to it.
    Artist(String),
    /// A playlist of the chosen mood or activity.
    Mood,
}

impl Origin {
    pub fn seed(&self) -> Option<&Seed> {
        match self {
            Origin::Seed(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Candidate {
    pub track: Track,
    pub origin: Origin,
    /// Position in its source list; earlier radio entries fit the seed better.
    pub rank: usize,
}

/// Session facts the score depends on.
#[derive(Debug, Default)]
pub struct SessionView<'a> {
    /// Tracks played or queued in this session: never served again.
    pub served: Option<&'a HashSet<String>>,
    /// Artist keys of the latest tracks (played, then queued), oldest first.
    pub recent_artists: &'a [Vec<String>],
    /// Live adjustments from this session's skips and likes, per artist key.
    pub session_artist: Option<&'a HashMap<String, f64>>,
    /// Artists that appear in the chosen mood's playlists.
    pub mood_artists: Option<&'a HashSet<String>>,
}

/// Tunables of the ranking.
#[derive(Debug, Clone, Copy)]
pub struct ScoreParams {
    /// Tracks started within this window are not offered again.
    pub repeat_block_s: i64,
    /// Liked tracks may come back sooner with the "Любимое" character.
    pub favorite_repeat_block_s: i64,
    /// Played within this window → a fading novelty penalty.
    pub novelty_s: i64,
    /// Artists at or below this affinity are not played at all.
    pub ban_threshold: f64,
    /// No artist twice within this many tracks (relaxed if impossible).
    pub artist_spacing: usize,
    /// Longest track the wave offers (skips hour-long mixes).
    pub max_duration_s: u32,
}

impl Default for ScoreParams {
    fn default() -> Self {
        Self {
            repeat_block_s: 3 * DAY,
            favorite_repeat_block_s: DAY,
            novelty_s: 14 * DAY,
            ban_threshold: -2.5,
            artist_spacing: 2,
            max_duration_s: 15 * 60,
        }
    }
}

/// Share of picks that explore instead of taking the best candidate.
pub fn epsilon(character: Character) -> f64 {
    match character {
        Character::Favorite => 0.05,
        Character::Unfamiliar => 0.25,
        Character::Default | Character::Popular => 0.12,
    }
}

/// "57M plays" / "1.2B views" / "830K" → number.
pub fn parse_count(raw: &str) -> Option<u64> {
    let token = raw.split_whitespace().next()?.replace(',', "");
    let (num, mult) = match token.chars().last()? {
        'K' | 'k' => (&token[..token.len() - 1], 1e3),
        'M' | 'm' => (&token[..token.len() - 1], 1e6),
        'B' | 'b' => (&token[..token.len() - 1], 1e9),
        _ => (token.as_str(), 1.0),
    };
    let n: f64 = num.parse().ok()?;
    Some((n * mult) as u64)
}

/// 0 for ≤100K plays, 1 for ≥1B, log scale between.
pub fn popularity(t: &Track) -> f64 {
    t.views
        .as_deref()
        .and_then(parse_count)
        .map(|n| (((n.max(1) as f64).log10() - 5.0) / 4.0).clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

fn artist_keys(t: &Track) -> Vec<String> {
    t.artists.iter().map(artist_key).collect()
}

fn session_adjust(t: &Track, s: &SessionView<'_>) -> f64 {
    let Some(adj) = s.session_artist else {
        return 0.0;
    };
    t.artists
        .iter()
        .take(1)
        .map(|a| adj.get(&artist_key(a)).copied().unwrap_or(0.0))
        .sum()
}

/// Hard filters: bans, repeats, unplayable or overlong tracks.
pub fn eligible(
    c: &Candidate,
    p: &TasteProfile,
    s: &SessionView<'_>,
    settings: &WaveSettings,
    params: &ScoreParams,
    now: i64,
) -> bool {
    let t = &c.track;
    if !t.playable || t.video_id.is_empty() || p.banned.contains(&t.video_id) {
        return false;
    }
    if s.served.is_some_and(|served| served.contains(&t.video_id)) {
        return false;
    }
    if t.duration_s.is_some_and(|d| d > params.max_duration_s) {
        return false;
    }
    if p.worst_artist(t) + session_adjust(t, s).min(0.0) <= params.ban_threshold {
        return false;
    }
    if let Some(started) = p.last_started.get(&t.video_id) {
        let block = if settings.character == Character::Favorite && p.liked.contains(&t.video_id) {
            params.favorite_repeat_block_s
        } else {
            params.repeat_block_s
        };
        if now - started < block {
            return false;
        }
    }
    true
}

/// Higher is better. Only meaningful for [`eligible`] candidates.
pub fn score(
    c: &Candidate,
    p: &TasteProfile,
    s: &SessionView<'_>,
    settings: &WaveSettings,
    params: &ScoreParams,
    now: i64,
) -> f64 {
    let t = &c.track;
    let affinity = (p.artist_affinity(t) / 3.0).tanh() + (session_adjust(t, s) / 2.0).tanh();
    let track_affinity = (p.track.get(&t.video_id).copied().unwrap_or(0.0) / 2.0).tanh();
    let mut score = affinity + 0.5 * track_affinity;

    // Novelty: fading penalty for tracks heard in the last two weeks.
    if let Some(started) = p.last_started.get(&t.video_id) {
        let age = (now - started).max(0);
        if age < params.novelty_s {
            score -= 0.6 * (1.0 - age as f64 / params.novelty_s as f64);
        }
    }

    let known = p.knows_artist(t);
    match settings.character {
        Character::Default => {
            if known {
                score += 0.2;
            }
        }
        Character::Favorite => {
            score += 0.8 * affinity.max(0.0);
            if p.liked.contains(&t.video_id) {
                score += 0.6;
            }
            if known {
                score += 0.4;
            }
        }
        Character::Unfamiliar => {
            score += if known { -0.8 } else { 0.5 };
            // Liking an artist must not drag them back in here.
            score -= 0.5 * affinity.max(0.0);
        }
        Character::Popular => score += 1.2 * popularity(t),
    }

    if settings.wants_mood() {
        if c.origin == Origin::Mood {
            score += 0.6;
        }
        if s.mood_artists
            .is_some_and(|m| artist_keys(t).iter().any(|k| m.contains(k)))
        {
            score += 0.3;
        }
    }

    if settings.language != Language::Any {
        let names: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
        score += match lang::detect(&t.title, &names) {
            l if l == settings.language => 0.4,
            Language::Any => -0.2,
            _ => -1.2,
        };
    }

    score -= (0.01 * c.rank as f64).min(0.3);
    score -= match t.kind {
        TrackKind::Song => 0.0,
        TrackKind::OfficialVideo => 0.05,
        TrackKind::Video => 0.3,
    };

    // Soft spacing beyond the hard window: an artist already heard a lot
    // lately keeps losing points.
    let keys = artist_keys(t);
    let repeats = s
        .recent_artists
        .iter()
        .rev()
        .take(10)
        .filter(|ks| ks.iter().any(|k| keys.contains(k)))
        .count();
    score -= 0.35 * repeats as f64;
    score
}

/// Picks the next track: the best eligible candidate that keeps artist
/// spacing, or with probability ε a random one from the better half.
/// Returns the index into `pool`.
pub fn pick(
    pool: &[Candidate],
    p: &TasteProfile,
    s: &SessionView<'_>,
    settings: &WaveSettings,
    params: &ScoreParams,
    now: i64,
    rng: &mut fastrand::Rng,
) -> Option<usize> {
    let ok: Vec<usize> = (0..pool.len())
        .filter(|&i| eligible(&pool[i], p, s, settings, params, now))
        .collect();
    if ok.is_empty() {
        return None;
    }

    let blocked: HashSet<&String> = s
        .recent_artists
        .iter()
        .rev()
        .take(params.artist_spacing)
        .flatten()
        .collect();
    let spaced: Vec<usize> = ok
        .iter()
        .copied()
        .filter(|&i| {
            !artist_keys(&pool[i].track)
                .iter()
                .any(|k| blocked.contains(k))
        })
        .collect();
    let choices = if spaced.is_empty() { ok } else { spaced };

    // Small jitter so equal scores (one radio) don't always come in order.
    let mut scored: Vec<(usize, f64)> = choices
        .into_iter()
        .map(|i| {
            (
                i,
                score(&pool[i], p, s, settings, params, now) + rng.f64() * 0.1,
            )
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    if scored.len() > 1 && rng.f64() < epsilon(settings.character) {
        let half = scored.len().div_ceil(2);
        return Some(scored[rng.usize(..half)].0);
    }
    Some(scored[0].0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Mood;
    use crate::signal::{PlayOutcome, Weights};
    use crate::store::{PlayRow, Snapshot, tests::track};

    const NOW: i64 = 1_000 * DAY;

    fn cand(id: &str, artist: &str) -> Candidate {
        Candidate {
            track: track(id, artist),
            origin: Origin::Seed(Seed::Track("seed".into())),
            rank: 0,
        }
    }

    fn profile(plays: &[(&str, &str, PlayOutcome)], likes: &[(&str, &str)]) -> TasteProfile {
        let mut s = Snapshot::default();
        for (id, artist, outcome) in plays {
            s.track_artists
                .insert((*id).into(), track(id, artist).artists);
            s.plays.push(PlayRow {
                ts: NOW,
                video_id: (*id).into(),
                outcome: *outcome,
                in_wave: true,
            });
        }
        for (id, artist) in likes {
            s.track_artists
                .insert((*id).into(), track(id, artist).artists);
            s.likes.push(((*id).into(), NOW));
        }
        TasteProfile::build(&s, &Weights::default(), NOW)
    }

    fn sc(c: &Candidate, p: &TasteProfile, settings: &WaveSettings) -> f64 {
        score(
            c,
            p,
            &SessionView::default(),
            settings,
            &ScoreParams::default(),
            NOW,
        )
    }

    #[test]
    fn liked_artists_rank_above_skipped_ones() {
        let p = profile(
            &[("x", "Skipped", PlayOutcome::EarlySkip)],
            &[("y", "Loved")],
        );
        let s = WaveSettings::default();
        assert!(sc(&cand("n1", "Loved"), &p, &s) > sc(&cand("n2", "Nobody"), &p, &s));
        assert!(sc(&cand("n2", "Nobody"), &p, &s) > sc(&cand("n3", "Skipped"), &p, &s));
    }

    #[test]
    fn character_changes_the_order() {
        let p = profile(&[], &[("y", "Loved")]);
        let known = cand("k", "Loved");
        let unknown = cand("u", "Fresh");
        let fav = WaveSettings {
            character: Character::Favorite,
            ..Default::default()
        };
        let unfamiliar = WaveSettings {
            character: Character::Unfamiliar,
            ..Default::default()
        };
        assert!(sc(&known, &p, &fav) > sc(&unknown, &p, &fav));
        assert!(sc(&unknown, &p, &unfamiliar) > sc(&known, &p, &unfamiliar));

        let mut hit = cand("h", "Fresh");
        hit.track.views = Some("1.2B plays".into());
        let mut niche = cand("n", "Fresh2");
        niche.track.views = Some("3K plays".into());
        let popular = WaveSettings {
            character: Character::Popular,
            ..Default::default()
        };
        assert!(sc(&hit, &p, &popular) > sc(&niche, &p, &popular) + 1.0);
    }

    #[test]
    fn language_and_mood_preferences() {
        let p = TasteProfile::default();
        let mut ru = cand("r", "Скриптонит");
        ru.track.title = "Привычка".into();
        let mut en = cand("e", "The Weeknd");
        en.track.title = "Blinding Lights".into();
        let want_ru = WaveSettings {
            language: Language::Russian,
            ..Default::default()
        };
        assert!(sc(&ru, &p, &want_ru) > sc(&en, &p, &want_ru) + 1.0);

        let mood = WaveSettings {
            mood: Some(Mood::Calm),
            ..Default::default()
        };
        let mut from_mood = cand("m", "A");
        from_mood.origin = Origin::Mood;
        assert!(sc(&from_mood, &p, &mood) > sc(&cand("o", "B"), &p, &mood));
    }

    #[test]
    fn hard_filters() {
        let params = ScoreParams::default();
        let mut p = profile(
            &[
                ("s1", "Hated", PlayOutcome::EarlySkip),
                ("s2", "Hated", PlayOutcome::EarlySkip),
            ],
            &[],
        );
        let s = WaveSettings::default();
        let view = SessionView::default();
        assert!(!eligible(&cand("n", "Hated"), &p, &view, &s, &params, NOW));
        assert!(eligible(&cand("n", "Other"), &p, &view, &s, &params, NOW));

        p.banned.insert("b".into());
        assert!(!eligible(&cand("b", "Other"), &p, &view, &s, &params, NOW));

        p.last_started.insert("r".into(), NOW - DAY);
        assert!(!eligible(&cand("r", "Other"), &p, &view, &s, &params, NOW));
        p.last_started.insert("r".into(), NOW - 4 * DAY);
        assert!(eligible(&cand("r", "Other"), &p, &view, &s, &params, NOW));

        let served: HashSet<String> = ["q".to_owned()].into();
        let view = SessionView {
            served: Some(&served),
            ..Default::default()
        };
        assert!(!eligible(&cand("q", "Other"), &p, &view, &s, &params, NOW));

        let mut long = cand("l", "Other");
        long.track.duration_s = Some(3600);
        assert!(!eligible(&long, &p, &view, &s, &params, NOW));
    }

    #[test]
    fn pick_keeps_artists_apart() {
        let pool = vec![cand("a1", "A"), cand("a2", "A"), cand("b1", "B")];
        let p = profile(&[], &[("x", "A")]);
        let recent = vec![vec!["UCA".to_owned()]];
        let view = SessionView {
            recent_artists: &recent,
            ..Default::default()
        };
        let s = WaveSettings {
            character: Character::Favorite,
            ..Default::default()
        };
        let mut rng = fastrand::Rng::with_seed(1);
        for _ in 0..20 {
            let i = pick(&pool, &p, &view, &s, &ScoreParams::default(), NOW, &mut rng).unwrap();
            assert_eq!(pool[i].track.video_id, "b1");
        }
        // Only A left: spacing relaxes rather than stalling the wave.
        let only_a = &pool[..2];
        assert!(
            pick(
                only_a,
                &p,
                &view,
                &s,
                &ScoreParams::default(),
                NOW,
                &mut rng
            )
            .is_some()
        );
    }

    #[test]
    fn counts_parse() {
        assert_eq!(parse_count("57M plays"), Some(57_000_000));
        assert_eq!(parse_count("1.2B views"), Some(1_200_000_000));
        assert_eq!(parse_count("830K"), Some(830_000));
        assert_eq!(parse_count("1,234 plays"), Some(1234));
        assert_eq!(parse_count("plays"), None);
    }
}
