//! Content-based roles for the extra rows of one lyric line (ADR 0041): LRC rows that share a
//! timestamp, SRT cues with several rows or paired cues, and ASS events at the same time in
//! another style. Each extra row becomes the translation or the romanization by what it is,
//! not by where it sits; only an ambiguous row falls back to the old positional rule
//! (second row translation, third romanization). Provider auxiliary lyrics (NetEase, QQ) carry
//! their own labels and never come through here.

use crate::LyricLine;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExtraRole {
    Translation,
    Romanization,
}

/// CJK ideographs, kana and Hangul.
fn is_asian(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // hiragana, katakana
        | 0x31F0..=0x31FF // katakana phonetic extensions
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x20000..=0x3FFFF
        | 0x1100..=0x11FF // Hangul jamo
        | 0x3130..=0x318F
        | 0xAC00..=0xD7AF
        | 0xFF66..=0xFF9F // half-width katakana
    )
}

/// Latin letters including the accented and tone-marked ones (ā ī ū ē ō, ǎ, ü, ...).
fn is_latin_letter(c: char) -> bool {
    c.is_ascii_alphabetic()
        || (matches!(c as u32, 0x00C0..=0x024F | 0x1E00..=0x1EFF) && c.is_alphabetic())
}

/// Combining diacritics, so decomposed tone marks count as Latin.
fn is_combining(c: char) -> bool {
    matches!(c as u32, 0x0300..=0x036F)
}

fn has_asian(text: &str) -> bool {
    text.chars().any(is_asian)
}

fn has_latin(text: &str) -> bool {
    text.chars().any(is_latin_letter)
}

/// Almost entirely Latin script: letters, spaces, apostrophes, hyphens, digits and basic
/// punctuation, with at least one letter and nothing from another script.
fn is_latin_row(text: &str) -> bool {
    has_latin(text)
        && text.chars().all(|c| {
            is_latin_letter(c)
                || is_combining(c)
                || c.is_whitespace()
                || c.is_ascii_digit()
                || c.is_ascii_punctuation()
                || matches!(
                    c,
                    '’' | '‘'
                        | '“'
                        | '”'
                        | '…'
                        | '–'
                        | '—'
                        | '·'
                        | '、'
                        | '。'
                        | '，'
                        | '！'
                        | '？'
                )
        })
}

/// English function words. A Latin row under an Asian original that uses two of them is an
/// English translation, not a romanization (romaji, Revised Romanization and pinyin do not use
/// them as words; words those systems share with English, such as "to", "me", "in", "no", are
/// left out).
const ENGLISH_WORDS: &[&str] = &[
    "the", "you", "your", "and", "my", "is", "of", "it", "are", "we", "this", "that", "for",
    "with", "i", "i'm", "don't", "be", "all", "can", "will", "what", "when", "never", "just",
];

fn looks_english(text: &str) -> bool {
    let lower = text.to_lowercase();
    let mut hits: Vec<&str> = Vec::new();
    for word in lower.split(|c: char| !(c.is_alphabetic() || c == '\'' || c == '’')) {
        let word = word.replace('’', "'");
        if let Some(found) = ENGLISH_WORDS.iter().find(|w| **w == word) {
            if !hits.contains(found) {
                hits.push(found);
            }
        }
    }
    hits.len() >= 2
}

/// The role an explicit ASS style name gives, if any.
pub fn role_from_style(style: &str) -> Option<ExtraRole> {
    let lower = style.to_lowercase();
    if lower.contains("roma")
        || style.contains("ローマ字")
        || style.contains("羅馬")
        || style.contains("罗马")
    {
        return Some(ExtraRole::Romanization);
    }
    if lower.contains("trans")
        || ["中文", "翻譯", "翻译", "譯文", "译文"]
            .iter()
            .any(|k| style.contains(k))
    {
        return Some(ExtraRole::Translation);
    }
    None
}

/// The role of `extra` beside `original` by content, or `None` when the content does not say.
pub fn classify_extra_row(original: &str, extra: &str) -> Option<ExtraRole> {
    if has_asian(original) {
        if is_latin_row(extra) {
            return Some(if looks_english(extra) {
                ExtraRole::Translation
            } else {
                ExtraRole::Romanization
            });
        }
        if has_asian(extra) {
            return Some(ExtraRole::Translation);
        }
        return None;
    }
    if has_latin(original) && (has_latin(extra) || has_asian(extra)) {
        return Some(ExtraRole::Translation);
    }
    None
}

fn has_kana(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c as u32, 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F))
}

fn has_hangul(text: &str) -> bool {
    text.chars()
        .any(|c| matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF))
}

/// The role of `row` only when its script clearly differs from `original`'s: a Latin row under
/// an Asian original, an Asian row under a Latin original, Han without kana under Japanese, or
/// no Hangul under Korean. Used for the rows of one SRT cue, where same-script rows are a
/// wrapped line rather than a translation.
pub fn contrast_role(original: &str, row: &str) -> Option<ExtraRole> {
    let contrast = (has_asian(original) && is_latin_row(row))
        || (!has_asian(original) && has_latin(original) && has_asian(row))
        || (has_kana(original) && has_asian(row) && !has_kana(row))
        || (has_hangul(original) && has_asian(row) && !has_hangul(row));
    if contrast {
        classify_extra_row(original, row)
    } else {
        None
    }
}

/// One extra row, with an explicit hint (an ASS style name) when it has one.
pub struct ExtraRow {
    pub line: LyricLine,
    pub hint: Option<ExtraRole>,
}

