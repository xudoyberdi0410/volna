//! Turns playback progress into plays: a port of `PlayTracker` from
//! `ui/js/wave-logic.js`, plus the 10 s history mark of `ui/js/events.js`.
//!
//! A play starts once a trusted position passes 0.5 s, and ends when the
//! player switches away, stops or reaches the end. Plays that never started
//! produce nothing. Plays are keyed by queue `uid`, so a repeated track is a
//! new play after [`PlayTracker::ended`].

use crate::model::PlayerTrack;

/// Seconds of listening after which a play goes to the history.
pub const HISTORY_AFTER_S: f64 = 10.0;
const START_AFTER_S: f64 = 0.5;

#[derive(Debug, Clone, PartialEq)]
pub enum PlayEvent {
    Start {
        track: PlayerTrack,
        in_wave: bool,
        at: f64,
    },
    History {
        track: PlayerTrack,
        at: f64,
    },
    End {
        track: PlayerTrack,
        started_at: f64,
        position: f64,
        duration: Option<f64>,
        ended: bool,
        in_wave: bool,
        at: f64,
    },
    /// «Не нравится» on a wave track (sent after its `End`).
    Dislike {
        track: PlayerTrack,
        at: f64,
    },
}

#[derive(Debug)]
struct Current {
    track: PlayerTrack,
    in_wave: bool,
    position: f64,
    duration: Option<f64>,
    started_at: Option<f64>,
    trusted: bool,
    ticks: u32,
    history: bool,
}

#[derive(Debug, Default)]
pub struct PlayTracker {
    current: Option<Current>,
}

