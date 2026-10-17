//! The play queue, a port of the queue logic of Monochrome's `ui/js/player.js`
//! (`setQueue`, `addToQueue`, `addNextToQueue`, `removeFromQueue`,
//! `moveInQueue`, `clearQueue`, `wipeQueue`, `toggleShuffle`) keyed by `uid`.
//!
//! Pure: no I/O, no clock. Every mutation goes through [`Queue::commit`],
//! which bumps `rev` by exactly one and describes the change.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::model::{Continuation, PlayerTrack, QueueChanged, QueueSnapshot, Repeat, TrackInput};

#[derive(Debug, Clone, Default)]
pub struct Queue {
    entries: HashMap<String, PlayerTrack>,
    /// Play order (the shuffled one while shuffle is on).
    order: Vec<String>,
    /// Pre-shuffle order, only while shuffle is on.
    original: Option<Vec<String>>,
    index: Option<usize>,
    pub repeat: Repeat,
    rev: u64,
    next_uid: u64,
    added: Vec<String>,
    updated: HashSet<String>,
}

/// What the queue looks like on disk (`persist.rs`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QueueState {
    pub items: Vec<PlayerTrack>,
    pub original: Option<Vec<String>>,
    pub index: Option<usize>,
    pub repeat: Repeat,
    pub next_uid: u64,
}

