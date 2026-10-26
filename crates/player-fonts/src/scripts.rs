//! Which writing systems a font covers, and whether it can show ordinary text at all, judged from its character map
//! (ADR 0042). Every test counts how many characters of a small set are present against a threshold, so an old but
//! complete font missing one or two of them still passes.

use serde::{Deserialize, Serialize};

/// Very common Han characters in their Traditional forms: whether a font can show ordinary Chinese text.
const COMMON_TRADITIONAL: &[char] = &[
    '的', '一', '是', '不', '了', '人', '我', '在', '有', '他', '這', '們', '中', '大', '來', '上',
    '國', '個', '到', '說', '時',
];
/// The same in their Simplified forms.
const COMMON_SIMPLIFIED: &[char] = &[
    '的', '一', '是', '不', '了', '人', '我', '在', '有', '他', '这', '们', '中', '大', '来', '上',
    '国', '个', '到', '说', '时',
];
/// Of either common set at least this many (of 21).
const COMMON_MIN: usize = 18;

/// Traditional-only forms (their Simplified forms differ). Japanese fonts with JIS X 0213 carry most of them too.
const TRADITIONAL_FORMS: &[char] = &['體', '邊', '說', '們', '這', '嗎', '妳', '戶'];
/// Bopomofo: every Big5 and HKSCS font has it, Japanese and Korean fonts do not. This is what tells a Traditional
/// Chinese font from a Japanese one with old forms (Meiryo, Yu Gothic).
const BOPOMOFO: &[char] = &['ㄅ', 'ㄆ', 'ㄇ', 'ㄈ', 'ㄉ', 'ㄊ'];
/// Simplified-only forms (not in Japanese character sets).
const SIMPLIFIED_FORMS: &[char] = &['体', '边', '说', '们', '这', '吗', '书', '东'];
/// Of the forms sets at least this many (of 8). A font that has every Simplified form needs every Traditional form
/// to count as Traditional as well: GBK fonts carry Bopomofo and most Traditional forms (方正 fonts lack 說 or 戶).
const FORMS_MIN: usize = 7;
/// Of `BOPOMOFO` at least this many (of 6).
const BOPOMOFO_MIN: usize = 4;

/// Kana.
const KANA: &[char] = &[
    'あ', 'い', 'う', 'え', 'お', 'の', 'を', 'ア', 'イ', 'ウ', 'エ', 'オ', 'ン', 'ヴ',
];
const KANA_MIN: usize = 12;
/// Very common kanji (Japanese forms: 会, not 會).
const KANJI: &[char] = &[
    '日', '本', '人', '一', '大', '年', '中', '出', '時', '行', '会', '見',
];
const KANJI_MIN: usize = 10;

/// Hangul syllables: a Korean font shows ordinary Korean text even without Hanja or kana.
const HANGUL: &[char] = &[
    '가', '나', '다', '라', '마', '바', '사', '아', '자', '차', '카', '타', '파', '하',
];
const HANGUL_MIN: usize = 12;

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

/// What a face's character map says about it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub scripts: Scripts,
    /// Not offered in the pickers: the font has Han, Bopomofo or kana glyphs, yet cannot show ordinary text in any
    /// of those writing systems (CJK Extension B-only fonts, EUDC / 外字 supplements, Bopomofo-only or kana-only
    /// decorative fonts).
    pub hidden: bool,
}

fn count(set: &[char], has_char: &impl Fn(char) -> bool) -> usize {
    set.iter().filter(|&&c| has_char(c)).count()
}

/// The writing systems `has_char` covers.
pub fn detect_scripts(has_char: impl Fn(char) -> bool) -> Scripts {
    let common_traditional = count(COMMON_TRADITIONAL, &has_char) >= COMMON_MIN;
    let common_simplified = count(COMMON_SIMPLIFIED, &has_char) >= COMMON_MIN;
    let simplified_forms = count(SIMPLIFIED_FORMS, &has_char);
    let traditional_forms_needed = if simplified_forms == SIMPLIFIED_FORMS.len() {
        TRADITIONAL_FORMS.len()
    } else {
        FORMS_MIN
    };
    let mut bits = 0;
    if common_traditional
        && count(TRADITIONAL_FORMS, &has_char) >= traditional_forms_needed
        && count(BOPOMOFO, &has_char) >= BOPOMOFO_MIN
    {
        bits |= Scripts::TRADITIONAL;
    }
    if common_simplified && simplified_forms >= FORMS_MIN {
        bits |= Scripts::SIMPLIFIED;
    }
    if count(KANA, &has_char) >= KANA_MIN && count(KANJI, &has_char) >= KANJI_MIN {
        bits |= Scripts::JAPANESE;
    }
    Scripts(bits)
}

/// A font with at least this many Han, Bopomofo or kana code points is a CJK font (a Latin font may carry a stray
/// ・ or 卍).
const CJK_MARKS_MIN: usize = 16;

