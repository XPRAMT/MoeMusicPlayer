//! The one Simplified/Traditional Chinese converter of MoeMusicPlayer (ADR 0030).
//!
//! Glyphs only, never wording: `to_traditional` is OpenCC s2tw without regional phrases
//! (STPhrases + STCharacters, then TWVariantsPhrases + TWVariants), with the user's keep-只
//! rule; `to_simplified` is OpenCC tw2s without TSCharactersExt (TWVariantsRevPhrases +
//! TWVariantsRev, then TSPhrases + TSCharacters). Text containing Japanese kana is never
//! converted. The dictionaries are the upstream OpenCC ver.1.4.2 text files in
//! `data/opencc-1.4.2/` (Apache-2.0, see `SOURCE.md`); replace that directory as a whole
//! set to upgrade them.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

macro_rules! dictionary {
    ($name:literal) => {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/data/opencc-1.4.2/",
            $name,
            ".txt"
        ))
    };
}

const ST_PHRASES: &str = dictionary!("STPhrases");
const ST_CHARACTERS: &str = dictionary!("STCharacters");
const TW_VARIANTS_PHRASES: &str = dictionary!("TWVariantsPhrases");
const TW_VARIANTS: &str = dictionary!("TWVariants");
const TW_VARIANTS_REV_PHRASES: &str = dictionary!("TWVariantsRevPhrases");
const TW_VARIANTS_REV: &str = dictionary!("TWVariantsRev");
const TS_PHRASES: &str = dictionary!("TSPhrases");
const TS_CHARACTERS: &str = dictionary!("TSCharacters");

/// One OpenCC lookup table: key → first candidate.
struct Dict {
    map: HashMap<Box<str>, Box<str>>,
    max_key_chars: usize,
}

impl Dict {
    /// Merge OpenCC text dictionaries; on equal keys the earlier source wins, which equals
    /// OpenCC's short-circuit group for these tables (the later ones hold single characters).
    fn from_sources(sources: &[&str]) -> Self {
        let mut dict = Self {
            map: HashMap::new(),
            max_key_chars: 0,
        };
        for source in sources {
            for (key, value) in entries(source) {
                dict.insert_if_absent(key, value);
            }
        }
        dict
    }

    fn insert_if_absent(&mut self, key: &str, value: &str) {
        if self.map.contains_key(key) {
            return;
        }
        self.max_key_chars = self.max_key_chars.max(key.chars().count());
        self.map.insert(key.into(), value.into());
    }

    /// Longest key that prefixes `text`: (byte length, value).
    fn longest_prefix<'a>(&'a self, text: &str) -> Option<(usize, &'a str)> {
        let mut ends = [0usize; 64];
        let mut count = 0;
        for (index, character) in text.char_indices().take(self.max_key_chars.min(ends.len())) {
            ends[count] = index + character.len_utf8();
            count += 1;
        }
        ends[..count].iter().rev().find_map(|&end| {
            self.map
                .get(&text[..end])
                .map(|value| (end, value.as_ref()))
        })
    }
}

/// `key<TAB>candidate candidate…` lines; `#` lines are comments. CRLF checkouts are fine.
fn entries(source: &str) -> impl Iterator<Item = (&str, &str)> {
    source.lines().filter_map(|line| {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let (key, values) = line.split_once('\t')?;
        let value = values.split(' ').next().filter(|value| !value.is_empty())?;
        (!key.is_empty()).then_some((key, value))
    })
}

/// OpenCC converter: max-match segmentation, then each conversion stage per segment.
struct Converter {
    segmentation: Dict,
    stages: Vec<Dict>,
}

impl Converter {
    fn convert(&self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        for segment in self.segments(text) {
            let mut converted = Cow::Borrowed(segment);
            for stage in &self.stages {
                converted = Cow::Owned(convert_stage(stage, &converted));
            }
            output.push_str(&converted);
        }
        output
    }

