//! Wave settings, modelled on Yandex Music's "My Wave" knobs.

use serde::{Deserialize, Serialize};

/// "Характер": what kind of tracks the wave leans to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Character {
    /// No preference: the default mix of familiar and new.
    #[default]
    Default,
    /// "Любимое": artists and tracks the listener already likes.
    Favorite,
    /// "Незнакомое": artists the listener has not heard yet.
    Unfamiliar,
    /// "Популярное": tracks with many plays.
    Popular,
}

/// "Настроение". Each maps to one "Moods & moments" button of YouTube Music.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mood {
    /// "Бодрое"
    Energetic,
    /// "Весёлое"
    Happy,
    /// "Спокойное"
    Calm,
    /// "Грустное"
    Sad,
}

impl Mood {
    /// Title of the matching YouTube Music mood (interface language `en`).
    pub fn ytm_title(self) -> &'static str {
        match self {
            Mood::Energetic => "Energize",
            Mood::Happy => "Feel good",
            Mood::Calm => "Chill",
            Mood::Sad => "Sad",
        }
    }
}

/// "Занятие": a YouTube Music mood or genre button (Workout, Focus, …).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    /// YouTube Music title, also the stable key ("Workout").
    pub title: String,
    /// `params` for `mood_playlists`.
    pub params: String,
}

/// "Язык" of the vocals, guessed from the title and artist names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Any,
    Russian,
    Uzbek,
    English,
}

/// Everything the listener can tweak in the settings popover.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct WaveSettings {
    pub character: Character,
    pub mood: Option<Mood>,
    pub activity: Option<Activity>,
    pub language: Language,
}

impl WaveSettings {
    /// A mood or activity is picked, so mood playlists join the candidates.
    pub fn wants_mood(&self) -> bool {
        self.mood.is_some() || self.activity.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_missing_fields() {
        let s = WaveSettings {
            character: Character::Unfamiliar,
            mood: Some(Mood::Calm),
            activity: Some(Activity {
                title: "Workout".into(),
                params: "abc".into(),
            }),
            language: Language::Uzbek,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<WaveSettings>(&json).unwrap(), s);
        let partial: WaveSettings = serde_json::from_str(r#"{"mood":"sad"}"#).unwrap();
        assert_eq!(partial.mood, Some(Mood::Sad));
        assert_eq!(partial.character, Character::Default);
        assert!(partial.wants_mood());
    }
}
