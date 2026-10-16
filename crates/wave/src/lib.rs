//! "My Wave" recommendation engine.
//!
//! Stores listening signals in SQLite (completed plays, early/late skips,
//! likes, dislikes — all time-decayed), gathers candidates from YTM radio,
//! related tracks, similar artists and moods & genres, and ranks them by
//! artist affinity, novelty, repeat penalties and ε-exploration. Supports
//! the wave settings (character, mood, activity, language) and cold start
//! from a few chosen artists.
