//! Which writing systems a font covers, judged from its character map (ADR 0042): a small set of characters that only
//! one writing system uses must all be present.

use serde::{Deserialize, Serialize};

/// Traditional-only forms (their Simplified forms differ) and Bopomofo: Japanese fonts with JIS X 0213 carry these
/// forms too, but no Bopomofo (Big5 and HKSCS fonts all do).
const TRADITIONAL: &[char] = &[
    '體', '邊', '說', '們', '這', '嗎', '妳', '戶', 'ㄅ', 'ㄆ', 'ㄇ', 'ㄈ',
];
/// Simplified-only forms (not in Japanese character sets).
const SIMPLIFIED: &[char] = &['体', '边', '说', '们', '这', '吗', '书', '东'];
/// Kana.
const JAPANESE: &[char] = &['あ', 'の', 'を', 'ア', 'ン', 'ヴ'];

/// A set of writing systems, stored as bits (`TRADITIONAL` 1, `SIMPLIFIED` 2, `JAPANESE` 4). No bit: Latin only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Scripts(pub u8);

impl Scripts {
    pub const TRADITIONAL: u8 = 1;
    pub const SIMPLIFIED: u8 = 2;
    pub const JAPANESE: u8 = 4;

    pub fn traditional(self) -> bool {
        self.0 & Self::TRADITIONAL != 0
    }
    pub fn simplified(self) -> bool {
        self.0 & Self::SIMPLIFIED != 0
    }
    pub fn japanese(self) -> bool {
        self.0 & Self::JAPANESE != 0
    }
    pub fn union(self, other: Scripts) -> Scripts {
        Scripts(self.0 | other.0)
    }
}

/// The writing systems `has_char` covers.
pub fn detect_scripts(has_char: impl Fn(char) -> bool) -> Scripts {
    let covers = |set: &[char]| set.iter().all(|&c| has_char(c));
    let mut bits = 0;
    if covers(TRADITIONAL) {
        bits |= Scripts::TRADITIONAL;
    }
    if covers(SIMPLIFIED) {
        bits |= Scripts::SIMPLIFIED;
    }
    if covers(JAPANESE) {
        bits |= Scripts::JAPANESE;
    }
    Scripts(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covering<'a>(sets: &'a [&'a [char]]) -> impl Fn(char) -> bool + 'a {
        move |c| c.is_ascii() || sets.iter().any(|set| set.contains(&c))
    }

    #[test]
    fn each_set_must_be_covered_completely() {
        assert_eq!(detect_scripts(covering(&[])), Scripts(0));
        assert_eq!(
            detect_scripts(covering(&[TRADITIONAL])),
            Scripts(Scripts::TRADITIONAL)
        );
        assert_eq!(
            detect_scripts(covering(&[SIMPLIFIED, JAPANESE])),
            Scripts(Scripts::SIMPLIFIED | Scripts::JAPANESE)
        );
        // A Japanese font with a few traditional forms (體 說 邊) but not all of them is not traditional.
        let japanese_with_kyujitai = |c: char| JAPANESE.contains(&c) || "體說邊".contains(c);
        assert_eq!(
            detect_scripts(japanese_with_kyujitai),
            Scripts(Scripts::JAPANESE)
        );
    }

    #[test]
    fn the_sets_do_not_overlap() {
        for c in TRADITIONAL {
            assert!(!SIMPLIFIED.contains(c) && !JAPANESE.contains(c));
        }
        for c in SIMPLIFIED {
            assert!(!JAPANESE.contains(c));
        }
    }
}