/// Whether `code` is a Han ideograph, Bopomofo or kana letter. Compatibility ideographs (icon fonts map glyphs there),
/// the sound marks and the middle dot / prolonged sound mark are left out.
pub fn is_cjk_mark(code: u32) -> bool {
    matches!(code,
        0x3041..=0x3096       // Hiragana
        | 0x30A1..=0x30FA     // Katakana
        | 0x3105..=0x312F     // Bopomofo
        | 0x31A0..=0x31BF     // Bopomofo Extended
        | 0x31F0..=0x31FF     // Katakana Phonetic Extensions
        | 0x3400..=0x4DBF     // Extension A
        | 0x4E00..=0x9FFF     // Unified Ideographs
        | 0x1B000..=0x1B16F   // Kana Supplement / Extended-A
        | 0x20000..=0x2F7FF   // Extensions B to F
        | 0x30000..=0x3FFFF) // Extensions G and later
}

/// The coverage of a Unicode character map: `has_char` asks for one character, `cjk_marks` how many code points the
/// map has a glyph for are `is_cjk_mark` (asked only when no ordinary text test passes).
pub fn assess_coverage(
    has_char: impl Fn(char) -> bool,
    cjk_marks: impl FnOnce() -> usize,
) -> Coverage {
    let scripts = detect_scripts(&has_char);
    let shows_text = scripts.0 != 0
        || count(COMMON_TRADITIONAL, &has_char) >= COMMON_MIN
        || count(COMMON_SIMPLIFIED, &has_char) >= COMMON_MIN
        || count(HANGUL, &has_char) >= HANGUL_MIN;
    Coverage {
        scripts,
        hidden: !shows_text && cjk_marks() >= CJK_MARKS_MIN,
    }
}