/// Result of [`Queue::remove`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removed {
    /// Another entry was removed; the current one stays.
    Other,
    /// The current entry was removed and the next one took its place.
    CurrentReplaced,
    /// The current entry was the last one; nothing is current now.
    CurrentGone,
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("ascii digits")
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_state(s: QueueState) -> Self {
        let mut q = Self {
            repeat: s.repeat,
            next_uid: s.next_uid,
            ..Default::default()
        };
        for t in s.items {
            if q.entries.contains_key(&t.uid) {
                continue;
            }
            q.order.push(t.uid.clone());
            q.entries.insert(t.uid.clone(), t);
        }
        q.original = s.original.map(|o| {
            let mut seen = HashSet::new();
            let mut o: Vec<String> = o
                .into_iter()
                .filter(|u| q.entries.contains_key(u) && seen.insert(u.clone()))
                .collect();
            // Anything missing from the saved original goes to its end.
            for u in &q.order {
                if !seen.contains(u) {
                    o.push(u.clone());
                }
            }
            o
        });
        q.index = s.index.filter(|i| *i < q.order.len());
        q
    }

    pub fn state(&self) -> QueueState {
        QueueState {
            items: self.items(),
            original: self.original.clone(),
            index: self.index,
            repeat: self.repeat,
            next_uid: self.next_uid,
        }
    }

    // ---- reading -------------------------------------------------------

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn set_rev(&mut self, rev: u64) {
        self.rev = rev;
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub fn index(&self) -> Option<usize> {
        self.index
    }

    pub fn shuffle(&self) -> bool {
        self.original.is_some()
    }

    pub fn order(&self) -> &[String] {
        &self.order
    }

    pub fn get(&self, uid: &str) -> Option<&PlayerTrack> {
        self.entries.get(uid)
    }

    pub fn at(&self, i: usize) -> Option<&PlayerTrack> {
        self.order.get(i).and_then(|u| self.entries.get(u))
    }

    pub fn position_of(&self, uid: &str) -> Option<usize> {
        self.order.iter().position(|u| u == uid)
    }

    pub fn current(&self) -> Option<&PlayerTrack> {
        self.index.and_then(|i| self.at(i))
    }

    pub fn current_uid(&self) -> Option<&str> {
        self.index
            .and_then(|i| self.order.get(i))
            .map(String::as_str)
    }

    /// Entries in play order.
    pub fn items(&self) -> Vec<PlayerTrack> {
        self.order
            .iter()
            .filter_map(|u| self.entries.get(u).cloned())
            .collect()
    }

    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.values().map(|t| t.id.as_str())
    }

    /// Entries after the current one (`remainingAfter` in wave-logic.js).
    pub fn remaining_after(&self) -> usize {
        match self.index {
            Some(i) => self.order.len().saturating_sub(i + 1),
            None => self.order.len(),
        }
    }

    /// Index of the entry `player_next` moves to: the next playable one,
    /// wrapping with repeat all (the current entry itself included last).
    pub fn next_candidate(&self, playable: impl Fn(&PlayerTrack) -> bool) -> Option<usize> {
        let n = self.order.len();
        let start = self.index.map_or(0, |i| i + 1);
        if let Some(i) = (start..n).find(|&i| self.at(i).is_some_and(&playable)) {
            return Some(i);
        }
        if self.repeat == Repeat::All && n > 0 {
            let end = self.index.map_or(n, |i| i + 1);
            return (0..end).find(|&i| self.at(i).is_some_and(&playable));
        }
        None
    }

    /// Index of the previous playable entry, never wrapping.
    pub fn prev_candidate(&self, playable: impl Fn(&PlayerTrack) -> bool) -> Option<usize> {
        let i = self.index?;
        (0..i).rev().find(|&j| self.at(j).is_some_and(&playable))
    }

    pub fn snapshot(&self, continuation: Continuation) -> QueueSnapshot {
        QueueSnapshot {
            rev: self.rev,
            items: self.items(),
            original: self.original.clone(),
            index: self.index_i64(),
            shuffle: self.shuffle(),
            repeat: self.repeat,
            continuation,
        }
    }

    fn index_i64(&self) -> i64 {
        self.index.map_or(-1, |i| i as i64)
    }

    // ---- change tracking ------------------------------------------------

    /// Closes one mutation: bumps `rev` and describes what changed.
    pub fn commit(&mut self, continuation: Continuation) -> QueueChanged {
        self.rev += 1;
        let added: Vec<PlayerTrack> = std::mem::take(&mut self.added)
            .into_iter()
            .filter_map(|u| self.entries.get(&u).cloned())
            .collect();
        let added_set: HashSet<&str> = added.iter().map(|t| t.uid.as_str()).collect();
        let mut updated: Vec<PlayerTrack> = std::mem::take(&mut self.updated)
            .into_iter()
            .filter(|u| !added_set.contains(u.as_str()))
            .filter_map(|u| self.entries.get(&u).cloned())
            .collect();
        updated.sort_by(|a, b| a.uid.cmp(&b.uid));
        QueueChanged {
            rev: self.rev,
            order: self.order.clone(),
            original: self.original.clone(),
            index: self.index_i64(),
            shuffle: self.shuffle(),
            repeat: self.repeat,
            continuation,
            added,
            updated,
        }
    }

    fn new_entry(&mut self, t: TrackInput) -> String {
        self.next_uid += 1;
        let uid = format!("q{}", base36(self.next_uid));
        self.entries
            .insert(uid.clone(), PlayerTrack::from_input(uid.clone(), t));
        self.added.push(uid.clone());
        uid
    }

    fn gc(&mut self) {
        let live: HashSet<&String> = self.order.iter().collect();
        self.entries.retain(|u, _| live.contains(u));
    }

    // ---- mutations ----------------------------------------------------------

    /// `setQueue`: replaces everything, shuffle off, repeat kept.
    pub fn set(&mut self, tracks: Vec<TrackInput>, start: usize) {
        self.order.clear();
        self.original = None;
        self.entries.clear();
        self.added.clear();
        self.updated.clear();
        for t in tracks {
            let uid = self.new_entry(t);
            self.order.push(uid);
        }
        self.index = if self.order.is_empty() {
            None
        } else {
            Some(start.min(self.order.len() - 1))
        };
    }

    /// `addToQueue`. Returns the uids added.
    pub fn enqueue_end(&mut self, tracks: Vec<TrackInput>) -> Vec<String> {
        let uids: Vec<String> = tracks.into_iter().map(|t| self.new_entry(t)).collect();
        self.order.extend(uids.iter().cloned());
        if let Some(o) = &mut self.original {
            o.extend(uids.iter().cloned());
        }
        uids
    }

    /// `addNextToQueue`: right after the current entry (in both orders).
    pub fn enqueue_next(&mut self, tracks: Vec<TrackInput>) -> Vec<String> {
        let uids: Vec<String> = tracks.into_iter().map(|t| self.new_entry(t)).collect();
        let at = self.index.map_or(0, |i| i + 1);
        let cur = self.current_uid().map(str::to_owned);
        self.order.splice(at..at, uids.iter().cloned());
        if let Some(o) = &mut self.original {
            match cur.and_then(|c| o.iter().position(|u| *u == c)) {
                Some(p) => {
                    o.splice(p + 1..p + 1, uids.iter().cloned());
                }
                None => o.extend(uids.iter().cloned()),
            }
        }
        uids
    }

    /// `removeFromQueue`. `None` for an unknown uid.
    pub fn remove(&mut self, uid: &str) -> Option<Removed> {
        let p = self.position_of(uid)?;
        let was_current = self.index == Some(p);
        if let Some(i) = self.index
            && p < i
        {
            self.index = Some(i - 1);
        }
        self.order.remove(p);
        if let Some(o) = &mut self.original {
            o.retain(|u| u != uid);
        }
        self.entries.remove(uid);
        self.updated.remove(uid);
        if !was_current {
            return Some(Removed::Other);
        }
        let i = self.index.expect("was current");
        if i < self.order.len() {
            Some(Removed::CurrentReplaced)
        } else {
            self.index = None;
            Some(Removed::CurrentGone)
        }
    }

    /// Removes the given entries after the current one; returns their ids.
    pub fn remove_upcoming(&mut self, pred: impl Fn(&PlayerTrack) -> bool) -> Vec<String> {
        let start = self.index.map_or(0, |i| i + 1);
        let doomed: Vec<String> = self.order[start.min(self.order.len())..]
            .iter()
            .filter(|u| self.entries.get(*u).is_some_and(&pred))
            .cloned()
            .collect();
        let mut ids = Vec::new();
        for u in doomed {
            if let Some(t) = self.entries.get(&u) {
                ids.push(t.id.clone());
            }
            self.remove(&u);
        }
        ids
    }

    /// `moveInQueue`: `to` is the target index in play order.
    pub fn move_to(&mut self, uid: &str, to: usize) -> Option<()> {
        let from = self.position_of(uid)?;
        if to >= self.order.len() {
            return None;
        }
        let u = self.order.remove(from);
        self.order.insert(to, u);
        if let Some(i) = self.index {
            self.index = Some(if i == from {
                to
            } else if from < i && to >= i {
                i - 1
            } else if from > i && to <= i {
                i + 1
            } else {
                i
            });
        }
        Some(())
    }

    /// `clearQueue` (keep the current entry) or `wipeQueue`.
    pub fn clear(&mut self, keep_current: bool) {
        let cur = if keep_current {
            self.current_uid().map(str::to_owned)
        } else {
            None
        };
        match cur {
            Some(c) => {
                self.order = vec![c.clone()];
                if self.original.is_some() {
                    self.original = Some(vec![c]);
                }
                self.index = Some(0);
            }
            None => {
                self.order.clear();
                if self.original.is_some() {
                    self.original = Some(Vec::new());
                }
                self.index = None;
            }
        }
        self.gc();
    }

    /// `toggleShuffle` on/off. On: current first, Fisher–Yates the rest.
    /// Off: back to the original order, current entry kept.
    pub fn set_shuffle(&mut self, on: bool, rng: &mut fastrand::Rng) {
        if on == self.shuffle() {
            return;
        }
        if on {
            self.original = Some(self.order.clone());
            let cur = self.index.map(|i| self.order.remove(i));
            let rest = &mut self.order;
            for i in (1..rest.len()).rev() {
                let j = rng.usize(0..=i);
                rest.swap(i, j);
            }
            match cur {
                Some(c) => {
                    self.order.insert(0, c);
                    self.index = Some(0);
                }
                None => self.index = None,
            }
        } else {
            let cur = self.current_uid().map(str::to_owned);
            self.order = self.original.take().unwrap_or_default();
            self.index = cur.and_then(|c| self.position_of(&c));
        }
    }

    pub fn set_index(&mut self, i: Option<usize>) {
        self.index = i.filter(|i| *i < self.order.len());
    }

    pub fn mark_unavailable(&mut self, uid: &str) {
        if let Some(t) = self.entries.get_mut(uid)
            && !t.unavailable
        {
            t.unavailable = true;
            self.updated.insert(uid.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContinuationKind, Origin};

    fn input(id: &str) -> TrackInput {
        TrackInput {
            id: format!("{id:_<11}"),
            title: id.into(),
            ..Default::default()
        }
    }

    fn titles(q: &Queue) -> Vec<String> {
        q.items().into_iter().map(|t| t.title).collect()
    }

    fn cont() -> Continuation {
        Continuation {
            kind: ContinuationKind::None,
            fetching: false,
        }
    }

    fn abcde() -> Queue {
        let mut q = Queue::new();
        q.set(["a", "b", "c", "d", "e"].map(input).to_vec(), 1);
        q.commit(cont());
        q
    }

    #[test]
    fn set_and_commit() {
        let mut q = Queue::new();
        q.set(vec![input("a"), input("b")], 5);
        assert_eq!(q.index(), Some(1));
        let c = q.commit(cont());
        assert_eq!(c.rev, 1);
        assert_eq!(c.added.len(), 2);
        assert_eq!(c.order, vec!["q1", "q2"]);
        let c = q.commit(cont());
        assert_eq!(c.rev, 2);
        assert!(c.added.is_empty());
        q.set(vec![], 0);
        assert_eq!(q.index(), None);
    }

    #[test]
    fn uids_are_unique_and_base36() {
        let mut q = Queue::new();
        q.set((0..40).map(|i| input(&i.to_string())).collect(), 0);
        let order = q.order().to_vec();
        assert_eq!(order[34], "qz");
        assert_eq!(order[35], "q10");
        q.set(vec![input("x")], 0);
        assert_eq!(q.order(), ["q15"]);
    }

    #[test]
    fn enqueue_end_and_next() {
        let mut q = abcde();
        q.enqueue_next(vec![input("n1"), input("n2")]);
        assert_eq!(titles(&q), ["a", "b", "n1", "n2", "c", "d", "e"]);
        q.enqueue_end(vec![input("z")]);
        assert_eq!(titles(&q).last().unwrap(), "z");
        let c = q.commit(cont());
        assert_eq!(c.added.len(), 3);
        // Nothing current: "next" goes first.
        let mut q = Queue::new();
        q.enqueue_next(vec![input("a")]);
        q.enqueue_next(vec![input("b")]);
        assert_eq!(titles(&q), ["b", "a"]);
    }

    #[test]
    fn remove_keeps_index_on_the_current_entry() {
        let mut q = abcde(); // current b
        let a = q.order()[0].clone();
        assert_eq!(q.remove(&a), Some(Removed::Other));
        assert_eq!(q.current().unwrap().title, "b");
        let b = q.current_uid().unwrap().to_owned();
        assert_eq!(q.remove(&b), Some(Removed::CurrentReplaced));
        assert_eq!(q.current().unwrap().title, "c");
        q.set_index(Some(2)); // e
        let e = q.current_uid().unwrap().to_owned();
        assert_eq!(q.remove(&e), Some(Removed::CurrentGone));
        assert_eq!(q.index(), None);
        assert_eq!(q.remove("nope"), None);
        assert_eq!(q.items().len(), 2);
    }

    #[test]
    fn move_adjusts_index_like_move_in_queue() {
        let mut q = abcde(); // current b at 1
        let d = q.order()[3].clone();
        q.move_to(&d, 0).unwrap();
        assert_eq!(titles(&q), ["d", "a", "b", "c", "e"]);
        assert_eq!(q.index(), Some(2));
        let d = q.order()[0].clone();
        q.move_to(&d, 4).unwrap();
        assert_eq!(q.index(), Some(1));
        let b = q.current_uid().unwrap().to_owned();
        q.move_to(&b, 3).unwrap();
        assert_eq!(q.index(), Some(3));
        assert!(q.move_to(&b, 9).is_none());
    }

    #[test]
    fn clear_keeps_or_wipes() {
        let mut q = abcde();
        q.clear(true);
        assert_eq!(titles(&q), ["b"]);
        assert_eq!(q.index(), Some(0));
        q.clear(false);
        assert!(q.is_empty());
        assert_eq!(q.index(), None);
    }

    #[test]
    fn shuffle_puts_current_first_and_restores() {
        let mut rng = fastrand::Rng::with_seed(7);
        let mut q = abcde();
        let before = q.order().to_vec();
        q.set_shuffle(true, &mut rng);
        assert!(q.shuffle());
        assert_eq!(q.index(), Some(0));
        assert_eq!(q.current().unwrap().title, "b");
        let mut sorted = q.order().to_vec();
        sorted.sort();
        let mut b2 = before.clone();
        b2.sort();
        assert_eq!(sorted, b2);
        // Moving in shuffle changes only the play order.
        q.set_index(Some(2));
        let cur = q.current_uid().unwrap().to_owned();
        q.set_shuffle(false, &mut rng);
        assert_eq!(q.order(), before.as_slice());
        assert_eq!(q.current_uid(), Some(cur.as_str()));
    }

    #[test]
    fn shuffle_keeps_both_orders_in_sync() {
        let mut rng = fastrand::Rng::with_seed(1);
        let mut q = abcde(); // current b
        q.set_shuffle(true, &mut rng);
        q.enqueue_next(vec![input("n")]);
        assert_eq!(q.at(1).unwrap().title, "n");
        q.enqueue_end(vec![input("z")]);
        let c_uid = q.items().into_iter().find(|t| t.title == "c").unwrap().uid;
        q.remove(&c_uid);
        q.set_shuffle(false, &mut rng);
        // Original order with "n" right after the current entry "b".
        assert_eq!(titles(&q), ["a", "b", "n", "d", "e", "z"]);
        assert_eq!(q.current().unwrap().title, "b");
    }

    #[test]
    fn shuffle_without_current() {
        let mut rng = fastrand::Rng::with_seed(3);
        let mut q = Queue::new();
        q.enqueue_end(vec![input("a"), input("b")]);
        q.set_shuffle(true, &mut rng);
        assert_eq!(q.index(), None);
        assert_eq!(q.len(), 2);
        q.clear(true);
        assert!(q.is_empty());
        assert!(q.shuffle());
    }

    #[test]
    fn candidates_skip_unplayable_and_wrap() {
        let mut q = abcde(); // current b
        let c = q.order()[2].clone();
        q.mark_unavailable(&c);
        let ok = |t: &PlayerTrack| !t.unavailable;
        assert_eq!(q.next_candidate(ok), Some(3));
        assert_eq!(q.prev_candidate(ok), Some(0));
        q.set_index(Some(4));
        assert_eq!(q.next_candidate(ok), None);
        q.repeat = Repeat::All;
        assert_eq!(q.next_candidate(ok), Some(0));
        // Only the current one is playable: repeat all replays it.
        let only_e = |t: &PlayerTrack| t.title == "e";
        assert_eq!(q.next_candidate(only_e), Some(4));
        q.set_index(Some(0));
        assert_eq!(q.prev_candidate(ok), None);
        let ch = q.commit(cont());
        assert_eq!(ch.updated.len(), 1);
        assert!(ch.updated[0].unavailable);
        assert_eq!(q.remaining_after(), 4);
    }

    #[test]
    fn remove_upcoming_by_origin() {
        let mut q = Queue::new();
        let w = |id: &str| TrackInput {
            origin: Some(Origin::Wave),
            ..input(id)
        };
        q.set(vec![w("a"), w("b"), input("u"), w("c")], 0);
        let ids = q.remove_upcoming(|t| t.origin == Origin::Wave);
        assert_eq!(ids.len(), 2);
        assert_eq!(titles(&q), ["a", "u"]);
        assert_eq!(q.index(), Some(0));
    }

    #[test]
    fn state_round_trip() {
        let mut rng = fastrand::Rng::with_seed(9);
        let mut q = abcde();
        q.set_shuffle(true, &mut rng);
        q.repeat = Repeat::One;
        let s = q.state();
        let json = serde_json::to_string(&s).unwrap();
        let r = Queue::from_state(serde_json::from_str(&json).unwrap());
        assert_eq!(r.order(), q.order());
        assert_eq!(r.index(), q.index());
        assert!(r.shuffle());
        assert_eq!(r.repeat, Repeat::One);
        let mut r = r;
        r.enqueue_end(vec![input("x")]);
        assert_eq!(r.order().last().unwrap(), "q6");
    }
}
