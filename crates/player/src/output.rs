//! Where the engine's audio goes. The engine (queue, transport, tracker,
//! continuation) only talks to an [`Output`] and its decks; GStreamer
//! (`playbin3` decks, feature `gstreamer`) is one output, a platform player
//! (ExoPlayer / AVPlayer, driven by `volna-core`) is another.
//!
//! A deck lives for one continuous playback: the engine makes a new one per
//! explicit load and tells their reports apart by [`DeckId`].

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use crate::analysis::Tap;
use crate::model::DspConfig;

pub type DeckId = u64;

/// Shape of a fade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Curve {
    Linear,
    /// sin(πt/2): the incoming side of an equal-power crossfade.
    EqualPowerIn,
    /// cos(πt/2): the outgoing side.
    EqualPowerOut,
}

impl Curve {
    pub fn shape(self, t: f64) -> f64 {
        use std::f64::consts::FRAC_PI_2;
        match self {
            Curve::Linear => t,
            Curve::EqualPowerIn => (FRAC_PI_2 * t).sin(),
            Curve::EqualPowerOut => 1.0 - (FRAC_PI_2 * t).cos(),
        }
    }
}

/// The two states the engine asks a deck for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckState {
    Paused,
    Playing,
}

/// What a deck reports back to the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum DeckEvent {
    /// Ready to play (GStreamer: `AsyncDone`); may repeat after seeks.
    Prerolled,
    /// The deck reached this state.
    State(DeckState),
    /// A stream began (the second one on a deck is a gapless switch).
    StreamStart,
    /// The end of the stream.
    Eos,
    Error {
        message: String,
        /// The data could not be decoded (rather than fetched).
        decode: bool,
    },
    /// Buffer fill, 0–100.
    Buffering(i32),
    DurationChanged,
    /// The gapless successor was handed to the deck.
    AboutToFinish,
}

/// A deck report, tagged with the deck it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct DeckMsg {
    pub deck: DeckId,
    pub event: DeckEvent,
}

pub type DeckSender = UnboundedSender<DeckMsg>;

/// What `about-to-finish` switches to (gapless).
#[derive(Debug, Clone)]
pub struct NextUp {
    pub uid: String,
    pub uri: String,
}

/// One playback of one URI.
pub trait OutputDeck: Send + 'static {
    /// Starts loading, paused. `start` is where the engine wants playback
    /// to begin; returns true when the deck starts there itself, false when
    /// the engine should seek once [`DeckEvent::Prerolled`] arrives.
    fn preroll(&mut self, start: Option<f64>) -> bool {
        let _ = start;
        self.set_state(DeckState::Paused);
        false
    }
    fn set_state(&self, state: DeckState);
    fn is_playing(&self) -> bool;
    /// Seconds, when known.
    fn position(&self) -> Option<f64>;
    fn duration(&self) -> Option<f64>;
    /// Flushing, accurate seek; also how the rate changes.
    fn seek(&self, position: f64, rate: f64) -> bool;
    /// Ramps this deck's gain to `to` over `seconds`, from `from` or the current gain.
    fn fade(&self, from: Option<f64>, to: f64, seconds: f64, curve: Curve);
    fn set_next_up(&self, next: Option<NextUp>) {
        let _ = next;
    }
    /// The uid `about-to-finish` switched to, if a gapless switch is pending.
    fn take_switched(&self) -> Option<String> {
        None
    }
    /// Feeds the visualizer with what this deck plays.
    fn set_tap(&self, tap: Option<Arc<Tap>>) {
        let _ = tap;
    }
}

/// Makes decks and holds what they share (EQ, master volume).
pub trait Output: Send + Sync + 'static {
    type Deck: OutputDeck;

    /// A deck for `uri`, reporting to `tx` as `id`.
    fn new_deck(
        &self,
        id: DeckId,
        uri: &str,
        preserve_pitch: bool,
        tx: DeckSender,
    ) -> Result<Self::Deck, String>;
    fn set_dsp(&self, config: &DspConfig) {
        let _ = config;
    }
    /// Master gain: curve(volume) × (muted ? 0 : 1).
    fn set_master(&self, gain: f64);
    /// Only one deck can sound at a time: no crossfade, no gapless handoff.
    fn single_deck(&self) -> bool {
        false
    }
}
