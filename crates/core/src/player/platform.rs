//! The engine on a platform player (Media3/ExoPlayer, AVPlayer): the core
//! keeps the queue, the wave and every decision, and only tells the
//! platform what to load and when to play; the platform reports back through
//! [`PlatformOutput::report`] (`volna_core_player_report` over FFI).
//!
//! One platform player sounds at a time, so the output is single-deck: no
//! crossfade and no gapless handoff. Each engine deck takes the player over
//! when it starts loading; a deck that lost it (a retired one) is ignored.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use volna_player::{
    Curve, DeckEvent, DeckId, DeckMsg, DeckSender, DeckState, Output, OutputDeck,
};

/// What the core asks of the platform player. Calls come from core
/// threads and must not block; positions are milliseconds.
pub trait PlatformPlayer: Send + Sync + 'static {
    /// Prepares `url` paused at `start_ms`; reply with
    /// [`PlayerReport::Ready`] once it can play (or an error).
    fn load(&self, url: &str, start_ms: u64);
    fn play(&self);
    fn pause(&self);
    fn seek(&self, position_ms: u64);
    /// Unloads; nothing more is reported for the stopped item.
    fn stop(&self);
    /// 0.0–1.0, the app's volume times its fades.
    fn set_volume(&self, volume: f64);
}

/// A platform player command as JSON (for hosts that take one callback,
/// like the FFI): `{"op":"load","url":…,"startMs":…}`, `{"op":"play"}`,
/// `{"op":"pause"}`, `{"op":"seek","positionMs":…}`, `{"op":"stop"}`,
/// `{"op":"setVolume","volume":…}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum PlayerCommand {
    #[serde(rename_all = "camelCase")]
    Load { url: String, start_ms: u64 },
    Play,
    Pause,
    #[serde(rename_all = "camelCase")]
    Seek { position_ms: u64 },
    Stop,
    SetVolume { volume: f64 },
}

/// [`PlatformPlayer`] as a stream of [`PlayerCommand`]s.
pub struct CommandPlayer<F>(pub F);

impl<F: Fn(PlayerCommand) + Send + Sync + 'static> PlatformPlayer for CommandPlayer<F> {
    fn load(&self, url: &str, start_ms: u64) {
        (self.0)(PlayerCommand::Load {
            url: url.to_owned(),
            start_ms,
        });
    }
    fn play(&self) {
        (self.0)(PlayerCommand::Play);
    }
    fn pause(&self) {
        (self.0)(PlayerCommand::Pause);
    }
    fn seek(&self, position_ms: u64) {
        (self.0)(PlayerCommand::Seek { position_ms });
    }
    fn stop(&self) {
        (self.0)(PlayerCommand::Stop);
    }
    fn set_volume(&self, volume: f64) {
        (self.0)(PlayerCommand::SetVolume { volume });
    }
}