/// Fold extra rows into `primary`. Roles the line already has are kept. Rows with an explicit
/// hint or a clear content role are placed first; ambiguous rows then take their positional
/// slot (the first extra row translation, the second romanization) or, when that is taken, the
/// other free one. Rows left over are dropped, as before.
pub fn fold_extra_rows(primary: &mut LyricLine, extras: Vec<ExtraRow>) {
    let mut ambiguous: Vec<(usize, LyricLine)> = Vec::new();
    for (position, row) in extras.into_iter().enumerate() {
        match row
            .hint
            .or_else(|| classify_extra_row(&primary.text, &row.line.text))
        {
            Some(role) if role_free(primary, role) => assign(primary, role, row.line),
            Some(_) => ambiguous.push((position, row.line)),
            None => ambiguous.push((position, row.line)),
        }
    }
    for (position, line) in ambiguous {
        let positional = if position == 0 {
            ExtraRole::Translation
        } else {
            ExtraRole::Romanization
        };
        let other = match positional {
            ExtraRole::Translation => ExtraRole::Romanization,
            ExtraRole::Romanization => ExtraRole::Translation,
        };
        if role_free(primary, positional) {
            assign(primary, positional, line);
        } else if role_free(primary, other) {
            assign(primary, other, line);
        }
    }
}

fn role_free(line: &LyricLine, role: ExtraRole) -> bool {
    match role {
        ExtraRole::Translation => line.translation.is_none(),
        ExtraRole::Romanization => line.romanization.is_none(),
    }
}

fn assign(primary: &mut LyricLine, role: ExtraRole, row: LyricLine) {
    match role {
        // Translations never carry word timing.
        ExtraRole::Translation => primary.translation = Some(row.text),
        ExtraRole::Romanization => {
            primary.romanization = Some(row.text);
            primary.romanization_words = row.words;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> LyricLine {
        LyricLine::timed(1_000, text)
    }

    fn fold(primary: &str, extras: &[&str]) -> LyricLine {
        let mut p = line(primary);
        fold_extra_rows(
            &mut p,
            extras
                .iter()
                .map(|t| ExtraRow {
                    line: line(t),
                    hint: None,
                })
                .collect(),
        );
        p
    }

    #[test]
    fn japanese_with_romaji_and_chinese_in_either_order() {
        for extras in [
            ["kimi no koe ga kikoeru", "聽見你的聲音"],
            ["聽見你的聲音", "kimi no koe ga kikoeru"],
        ] {
            let l = fold("君の声が聞こえる", &extras);
            assert_eq!(l.romanization.as_deref(), Some("kimi no koe ga kikoeru"));
            assert_eq!(l.translation.as_deref(), Some("聽見你的聲音"));
        }
        let macrons = fold("東京", &["Tōkyō"]);
        assert_eq!(macrons.romanization.as_deref(), Some("Tōkyō"));
        assert!(macrons.translation.is_none());
    }

    #[test]
    fn korean_with_romanization_and_an_english_translation() {
        let l = fold(
            "사랑해요",
            &["I love you, and you are my all", "saranghaeyo"],
        );
        assert_eq!(l.romanization.as_deref(), Some("saranghaeyo"));
        assert_eq!(
            l.translation.as_deref(),
            Some("I love you, and you are my all")
        );
    }

    #[test]
    fn english_original_takes_a_chinese_translation() {
        let l = fold("Hello darkness my old friend", &["你好黑暗，我的老朋友"]);
        assert_eq!(l.translation.as_deref(), Some("你好黑暗，我的老朋友"));
        assert!(l.romanization.is_none());
    }

    #[test]
    fn chinese_with_pinyin() {
        let l = fold("我爱你", &["wǒ ài nǐ"]);
        assert_eq!(l.romanization.as_deref(), Some("wǒ ài nǐ"));
        let decomposed = fold("我爱你", &["wo\u{30c} a\u{300}i ni\u{30c}"]);
        assert!(decomposed.romanization.is_some());
        let with_translation = fold("我爱你", &["wo ai ni", "I love you"]);
        assert_eq!(with_translation.romanization.as_deref(), Some("wo ai ni"));
        assert_eq!(with_translation.translation.as_deref(), Some("I love you"));
    }

    #[test]
    fn style_hints_win_over_content() {
        let mut p = line("君の声");
        fold_extra_rows(
            &mut p,
            vec![
                ExtraRow {
                    line: line("kimi no koe"),
                    hint: role_from_style("Translation"),
                },
                ExtraRow {
                    line: line("你的聲音"),
                    hint: role_from_style("Romaji"),
                },
            ],
        );
        assert_eq!(p.translation.as_deref(), Some("kimi no koe"));
        assert_eq!(p.romanization.as_deref(), Some("你的聲音"));
        assert_eq!(role_from_style("ローマ字"), Some(ExtraRole::Romanization));
        assert_eq!(role_from_style("中文"), Some(ExtraRole::Translation));
        assert_eq!(role_from_style("TRANS-cn"), Some(ExtraRole::Translation));
        assert_eq!(role_from_style("Default"), None);
    }

    #[test]
    fn ambiguous_rows_fall_back_to_position() {
        // Punctuation and digits under an Asian original say nothing.
        let l = fold("♪", &["...", "123"]);
        assert_eq!(l.translation.as_deref(), Some("..."));
        assert_eq!(l.romanization.as_deref(), Some("123"));
        // A romaji row and an ambiguous row: the ambiguous one takes the free slot.
        let l = fold("こんにちは", &["konnichiwa", "!!"]);
        assert_eq!(l.romanization.as_deref(), Some("konnichiwa"));
        assert_eq!(l.translation.as_deref(), Some("!!"));
    }

    #[test]
    fn two_rows_of_one_role_keep_the_first_and_place_the_second_positionally() {
        let l = fold("君", &["你", "妳"]);
        assert_eq!(l.translation.as_deref(), Some("你"));
        assert_eq!(l.romanization.as_deref(), Some("妳"));
    }
}
