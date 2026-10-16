//! "My Wave" recommendation engine.
//!
//! Stores listening signals in SQLite (completed plays, early/late skips,
//! likes, dislikes — all time-decayed), gathers candidates from YTM radio,
//! related tracks, similar artists and moods & genres, and ranks them by
//! artist affinity, novelty, repeat penalties and ε-exploration. Supports
//! the wave settings (character, mood, activity, language) and cold start
//! from a few chosen artists.
//!
//! Layout:
//! - [`signal`]: what a play/skip/like is worth and how it fades;
//! - [`store`]: the SQLite log; [`profile`]: taste folded from it;
//! - [`scoring`]: pure ranking and picking; [`lang`]: language guess;
//! - [`source`]: candidate generators ([`YtmSource`] for real use);
//! - [`engine`]: the session ([`Wave`]) tying it together.

pub mod engine;
pub mod error;
pub mod lang;
pub mod profile;
pub mod scoring;
pub mod settings;
pub mod signal;
pub mod source;
pub mod store;

pub use engine::{Clock, PlayerEvent, RecordOutcome, Wave, WaveStatus};
pub use error::{Error, Result};
pub use settings::{Activity, Character, Language, Mood, WaveSettings};
pub use signal::PlayOutcome;
pub use source::{ArtistBrief, ArtistInfo, CandidateSource, MoodQuery, YtmSource};
pub use store::{SeedArtist, Store};