/// What the platform reports, as JSON: `{"type":"ready","durationMs":…}`,
/// `{"type":"playing"}`, `{"type":"paused"}`,
/// `{"type":"position","positionMs":…,"durationMs":…}`,
/// `{"type":"buffering","on":true}`, `{"type":"ended"}`,
/// `{"type":"error","message":…,"decode":false}`.
/// Reports are about the item of the latest `load`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PlayerReport {
    /// Loaded and ready to play from the requested start.
    #[serde(rename_all = "camelCase")]
    Ready {
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    /// Actually playing (after `play`, or when buffering ended).
    Playing,
    /// Actually paused.
    Paused,
    /// The playhead; send it every few hundred ms while playing.
    #[serde(rename_all = "camelCase")]
    Position {
        position_ms: u64,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    Buffering {
        on: bool,
    },
    /// The item played to its end.
    Ended,
    Error {
        message: String,
        /// The data was fetched but could not be decoded.
        #[serde(default)]
        decode: bool,
    },
}

struct Bridge {
    player: Mutex<Option<Arc<dyn PlatformPlayer>>>,
    inner: Mutex<Inner>,
}

struct Inner {
    /// The deck the platform player belongs to.
    owner: Option<DeckId>,
    tx: Option<DeckSender>,
    master: f64,
    fade: f64,
    playing: bool,
    position: f64,
    at: Instant,
    duration: Option<f64>,
}

impl Bridge {
    fn player(&self) -> Option<Arc<dyn PlatformPlayer>> {
        self.player.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Applies `update` and hands back the platform player if `deck` owns
    /// it. The player is called by the caller after the lock is released,
    /// so a platform that reports from inside a command cannot deadlock.
    fn as_owner<R>(
        &self,
        deck: DeckId,
        update: impl FnOnce(&mut Inner) -> R,
    ) -> Option<(Arc<dyn PlatformPlayer>, R)> {
        let r = {
            let mut inner = self.inner();
            if inner.owner != Some(deck) {
                return None;
            }
            update(&mut inner)
        };
        self.player().map(|p| (p, r))
    }
}

/// The [`Output`] for a [`PlatformPlayer`]. Cheap to clone; clones share
/// the player.
#[derive(Clone)]
pub struct PlatformOutput {
    bridge: Arc<Bridge>,
}

impl Default for PlatformOutput {
    fn default() -> Self {
        Self::new()
    }
}

fn ms(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

impl PlatformOutput {
    /// No platform player yet: commands are dropped until [`Self::set_player`].
    pub fn new() -> Self {
        Self {
            bridge: Arc::new(Bridge {
                player: Mutex::new(None),
                inner: Mutex::new(Inner {
                    owner: None,
                    tx: None,
                    master: 1.0,
                    fade: 1.0,
                    playing: false,
                    position: 0.0,
                    at: Instant::now(),
                    duration: None,
                }),
            }),
        }
    }

    pub fn with_player(player: Arc<dyn PlatformPlayer>) -> Self {
        let out = Self::new();
        out.set_player(Some(player));
        out
    }

    pub fn set_player(&self, player: Option<Arc<dyn PlatformPlayer>>) {
        *self.bridge.player.lock().unwrap_or_else(|e| e.into_inner()) = player;
    }

    /// A report from the platform player, for the current deck.
    pub fn report(&self, report: PlayerReport) {
        let mut inner = self.bridge.inner();
        let (Some(deck), Some(tx)) = (inner.owner, inner.tx.clone()) else {
            return;
        };
        let mut events = Vec::new();
        let set_duration = |inner: &mut Inner, d: Option<u64>, events: &mut Vec<DeckEvent>| {
            let d = d.filter(|d| *d > 0).map(|d| d as f64 / 1000.0);
            if d.is_some() && d != inner.duration {
                inner.duration = d;
                events.push(DeckEvent::DurationChanged);
            }
        };
        match report {
            PlayerReport::Ready { duration_ms } => {
                let d = duration_ms.filter(|d| *d > 0).map(|d| d as f64 / 1000.0);
                if d.is_some() {
                    inner.duration = d;
                }
                events.push(DeckEvent::Prerolled);
            }
            PlayerReport::Playing => {
                inner.playing = true;
                inner.at = Instant::now();
                events.push(DeckEvent::State(DeckState::Playing));
            }
            PlayerReport::Paused => {
                inner.position = position_now(&inner);
                inner.playing = false;
                inner.at = Instant::now();
                events.push(DeckEvent::State(DeckState::Paused));
            }
            PlayerReport::Position {
                position_ms,
                duration_ms,
            } => {
                inner.position = position_ms as f64 / 1000.0;
                inner.at = Instant::now();
                set_duration(&mut inner, duration_ms, &mut events);
            }
            PlayerReport::Buffering { on } => {
                events.push(DeckEvent::Buffering(if on { 0 } else { 100 }));
            }
            PlayerReport::Ended => {
                inner.playing = false;
                events.push(DeckEvent::Eos);
            }
            PlayerReport::Error { message, decode } => {
                inner.playing = false;
                events.push(DeckEvent::Error { message, decode });
            }
        }
        drop(inner);
        for event in events {
            let _ = tx.send(DeckMsg { deck, event });
        }
    }
}

fn position_now(inner: &Inner) -> f64 {
    let p = if inner.playing {
        inner.position + inner.at.elapsed().as_secs_f64()
    } else {
        inner.position
    };
    inner.duration.map_or(p, |d| p.min(d))
}

impl Output for PlatformOutput {
    type Deck = PlatformDeck;

    fn new_deck(
        &self,
        id: DeckId,
        uri: &str,
        _preserve_pitch: bool,
        tx: DeckSender,
    ) -> Result<PlatformDeck, String> {
        Ok(PlatformDeck {
            id,
            uri: uri.to_owned(),
            tx: Some(tx),
            bridge: self.bridge.clone(),
        })
    }

    fn set_master(&self, gain: f64) {
        let volume = {
            let mut inner = self.bridge.inner();
            inner.master = gain;
            inner.owner.map(|_| (inner.fade * gain).clamp(0.0, 1.0))
        };
        if let (Some(v), Some(p)) = (volume, self.bridge.player()) {
            p.set_volume(v);
        }
    }

    fn single_deck(&self) -> bool {
        true
    }
}

pub struct PlatformDeck {
    id: DeckId,
    uri: String,
    /// Handed to the bridge when this deck takes the player over.
    tx: Option<DeckSender>,
    bridge: Arc<Bridge>,
}

impl OutputDeck for PlatformDeck {
    fn preroll(&mut self, start: Option<f64>) -> bool {
        let start = start.unwrap_or(0.0).max(0.0);
        let volume = {
            let mut inner = self.bridge.inner();
            inner.owner = Some(self.id);
            inner.tx = self.tx.take();
            inner.fade = 1.0;
            inner.playing = false;
            inner.position = start;
            inner.at = Instant::now();
            inner.duration = None;
            inner.master.clamp(0.0, 1.0)
        };
        if let Some(p) = self.bridge.player() {
            p.set_volume(volume);
            p.load(&self.uri, ms(start));
        }
        true
    }

    fn set_state(&self, state: DeckState) {
        if let Some((p, ())) = self.bridge.as_owner(self.id, |_| ()) {
            match state {
                DeckState::Playing => p.play(),
                DeckState::Paused => p.pause(),
            }
        }
    }

    fn is_playing(&self) -> bool {
        let inner = self.bridge.inner();
        inner.owner == Some(self.id) && inner.playing
    }

    fn position(&self) -> Option<f64> {
        let inner = self.bridge.inner();
        (inner.owner == Some(self.id)).then(|| position_now(&inner))
    }

    fn duration(&self) -> Option<f64> {
        let inner = self.bridge.inner();
        inner.duration.filter(|_| inner.owner == Some(self.id))
    }

    fn seek(&self, position: f64, _rate: f64) -> bool {
        let owned = self.bridge.as_owner(self.id, |inner| {
            inner.position = position.max(0.0);
            inner.at = Instant::now();
        });
        match owned {
            Some((p, ())) => {
                p.seek(ms(position));
                true
            }
            None => false,
        }
    }

    /// No sample-accurate ramps here: the gain jumps to the target (the
    /// de-click before a pause, the sleep fade).
    fn fade(&self, _from: Option<f64>, to: f64, _seconds: f64, _curve: Curve) {
        if let Some((p, volume)) = self.bridge.as_owner(self.id, |inner| {
            inner.fade = to;
            (to * inner.master).clamp(0.0, 1.0)
        }) {
            p.set_volume(volume);
        }
    }
}

impl Drop for PlatformDeck {
    fn drop(&mut self) {
        let owned = self.bridge.as_owner(self.id, |inner| {
            inner.owner = None;
            inner.tx = None;
            inner.playing = false;
        });
        if let Some((p, ())) = owned {
            p.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_shapes() {
        let r: PlayerReport =
            serde_json::from_value(json!({"type":"ready","durationMs":1000})).unwrap();
        assert_eq!(
            r,
            PlayerReport::Ready {
                duration_ms: Some(1000)
            }
        );
        let r: PlayerReport =
            serde_json::from_value(json!({"type":"error","message":"x"})).unwrap();
        assert_eq!(
            r,
            PlayerReport::Error {
                message: "x".into(),
                decode: false
            }
        );
        let c = PlayerCommand::Load {
            url: "http://127.0.0.1:1/a".into(),
            start_ms: 5,
        };
        assert_eq!(
            serde_json::to_value(c).unwrap(),
            json!({"op":"load","url":"http://127.0.0.1:1/a","startMs":5})
        );
        assert_eq!(
            serde_json::to_value(PlayerCommand::SetVolume { volume: 0.5 }).unwrap(),
            json!({"op":"setVolume","volume":0.5})
        );
    }

    #[test]
    fn only_the_newest_deck_drives_the_player() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        let out = PlatformOutput::with_player(Arc::new(CommandPlayer(move |c| {
            s.lock().unwrap().push(c)
        })));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut a = out.new_deck(1, "u1", false, tx.clone()).unwrap();
        assert!(a.preroll(Some(2.0)));
        let mut b = out.new_deck(2, "u2", false, tx).unwrap();
        b.preroll(None);
        // The retired deck can no longer touch the player.
        a.set_state(DeckState::Playing);
        drop(a);
        b.set_state(DeckState::Playing);
        out.report(PlayerReport::Ended);
        assert_eq!(
            rx.try_recv().unwrap(),
            DeckMsg {
                deck: 2,
                event: DeckEvent::Eos
            }
        );
        let seen = seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .filter(|c| !matches!(c, PlayerCommand::SetVolume { .. }))
                .cloned()
                .collect::<Vec<_>>(),
            [
                PlayerCommand::Load {
                    url: "u1".into(),
                    start_ms: 2000
                },
                PlayerCommand::Load {
                    url: "u2".into(),
                    start_ms: 0
                },
                PlayerCommand::Play,
            ]
        );
    }
}