impl PlayTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// The uid of the play being followed.
    pub fn uid(&self) -> Option<&str> {
        self.current.as_ref().map(|c| c.track.uid.as_str())
    }

    pub fn started(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|c| c.started_at.is_some())
    }

    /// A position sample of the active deck playing `track`.
    pub fn tick(
        &mut self,
        track: &PlayerTrack,
        in_wave: bool,
        position: f64,
        duration: Option<f64>,
        now: f64,
    ) -> Vec<PlayEvent> {
        let mut out = Vec::new();
        if self.uid() != Some(track.uid.as_str()) {
            out.extend(self.leave(now));
            self.current = Some(Current {
                track: track.clone(),
                in_wave,
                position: 0.0,
                duration: None,
                started_at: None,
                trusted: false,
                ticks: 0,
                history: false,
            });
        }
        let c = self.current.as_mut().expect("just set");
        c.ticks += 1;
        if let Some(d) = duration.filter(|d| d.is_finite() && *d > 0.0) {
            c.duration = Some(d);
        }
        if position.is_finite() {
            // Right after a switch the pipeline may still report the old
            // track's time; trust positions near the start, or after a few
            // ticks (for tracks resumed mid-way).
            if !c.trusted && (position < 3.0 || c.ticks > 4) {
                c.trusted = true;
            }
            if c.trusted {
                c.position = position.max(0.0);
            }
        }
        if c.started_at.is_none() && c.trusted && c.position > START_AFTER_S {
            c.started_at = Some(now);
            out.push(PlayEvent::Start {
                track: c.track.clone(),
                in_wave: c.in_wave,
                at: now,
            });
        }
        if c.started_at.is_some() && !c.history && c.position >= HISTORY_AFTER_S {
            c.history = true;
            out.push(PlayEvent::History {
                track: c.track.clone(),
                at: now,
            });
        }
        out
    }

    /// The track `uid` played to its end.
    pub fn ended(&mut self, uid: &str, now: f64) -> Vec<PlayEvent> {
        if self.uid() != Some(uid) {
            return Vec::new();
        }
        let c = self.current.as_mut().expect("checked");
        if let Some(d) = c.duration.or(c.track.duration) {
            c.position = d;
        }
        self.close(true, now)
    }

    /// The player moved to `uid` (it may not play yet): closes the previous
    /// play if it was another entry.
    pub fn switch_to(&mut self, uid: Option<&str>, now: f64) -> Vec<PlayEvent> {
        if self.current.is_none() || (uid.is_some() && self.uid() == uid) {
            return Vec::new();
        }
        self.leave(now)
    }

    /// Closes the current play (skip, stop, shutdown).
    pub fn leave(&mut self, now: f64) -> Vec<PlayEvent> {
        self.close(false, now)
    }

    fn close(&mut self, ended: bool, now: f64) -> Vec<PlayEvent> {
        let Some(c) = self.current.take() else {
            return Vec::new();
        };
        let Some(started_at) = c.started_at else {
            return Vec::new();
        };
        vec![PlayEvent::End {
            duration: c.duration.or(c.track.duration),
            track: c.track,
            started_at,
            position: c.position,
            ended,
            in_wave: c.in_wave,
            at: now,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrackInput;

    fn track(uid: &str) -> PlayerTrack {
        PlayerTrack::from_input(
            uid.into(),
            TrackInput {
                id: "dQw4w9WgXcQ".into(),
                duration: Some(200.0),
                ..Default::default()
            },
        )
    }

    fn kinds(evs: &[PlayEvent]) -> Vec<&'static str> {
        evs.iter()
            .map(|e| match e {
                PlayEvent::Start { .. } => "start",
                PlayEvent::History { .. } => "history",
                PlayEvent::End { .. } => "end",
                PlayEvent::Dislike { .. } => "dislike",
            })
            .collect()
    }

    #[test]
    fn start_after_half_a_second_then_history_then_end() {
        let mut t = PlayTracker::new();
        let a = track("q1");
        assert!(t.tick(&a, true, 0.2, Some(200.0), 1.0).is_empty());
        let e = t.tick(&a, true, 0.6, Some(200.0), 2.0);
        assert_eq!(kinds(&e), ["start"]);
        assert!(t.tick(&a, true, 5.0, None, 3.0).is_empty());
        assert_eq!(kinds(&t.tick(&a, true, 10.0, None, 4.0)), ["history"]);
        assert!(t.tick(&a, true, 11.0, None, 5.0).is_empty());
        let e = t.ended("q1", 6.0);
        match &e[..] {
            [
                PlayEvent::End {
                    position,
                    ended,
                    started_at,
                    in_wave,
                    ..
                },
            ] => {
                assert_eq!(*position, 200.0);
                assert!(*ended && *in_wave);
                assert_eq!(*started_at, 2.0);
            }
            other => panic!("{other:?}"),
        }
        assert!(t.uid().is_none());
    }

    #[test]
    fn switching_closes_the_previous_play() {
        let mut t = PlayTracker::new();
        let (a, b) = (track("q1"), track("q2"));
        t.tick(&a, false, 1.0, None, 1.0);
        assert!(t.switch_to(Some("q1"), 2.0).is_empty());
        let e = t.switch_to(Some("q2"), 2.0);
        assert_eq!(kinds(&e), ["end"]);
        // A track that never started gives nothing.
        t.tick(&b, false, 0.0, None, 3.0);
        assert!(t.leave(4.0).is_empty());
        // A tick of another uid closes the old play first.
        t.tick(&a, false, 1.0, None, 5.0);
        let e = t.tick(&b, false, 1.0, None, 6.0);
        assert_eq!(kinds(&e), ["end", "start"]);
    }

    #[test]
    fn stale_positions_are_not_trusted() {
        let mut t = PlayTracker::new();
        let a = track("q1");
        // The old track's time leaks in for a few ticks.
        for i in 0..4 {
            assert!(t.tick(&a, false, 150.0, None, f64::from(i)).is_empty());
        }
        // Fifth tick: resumed mid-way, trusted now.
        assert_eq!(
            kinds(&t.tick(&a, false, 150.0, None, 5.0)),
            ["start", "history"]
        );
        match &t.leave(6.0)[..] {
            [
                PlayEvent::End {
                    position, ended, ..
                },
            ] => {
                assert_eq!(*position, 150.0);
                assert!(!*ended);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn repeat_one_is_a_new_play() {
        let mut t = PlayTracker::new();
        let a = track("q1");
        t.tick(&a, false, 1.0, None, 1.0);
        assert_eq!(kinds(&t.ended("q1", 2.0)), ["end"]);
        assert_eq!(kinds(&t.tick(&a, false, 1.0, None, 3.0)), ["start"]);
        assert!(t.ended("other", 4.0).is_empty());
    }
}
