//! Listening signals and how much each one counts.
//!
//! Per PLAN.md: a play past 70 % is a plus, a skip in the first 30 s a
//! strong minus, a later skip a weak minus, a like a double plus, a dislike
//! bans the track and pushes its artist down hard. Every signal fades with
//! its own half-life.

use serde::{Deserialize, Serialize};

/// Seconds in a day.
pub const DAY: i64 = 86_400;

/// What a finished (or abandoned) play turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayOutcome {
    /// Listened past [`COMPLETE_RATIO`] or to the end.
    Complete,
    /// Skipped within [`EARLY_SKIP_S`].
    EarlySkip,
    /// Skipped later than that but before the end.
    LateSkip,
}

/// Share of the track that counts as "listened".
pub const COMPLETE_RATIO: f64 = 0.7;
/// A skip before this many seconds is an early (strong) skip.
pub const EARLY_SKIP_S: f64 = 30.0;

impl PlayOutcome {
    /// Classifies the end of a play. `ended` is true when the player reached
    /// the end on its own. Unknown durations rely on `ended` and the 30 s rule.
    pub fn classify(position_s: f64, duration_s: Option<f64>, ended: bool) -> Self {
        if ended {
            return PlayOutcome::Complete;
        }
        if let Some(d) = duration_s.filter(|d| *d > 0.0)
            && position_s >= d * COMPLETE_RATIO
        {
            return PlayOutcome::Complete;
        }
        if position_s < EARLY_SKIP_S {
            PlayOutcome::EarlySkip
        } else {
            PlayOutcome::LateSkip
        }
    }

    pub fn is_skip(self) -> bool {
        !matches!(self, PlayOutcome::Complete)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PlayOutcome::Complete => "complete",
            PlayOutcome::EarlySkip => "early_skip",
            PlayOutcome::LateSkip => "late_skip",
        }
    }

    pub(crate) fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "complete" => PlayOutcome::Complete,
            "early_skip" => PlayOutcome::EarlySkip,
            "late_skip" => PlayOutcome::LateSkip,
            _ => return None,
        })
    }
}

/// Weights and half-lives of the signals. One place so tests and tuning
/// agree on the numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Weights {
    pub complete: f64,
    pub early_skip: f64,
    pub late_skip: f64,
    pub like: f64,
    /// Applied to the artist of a disliked track (the track itself is banned).
    pub dislike_artist: f64,
    /// A cold-start artist pick.
    pub seed_artist: f64,
    /// Skips outside the wave (the listener picked something else) are less
    /// telling; they count with this factor.
    pub outside_wave_skip_factor: f64,
    pub play_half_life_days: f64,
    pub like_half_life_days: f64,
    pub dislike_half_life_days: f64,
    pub seed_half_life_days: f64,
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            complete: 1.0,
            early_skip: -1.5,
            late_skip: -0.4,
            like: 3.0,
            dislike_artist: -4.0,
            seed_artist: 2.5,
            outside_wave_skip_factor: 0.5,
            play_half_life_days: 30.0,
            like_half_life_days: 120.0,
            dislike_half_life_days: 90.0,
            seed_half_life_days: 60.0,
        }
    }
}

impl Weights {
    pub fn outcome(&self, outcome: PlayOutcome, in_wave: bool) -> f64 {
        match outcome {
            PlayOutcome::Complete => self.complete,
            PlayOutcome::EarlySkip if in_wave => self.early_skip,
            PlayOutcome::LateSkip if in_wave => self.late_skip,
            PlayOutcome::EarlySkip => self.early_skip * self.outside_wave_skip_factor,
            PlayOutcome::LateSkip => self.late_skip * self.outside_wave_skip_factor,
        }
    }
}

/// Exponential fade: 1 now, 0.5 after one half-life. Future timestamps
/// (clock skew) count as now.
pub fn decay(age_s: i64, half_life_days: f64) -> f64 {
    let age_days = age_s.max(0) as f64 / DAY as f64;
    0.5f64.powf(age_days / half_life_days)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_thresholds() {
        use PlayOutcome::*;
        assert_eq!(PlayOutcome::classify(150.0, Some(200.0), false), Complete);
        assert_eq!(PlayOutcome::classify(139.0, Some(200.0), false), LateSkip);
        assert_eq!(PlayOutcome::classify(12.0, Some(200.0), false), EarlySkip);
        assert_eq!(PlayOutcome::classify(5.0, Some(200.0), true), Complete);
        assert_eq!(PlayOutcome::classify(40.0, None, false), LateSkip);
        // A 20 s jingle listened to 15 s is complete, not an early skip.
        assert_eq!(PlayOutcome::classify(15.0, Some(20.0), false), Complete);
    }

    #[test]
    fn decay_halves_per_half_life() {
        assert!((decay(0, 30.0) - 1.0).abs() < 1e-9);
        assert!((decay(30 * DAY, 30.0) - 0.5).abs() < 1e-9);
        assert!((decay(60 * DAY, 30.0) - 0.25).abs() < 1e-9);
        assert!((decay(-DAY, 30.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn skips_outside_the_wave_count_less() {
        let w = Weights::default();
        assert!(w.outcome(PlayOutcome::EarlySkip, false) > w.outcome(PlayOutcome::EarlySkip, true));
        assert!(w.outcome(PlayOutcome::EarlySkip, true) < w.outcome(PlayOutcome::LateSkip, true));
        assert_eq!(w.outcome(PlayOutcome::Complete, false), w.complete);
    }
}