    /// OpenCC MaxMatchSegmentation: dictionary matches are their own segments, runs of
    /// unmatched characters are grouped into one segment.
    fn segments<'a>(&self, text: &'a str) -> Vec<&'a str> {
        let mut segments = Vec::new();
        let mut run_start = 0;
        let mut position = 0;
        while position < text.len() {
            match self.segmentation.longest_prefix(&text[position..]) {
                Some((length, _)) => {
                    if run_start < position {
                        segments.push(&text[run_start..position]);
                    }
                    segments.push(&text[position..position + length]);
                    position += length;
                    run_start = position;
                }
                None => {
                    position += text[position..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
        if run_start < text.len() {
            segments.push(&text[run_start..]);
        }
        segments
    }
}

fn convert_stage(dict: &Dict, text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut position = 0;
    while position < text.len() {
        let rest = &text[position..];
        match dict.longest_prefix(rest) {
            Some((length, value)) => {
                output.push_str(value);
                position += length;
            }
            None => {
                let character = rest.chars().next().expect("non-empty rest");
                output.push(character);
                position += character.len_utf8();
            }
        }
    }
    output
}

/// User rule: 只 stays 只. STPhrases entries that turn 只 into 隻 (这只, 一只猫, 船只…) get
/// an override restoring 只 at those positions, keeping the rest of the phrase conversion.
fn keep_zhi_overrides() -> Vec<(String, String)> {
    entries(ST_PHRASES)
        .filter(|(key, value)| key.contains('只') && value.contains('隻'))
        .filter_map(|(key, value)| {
            let key_chars: Vec<char> = key.chars().collect();
            let value_chars: Vec<char> = value.chars().collect();
            (key_chars.len() == value_chars.len()).then(|| {
                let kept = key_chars
                    .iter()
                    .zip(&value_chars)
                    .map(|(&k, &v)| if k == '只' && v == '隻' { '只' } else { v })
                    .collect();
                (key.to_owned(), kept)
            })
        })
        .collect()
}

fn traditional_converter() -> &'static Converter {
    static CONVERTER: OnceLock<Converter> = OnceLock::new();
    CONVERTER.get_or_init(|| {
        let mut phrases = Dict::from_sources(&[]);
        for (key, value) in keep_zhi_overrides() {
            phrases.insert_if_absent(&key, &value);
        }
        for (key, value) in entries(ST_PHRASES) {
            phrases.insert_if_absent(key, value);
        }
        let segmentation = Dict {
            map: phrases.map.clone(),
            max_key_chars: phrases.max_key_chars,
        };
        for (key, value) in entries(ST_CHARACTERS) {
            phrases.insert_if_absent(key, value);
        }
        Converter {
            segmentation,
            stages: vec![
                phrases,
                Dict::from_sources(&[TW_VARIANTS_PHRASES, TW_VARIANTS]),
            ],
        }
    })
}

fn simplified_converter() -> &'static Converter {
    static CONVERTER: OnceLock<Converter> = OnceLock::new();
    CONVERTER.get_or_init(|| Converter {
        segmentation: Dict::from_sources(&[TS_PHRASES]),
        stages: vec![
            Dict::from_sources(&[TW_VARIANTS_REV_PHRASES, TW_VARIANTS_REV]),
            Dict::from_sources(&[TS_PHRASES, TS_CHARACTERS]),
        ],
    })
}

fn character_fold() -> &'static HashMap<char, char> {
    static FOLD: OnceLock<HashMap<char, char>> = OnceLock::new();
    FOLD.get_or_init(|| {
        let single = |source| {
            entries(source).filter_map(|(key, value)| {
                let mut key = key.chars();
                let mut value = value.chars();
                match (key.next(), key.next(), value.next(), value.next()) {
                    (Some(k), None, Some(v), None) => Some((k, v)),
                    _ => None,
                }
            })
        };
        let variants: HashMap<char, char> = single(TW_VARIANTS_REV).collect();
        let characters: HashMap<char, char> = single(TS_CHARACTERS).collect();
        let mut fold = HashMap::new();
        for (&key, &value) in variants.iter().chain(characters.iter()) {
            let base = variants.get(&key).copied().unwrap_or(key);
            let folded =
                characters
                    .get(&base)
                    .copied()
                    .unwrap_or(if key == base { value } else { base });
            if folded != key {
                fold.entry(key).or_insert(folded);
            }
        }
        fold
    })
}

/// Han ideograph (CJK Unified, extensions, compatibility, radicals, 々〇).
pub fn is_han(character: char) -> bool {
    matches!(
        character as u32,
        0x2E80..=0x2E99
            | 0x2E9B..=0x2EF3
            | 0x2F00..=0x2FD5
            | 0x3005
            | 0x3007
            | 0x3021..=0x3029
            | 0x3038..=0x303B
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFA6D
            | 0xFA70..=0xFAD9
            | 0x20000..=0x2FA1F
            | 0x30000..=0x323AF
    )
}

