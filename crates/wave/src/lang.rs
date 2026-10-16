//! Guessing the language of a track from its title and artist names.
//!
//! YouTube Music does not say what language a song is in, so this looks at
//! the script and a few Uzbek giveaways. It only has to be right often
//! enough to steer the wave; a wrong guess costs a score penalty, not a ban.

use crate::settings::Language;

/// Letters that only Uzbek Cyrillic uses (Russian has none of them).
const UZ_CYRILLIC: &[char] = &['ў', 'қ', 'ғ', 'ҳ', 'Ў', 'Қ', 'Ғ', 'Ҳ'];

/// Apostrophes that follow `o` / `g` in Uzbek Latin (o‘zbek, g‘alaba).
const UZ_APOSTROPHES: &[char] = &['\'', '‘', '’', 'ʻ', 'ʼ', '`'];

/// Short, common Uzbek words that rarely appear in English or Russian titles.
const UZ_WORDS: &[&str] = &[
    "va",
    "men",
    "sen",
    "seni",
    "meni",
    "yor",
    "yorim",
    "jonim",
    "sevgi",
    "sevaman",
    "qiz",
    "qizlar",
    "bola",
    "dunyo",
    "yurak",
    "yuragim",
    "onam",
    "ona",
    "dada",
    "kel",
    "ketma",
    "nega",
    "qani",
    "bilan",
    "uchun",
    "emas",
    "bor",
    "yoq",
    "go'zal",
    "chiroyli",
    "baxt",
    "baxtim",
    "muhabbat",
    "ishq",
    "oy",
    "kun",
    "tun",
    "yomg'ir",
    "do'st",
    "xayr",
    "salom",
    "qo'shiq",
    "aytaman",
    "o'zbek",
    "o'zbekiston",
    "toshkent",
    "farg'ona",
];

/// Best guess of the vocal language. `Language::Any` means "can't tell"
/// (another script, or nothing to go on).
pub fn detect(title: &str, artists: &[&str]) -> Language {
    let mut text = String::from(title);
    for a in artists {
        text.push(' ');
        text.push_str(a);
    }

    let mut cyrillic = 0usize;
    let mut latin = 0usize;
    let mut other = 0usize;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        if UZ_CYRILLIC.contains(&c) {
            return Language::Uzbek;
        }
        match c {
            'а'..='я' | 'А'..='Я' | 'ё' | 'Ё' => cyrillic += 1,
            c if c.is_ascii_alphabetic() => latin += 1,
            _ => other += 1,
        }
    }

    if other > cyrillic + latin {
        return Language::Any;
    }
    if cyrillic > latin {
        return Language::Russian;
    }
    if latin == 0 {
        return Language::Any;
    }
    if looks_uzbek_latin(&text) {
        Language::Uzbek
    } else {
        Language::English
    }
}

fn looks_uzbek_latin(text: &str) -> bool {
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    // o‘ / g‘ digraphs.
    if chars
        .windows(2)
        .any(|w| (w[0] == 'o' || w[0] == 'g') && UZ_APOSTROPHES.contains(&w[1]))
    {
        return true;
    }
    lower
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .any(|w| UZ_WORDS.contains(&w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_and_uzbek_markers() {
        assert_eq!(detect("Привычка", &["Скриптонит"]), Language::Russian);
        assert_eq!(
            detect("Blinding Lights", &["The Weeknd"]),
            Language::English
        );
        assert_eq!(detect("Yuragim", &["Shahzoda"]), Language::Uzbek);
        assert_eq!(
            detect("Sevgi", &["Jaloliddin Ahmadaliyev"]),
            Language::Uzbek
        );
        assert_eq!(detect("O‘zbegim", &["Yulduz Usmonova"]), Language::Uzbek);
        assert_eq!(detect("Қизғалдоқ", &[]), Language::Uzbek);
        assert_eq!(detect("Dynamite", &["BTS"]), Language::English);
        assert_eq!(detect("봄날", &["방탄소년단"]), Language::Any);
        assert_eq!(detect("", &[]), Language::Any);
    }

    #[test]
    fn mixed_titles_follow_the_majority() {
        // Russian song by an artist with a Latin name.
        assert_eq!(detect("Мама, я танцую", &["#2Маши"]), Language::Russian);
        assert_eq!(
            detect("Love Me Like You Do", &["Скриптонит"]),
            Language::English
        );
    }
}