/// The writing systems of a font whose character map has no Unicode subtable, from its legacy encodings
/// (`(platform id, encoding id)` of each subtable): Windows ShiftJIS (3, 2) and Macintosh Japanese (1, 1) are
/// Japanese, Windows PRC (3, 3) and Macintosh Simplified Chinese (1, 25) Simplified, Windows Big5 (3, 4) and Macintosh
/// Traditional Chinese (1, 2) Traditional. Korean encodings (Wansung, Johab, Macintosh Korean) mark nothing, so such a
/// font sorts with the Latin-only ones. These fonts are never hidden.
pub fn legacy_encoding_scripts(encodings: &[(u16, u16)]) -> Scripts {
    let mut bits = 0;
    for &encoding in encodings {
        bits |= match encoding {
            (3, 2) | (1, 1) => Scripts::JAPANESE,
            (3, 3) | (1, 25) => Scripts::SIMPLIFIED,
            (3, 4) | (1, 2) => Scripts::TRADITIONAL,
            _ => 0,
        };
    }
    Scripts(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covering<'a>(sets: &'a [&'a [char]]) -> impl Fn(char) -> bool + 'a {
        move |c| c.is_ascii() || sets.iter().any(|set| set.contains(&c))
    }

    fn without<'a>(sets: &'a [&'a [char]], missing: &'a str) -> impl Fn(char) -> bool + 'a {
        move |c| !missing.contains(c) && covering(sets)(c)
    }

    const TC: &[&[char]] = &[COMMON_TRADITIONAL, TRADITIONAL_FORMS, BOPOMOFO];
    const SC: &[&[char]] = &[COMMON_SIMPLIFIED, SIMPLIFIED_FORMS];
    const JA: &[&[char]] = &[KANA, KANJI];

    #[test]
    fn each_writing_system_needs_its_sets_up_to_the_thresholds() {
        assert_eq!(detect_scripts(covering(&[])), Scripts(0));
        assert_eq!(detect_scripts(covering(TC)), Scripts(Scripts::TRADITIONAL));
        assert_eq!(detect_scripts(covering(SC)), Scripts(Scripts::SIMPLIFIED));
        assert_eq!(detect_scripts(covering(JA)), Scripts(Scripts::JAPANESE));
        let all: Vec<&[char]> = TC.iter().chain(SC).chain(JA).copied().collect();
        assert_eq!(
            detect_scripts(covering(&all)),
            Scripts(Scripts::TRADITIONAL | Scripts::SIMPLIFIED | Scripts::JAPANESE)
        );
    }

    #[test]
    fn an_old_complete_font_missing_a_few_characters_still_passes() {
        // A Big5 font without 妳, two Bopomofo letters and one common character.
        assert_eq!(
            detect_scripts(without(TC, "妳ㄉㄊ的")),
            Scripts(Scripts::TRADITIONAL)
        );
        assert_eq!(
            detect_scripts(without(SC, "东的")),
            Scripts(Scripts::SIMPLIFIED)
        );
        // A GBK font: every Simplified form, Bopomofo, all but one Traditional form: Simplified only.
        let gbk: &[&[char]] = &[
            COMMON_SIMPLIFIED,
            SIMPLIFIED_FORMS,
            COMMON_TRADITIONAL,
            TRADITIONAL_FORMS,
            BOPOMOFO,
        ];
        assert_eq!(
            detect_scripts(without(gbk, "戶")),
            Scripts(Scripts::SIMPLIFIED)
        );
        assert_eq!(
            detect_scripts(covering(gbk)),
            Scripts(Scripts::TRADITIONAL | Scripts::SIMPLIFIED)
        );
        // Too many gone: not that writing system any more.
        assert_eq!(detect_scripts(without(TC, "妳嗎")), Scripts(0));
        assert_eq!(detect_scripts(without(TC, "ㄅㄆㄇ")), Scripts(0));
        assert_eq!(detect_scripts(without(TC, "的一是不")), Scripts(0));
    }

    #[test]
    fn a_japanese_font_with_old_forms_but_no_bopomofo_is_not_traditional() {
        // Meiryo / Yu Gothic: kana, kanji and every Traditional form, but no Bopomofo.
        let meiryo: &[&[char]] = &[KANA, KANJI, COMMON_TRADITIONAL, TRADITIONAL_FORMS];
        let coverage = assess_coverage(covering(meiryo), || 100);
        assert_eq!(coverage.scripts, Scripts(Scripts::JAPANESE));
        assert!(!coverage.hidden);
    }

    #[test]
    fn fonts_that_cannot_show_ordinary_text_are_hidden() {
        // CJK Extension B only (細明體-ExtB): Han glyphs, none of the common characters.
        let ext_b = |c: char| c.is_ascii() || u32::from(c) >= 0x20000;
        let coverage = assess_coverage(ext_b, || 100);
        assert_eq!(
            coverage,
            Coverage {
                scripts: Scripts(0),
                hidden: true
            }
        );
        // A Latin font with a few stray CJK glyphs is not a CJK font.
        assert!(!assess_coverage(ext_b, || 3).hidden);
        // An EUDC supplement (外字集): a few common characters and full Bopomofo, mostly rare and private-use glyphs.
        let eudc = |c: char| c.is_ascii() || "的人ㄅㄆㄇㄈㄉㄊ".contains(c);
        assert!(assess_coverage(eudc, || 100).hidden);
        // Bopomofo letters and Latin only (精靈文).
        let bopomofo = covering(&[BOPOMOFO]);
        assert!(assess_coverage(bopomofo, || 100).hidden);
        // Kana only.
        assert!(assess_coverage(covering(&[KANA]), || 100).hidden);
    }

    #[test]
    fn latin_korean_and_hanja_fonts_stay_listed() {
        // Plain Latin: no CJK glyphs at all.
        assert_eq!(assess_coverage(covering(&[]), || 0), Coverage::default());
        // A Korean font: Hangul shows ordinary text, whatever stray Han or kana glyphs it has.
        assert!(!assess_coverage(covering(&[HANGUL]), || 100).hidden);
        // Korean Hanja fonts: the common characters in Traditional forms, no Bopomofo, no kana.
        let hanja = covering(&[COMMON_TRADITIONAL]);
        let coverage = assess_coverage(hanja, || 100);
        assert_eq!(
            coverage,
            Coverage {
                scripts: Scripts(0),
                hidden: false
            }
        );
    }

    #[test]
    fn legacy_encodings_decide_the_writing_system() {
        assert_eq!(
            legacy_encoding_scripts(&[(3, 3)]),
            Scripts(Scripts::SIMPLIFIED)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(3, 4)]),
            Scripts(Scripts::TRADITIONAL)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(3, 2)]),
            Scripts(Scripts::JAPANESE)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(1, 0), (1, 2)]),
            Scripts(Scripts::TRADITIONAL)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(1, 25)]),
            Scripts(Scripts::SIMPLIFIED)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(1, 1)]),
            Scripts(Scripts::JAPANESE)
        );
        assert_eq!(
            legacy_encoding_scripts(&[(3, 5)]),
            Scripts(0),
            "Wansung: Latin group"
        );
        assert_eq!(
            legacy_encoding_scripts(&[(3, 6), (1, 3), (1, 0), (3, 0)]),
            Scripts(0)
        );
    }

    #[test]
    fn cjk_marks_cover_han_bopomofo_and_kana_but_not_hangul_or_private_use() {
        for code in [
            0x4E00, 0x3400, 0x20000, 0x2A6D6, 0x31350, 0x3105, 0x3042, 0x30A2,
        ] {
            assert!(is_cjk_mark(code), "{code:X}");
        }
        for code in [
            0x41, 0xAC00, 0xE000, 0xF0000, 0x2E80, 0xF900, 0x2F800, 0x30FB, 0x30FC,
        ] {
            assert!(!is_cjk_mark(code), "{code:X}");
        }
    }

    #[test]
    fn the_distinguishing_sets_do_not_overlap() {
        for c in TRADITIONAL_FORMS.iter().chain(BOPOMOFO) {
            assert!(!SIMPLIFIED_FORMS.contains(c) && !KANA.contains(c) && !KANJI.contains(c));
        }
        for c in SIMPLIFIED_FORMS {
            assert!(!KANA.contains(c) && !KANJI.contains(c));
        }
    }
}