/// Unicode Script=Hiragana or Script=Katakana. The middle dot ・ and the prolonged sound
/// mark ー are Script=Common, so a Chinese name such as 約翰・藍儂 still converts.
pub fn is_kana(character: char) -> bool {
    matches!(
        character as u32,
        0x3041..=0x3096
            | 0x309D..=0x309F
            | 0x30A1..=0x30FA
            | 0x30FD..=0x30FF
            | 0x31F0..=0x31FF
            | 0x32D0..=0x32FE
            | 0x3300..=0x3357
            | 0xFF66..=0xFF6F
            | 0xFF71..=0xFF9D
            | 0x1AFF0..=0x1AFF3
            | 0x1AFF5..=0x1AFFB
            | 0x1AFFD..=0x1AFFE
            | 0x1B000..=0x1B122
            | 0x1B132
            | 0x1B150..=0x1B152
            | 0x1B155
            | 0x1B164..=0x1B167
            | 0x1F200
    )
}

/// The text has Han and no Japanese kana, so the Chinese dictionaries may touch it.
pub fn is_convertible(text: &str) -> bool {
    text.chars().any(is_han) && !text.chars().any(is_kana)
}

/// Simplified → Taiwan Traditional glyphs. Unchanged when [`is_convertible`] is false.
pub fn to_traditional(text: &str) -> Cow<'_, str> {
    convert_if_needed(text, traditional_converter())
}

/// (Taiwan) Traditional → Simplified glyphs. Unchanged when [`is_convertible`] is false.
pub fn to_simplified(text: &str) -> Cow<'_, str> {
    convert_if_needed(text, simplified_converter())
}

fn convert_if_needed<'a>(text: &'a str, converter: &Converter) -> Cow<'a, str> {
    if !is_convertible(text) {
        return Cow::Borrowed(text);
    }
    let converted = converter.convert(text);
    if converted == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(converted)
    }
}

/// Single-character Traditional → Simplified fold for script-insensitive comparison
/// (lyric candidate scoring). Phrase context is ignored on purpose.
pub fn fold_to_simplified(character: char) -> char {
    character_fold()
        .get(&character)
        .copied()
        .unwrap_or(character)
}

/// Other-script spellings of a search query (Simplified, then Traditional), without the
/// query itself and without duplicates. Empty for text without Han or with kana.
pub fn search_alternates(query: &str) -> Vec<String> {
    let query = query.trim();
    if !is_convertible(query) {
        return Vec::new();
    }
    let mut alternates: Vec<String> = Vec::with_capacity(2);
    for variant in [to_simplified(query), to_traditional(query)] {
        let variant = variant.trim();
        if !variant.is_empty() && variant != query && !alternates.iter().any(|v| v == variant) {
            alternates.push(variant.to_owned());
        }
    }
    alternates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_zhi_and_converts_the_rest() {
        assert_eq!(to_traditional("这只有我知道"), "這只有我知道");
        assert_eq!(to_traditional("一只猫"), "一只貓");
        assert_eq!(to_traditional("船只"), "船只");
    }

    #[test]
    fn one_to_many_uses_phrases() {
        assert_eq!(to_traditional("头发"), "頭髮");
        assert_eq!(to_traditional("发现"), "發現");
        assert_eq!(to_simplified("頭髮"), "头发");
    }

    #[test]
    fn taiwan_glyphs_and_reverse() {
        assert_eq!(to_traditional("里面"), "裡面");
        assert_eq!(to_simplified("看著你"), "看着你");
        assert_eq!(to_simplified("著名"), "著名");
    }

    #[test]
    fn kana_lines_stay() {
        assert_eq!(to_traditional("東京の空"), "東京の空");
        assert_eq!(to_simplified("東京の空"), "東京の空");
        assert!(matches!(to_traditional("ABC"), Cow::Borrowed(_)));
        assert_eq!(to_traditional("约翰・蓝侬"), "約翰・藍儂");
    }

    #[test]
    fn character_fold_matches_simplified() {
        assert_eq!(fold_to_simplified('發'), '发');
        assert_eq!(fold_to_simplified('裡'), '里');
        assert_eq!(fold_to_simplified('a'), 'a');
    }

    #[test]
    fn search_alternates_are_distinct() {
        assert_eq!(search_alternates("周杰伦"), vec!["周杰倫"]);
        assert_eq!(search_alternates("周杰倫"), vec!["周杰伦"]);
        assert!(search_alternates("君の名は").is_empty());
        assert!(search_alternates("love").is_empty());
    }
}
