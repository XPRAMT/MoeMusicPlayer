use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use unicode_normalization::UnicodeNormalization;

pub const MAX_LYRIC_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
/// Automatic lyric acceptance gates (both must pass; the weighted score only ranks).
/// Title: same normalized similarity used for 「標題相符」 in the match explanation.
pub const AUTO_MATCH_TITLE_MIN_SIMILARITY: f32 = 0.88;
/// Duration: the full-credit bucket of the duration score (|local - remote| <= 2 s).
/// Both durations must be known; an unverifiable duration never auto-applies.
pub const AUTO_MATCH_MAX_DURATION_DIFF_MS: u64 = 2_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LyricProvider {
    #[serde(rename = "sidecar")]
    Sidecar,
    #[serde(rename = "embedded")]
    Embedded,
    #[serde(rename = "netease")]
    NetEase,
    #[serde(rename = "qqmusic", alias = "qq")]
    Qq,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricFormat {
    Lrc,
    Yrc,
    Qrc,
    Plain,
    Srt,
    Ass,
}

/// One timed word (or syllable) of a line. `text` keeps the spaces that follow the word, so the
/// line text is the concatenation of its words.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub start_ms: Option<u64>,
    /// The end of the line when the source says it: SRT cue end, ASS event end, yrc and QRC line start + length (or
    /// a later last word end), enhanced LRC closing stamp. Plain LRC lines have none (they end at the next one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_ms: Option<u64>,
    pub text: String,
    #[serde(default)]
    pub translation: Option<String>,
    #[serde(default)]
    pub romanization: Option<String>,
    /// Word timing; empty for line-level lyrics. When present, `text` equals the words joined.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LyricWord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub romanization_words: Vec<LyricWord>,
}

impl LyricLine {
    pub fn timed(start_ms: u64, text: impl Into<String>) -> Self {
        Self {
            start_ms: Some(start_ms),
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn untimed(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LyricAuxiliaryKind {
    Translation,
    Romanization,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedLyrics {
    pub format: LyricFormat,
    pub lines: Vec<LyricLine>,
    pub raw_text: String,
    pub raw_karaoke: Option<String>,
    pub synced: bool,
}

/// Persisted line-synced lyrics for one internal track identity. The manual flag records that a
/// person selected this provider candidate; future automatic results must not replace it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackLyrics {
    pub track_id: crate::TrackId,
    pub provider: LyricProvider,
    pub candidate_id: Option<String>,
    pub lyrics: ParsedLyrics,
    pub manually_selected: bool,
    /// UTC epoch milliseconds.
    pub updated_at_utc_ms: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsTrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricCandidate {
    pub candidate_id: String,
    pub provider: LyricProvider,
    pub provider_track_id: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub lyrics: ParsedLyrics,
    pub score: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LyricMatchScore {
    pub value: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LyricsParseError {
    PayloadTooLarge,
}

impl std::fmt::Display for LyricsParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PayloadTooLarge => write!(f, "lyric payload exceeds the supported size limit"),
        }
    }
}

impl std::error::Error for LyricsParseError {}

/// Parse ordinary LRC timestamps. Repeated timestamps on one physical line become separate
/// line rows. An offset header applies to all timestamped rows in the document.
///
/// Rows from different physical lines that share one start timestamp are folded into a single
/// `LyricLine` by [`merge_same_timestamp_lines`]: the first row in file order is the primary text,
/// the second is the translation and the third is the romanization; further rows are ignored.
pub fn parse_lrc(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    check_payload_size(input)?;

    let mut offset_ms = 0_i64;
    for line in input.lines() {
        let trimmed = line.trim_start_matches('\u{feff}').trim();
        if let Some(value) = metadata_tag_value(trimmed, "offset") {
            if let Ok(value) = value.trim().parse::<i64>() {
                offset_ms = value;
            }
        }
    }

    let mut lines = Vec::new();
    let mut plain_lines = Vec::new();
    for line in input.lines() {
        let physical = line.trim_start_matches('\u{feff}');
        let mut remainder = physical;
        let mut timestamps = Vec::new();
        while let Some(tagged) = remainder.strip_prefix('[') {
            let Some(end) = tagged.find(']') else {
                break;
            };
            let value = &tagged[..end];
            if let Some(timestamp) = parse_lrc_timestamp(value) {
                timestamps.push(apply_offset(timestamp, offset_ms));
                remainder = &tagged[end + 1..];
            } else if metadata_tag_value(&format!("[{value}]"), "offset").is_some() {
                remainder = &tagged[end + 1..];
            } else {
                break;
            }
        }

        let (text, words, end_ms) = match enhanced_lrc_words(remainder.trim(), offset_ms) {
            Some((text, words, end_ms)) => (text, words, end_ms),
            None => (remainder.trim().to_owned(), Vec::new(), None),
        };
        // `[00:01.00][00:01.00]text` names one cue twice; it must not turn into its own
        // translation when same-timestamp rows are folded below.
        timestamps.sort_unstable();
        timestamps.dedup();
        if !timestamps.is_empty() {
            if !text.is_empty() {
                lines.extend(timestamps.into_iter().map(|start_ms| LyricLine {
                    words: words.clone(),
                    // The closing stamp; never before the line's start.
                    end_ms: end_ms.map(|end| end.max(start_ms)),
                    ..LyricLine::timed(start_ms, text.clone())
                }));
            }
        } else if !text.is_empty() && !is_lrc_metadata_tag(text.as_str()) {
            plain_lines.push(LyricLine::untimed(text));
        }
    }

    // Stable sort keeps file order inside one timestamp, which defines the role of each row.
    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());
    let mut lines = merge_same_timestamp_lines(lines);
    close_open_enhanced_words(&mut lines);
    let synced = !lines.is_empty();
    if !synced {
        lines = plain_lines;
    }

    Ok(ParsedLyrics {
        format: if synced {
            LyricFormat::Lrc
        } else {
            LyricFormat::Plain
        },
        lines,
        raw_text: input.to_owned(),
        raw_karaoke: None,
        synced,
    })
}

/// Parse NetEase's word-timed YRC: `[line start,line length](word start,word length,0)word…`, all
/// in absolute milliseconds. The JSON credit rows (`{"t":…}`) are skipped; spaces after a word stay
/// with it, so the line text is its words joined. The payload is kept in `raw_karaoke` so stored
/// results can be parsed again by a later parser.
pub fn parse_yrc(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    check_payload_size(input)?;

    let mut lines = Vec::new();
    for physical in input.lines() {
        let line = physical
            .trim_start_matches('\u{feff}')
            .trim_end_matches('\r');
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('{') {
            continue;
        }
        if let Some(parsed) = parse_yrc_line(trimmed) {
            if !parsed.text.trim().is_empty() {
                lines.push(parsed);
            }
        }
    }
    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());

    Ok(ParsedLyrics {
        format: LyricFormat::Yrc,
        synced: !lines.is_empty(),
        lines,
        raw_text: input.to_owned(),
        raw_karaoke: Some(input.to_owned()),
    })
}

/// Preserve a provider's QRC payload without attempting the proprietary decryption. If a
/// provider also returned line-level LRC, callers should parse that LRC and attach this raw value
/// with `with_raw_karaoke`.
pub fn preserve_qrc(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    check_payload_size(input)?;
    Ok(ParsedLyrics {
        format: LyricFormat::Qrc,
        lines: Vec::new(),
        raw_text: String::new(),
        raw_karaoke: Some(input.to_owned()),
        synced: false,
    })
}

pub fn with_raw_karaoke(mut lyrics: ParsedLyrics, raw_karaoke: Option<String>) -> ParsedLyrics {
    lyrics.raw_karaoke = raw_karaoke;
    lyrics
}

/// Fold adjacent timed rows that share one start timestamp into one line. Within a group the
/// first row (file order) is the primary text; the other rows become its translation or
/// romanization by content ([`crate::lyric_roles`], ADR 0041), with the old positional rule
/// (second row translation, third romanization) only for rows the content does not decide. A
/// role that is already populated is kept; rows beyond the two roles are ignored. Untimed rows
/// are never merged. Callers must pass rows already ordered by start time with file order
/// preserved inside each timestamp (as `parse_lrc` does).
pub fn merge_same_timestamp_lines(lines: Vec<LyricLine>) -> Vec<LyricLine> {
    let mut groups: Vec<(LyricLine, Vec<crate::lyric_roles::ExtraRow>)> =
        Vec::with_capacity(lines.len());
    for line in lines {
        if let (Some(start_ms), Some((primary, extras))) = (line.start_ms, groups.last_mut()) {
            if primary.start_ms == Some(start_ms) {
                extras.push(crate::lyric_roles::ExtraRow { line, hint: None });
                continue;
            }
        }
        groups.push((line, Vec::new()));
    }
    groups
        .into_iter()
        .map(|(mut primary, extras)| {
            crate::lyric_roles::fold_extra_rows(&mut primary, extras);
            primary
        })
        .collect()
}

/// Attach translation or romanization only when its parsed LRC start timestamp matches a primary
/// line. Mismatched or untimed auxiliary text is left unattached instead of shifting line pairs.
/// A role already supplied by same-timestamp rows inside the primary LRC wins; a separate
/// translation/romanization document only fills lines that are still missing that role.
pub fn merge_lrc_auxiliary(
    primary: &mut ParsedLyrics,
    auxiliary_text: &str,
    kind: LyricAuxiliaryKind,
) -> Result<(), LyricsParseError> {
    if !primary.synced {
        return Ok(());
    }
    let auxiliary = parse_lrc(auxiliary_text)?;
    if !auxiliary.synced {
        return Ok(());
    }

    let mut lines_by_time = BTreeMap::<u64, VecDeque<String>>::new();
    for line in auxiliary.lines {
        if let Some(start_ms) = line.start_ms {
            lines_by_time
                .entry(start_ms)
                .or_default()
                .push_back(line.text);
        }
    }
    for line in &mut primary.lines {
        let Some(start_ms) = line.start_ms else {
            continue;
        };
        let Some(auxiliary_text) = lines_by_time
            .get_mut(&start_ms)
            .and_then(VecDeque::pop_front)
        else {
            continue;
        };
        let slot = match kind {
            LyricAuxiliaryKind::Translation => &mut line.translation,
            LyricAuxiliaryKind::Romanization => &mut line.romanization,
        };
        if slot.is_none() {
            *slot = Some(auxiliary_text);
        }
    }
    Ok(())
}

fn has_words(candidate: &LyricCandidate) -> bool {
    candidate
        .lyrics
        .lines
        .iter()
        .any(|line| !line.words.is_empty())
}

/// Order by score (best first); equal scores prefer word-timed lyrics, then the provider track id.
pub fn rank_lyric_candidates(
    local: &LyricsTrackMetadata,
    candidates: impl IntoIterator<Item = LyricCandidate>,
) -> Vec<LyricCandidate> {
    let mut ranked = candidates
        .into_iter()
        .map(|mut candidate| {
            candidate.score = score_lyric_candidate(local, &candidate).value;
            candidate
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            // Equal scores: word timing beats line timing (the AMLL QRC of a QQ song).
            .then_with(|| has_words(right).cmp(&has_words(left)))
            .then_with(|| left.provider_track_id.cmp(&right.provider_track_id))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    ranked
}

/// Pass/fail for automatic use: title close AND duration close (see the gate constants).
pub fn passes_auto_match_gates(local: &LyricsTrackMetadata, candidate: &LyricCandidate) -> bool {
    let title_close = matches!(
        (&local.title, &candidate.title),
        (Some(local), Some(remote)) if text_similarity(local, remote) >= AUTO_MATCH_TITLE_MIN_SIMILARITY
    );
    let duration_close = matches!(
        (local.duration_ms, candidate.duration_ms),
        (Some(local), Some(remote)) if local.abs_diff(remote) <= AUTO_MATCH_MAX_DURATION_DIFF_MS
    );
    title_close && duration_close
}

/// Highest-scored timed candidate among those passing the gates. `ranked` is sorted by score
/// (see `rank_lyric_candidates`), so the score only orders; it no longer decides acceptance.
pub fn auto_lyric_candidate<'a>(
    local: &LyricsTrackMetadata,
    ranked: &'a [LyricCandidate],
) -> Option<&'a LyricCandidate> {
    ranked
        .iter()
        .find(|candidate| candidate.lyrics.synced && passes_auto_match_gates(local, candidate))
}

pub fn score_lyric_candidate(
    local: &LyricsTrackMetadata,
    candidate: &LyricCandidate,
) -> LyricMatchScore {
    let mut score = 0.0_f32;
    if let (Some(local), Some(remote)) = (&local.title, &candidate.title) {
        score += 0.55 * text_similarity(local, remote);
    }
    if let (Some(local), Some(remote)) = (&local.artist, &candidate.artist) {
        score += 0.25 * text_similarity(local, remote);
    }
    if let (Some(local), Some(remote)) = (local.duration_ms, candidate.duration_ms) {
        score += 0.15 * duration_similarity(local, remote);
    }
    if let (Some(local), Some(remote)) = (&local.album, &candidate.album) {
        score += 0.05 * text_similarity(local, remote);
    }
    LyricMatchScore {
        value: score.clamp(0.0, 1.0),
    }
}

fn check_payload_size(input: &str) -> Result<(), LyricsParseError> {
    if input.len() > MAX_LYRIC_PAYLOAD_BYTES {
        Err(LyricsParseError::PayloadTooLarge)
    } else {
        Ok(())
    }
}

fn metadata_tag_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let (tag, value) = inner.split_once(':')?;
    tag.eq_ignore_ascii_case(name).then_some(value)
}

fn is_lrc_metadata_tag(line: &str) -> bool {
    let Some(inner) = line
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    else {
        return false;
    };
    let Some((tag, _)) = inner.split_once(':') else {
        return false;
    };
    matches!(
        tag.to_ascii_lowercase().as_str(),
        "ar" | "al" | "ti" | "au" | "by" | "re" | "ve" | "length" | "offset" | "la" | "id"
    )
}

fn parse_lrc_timestamp(value: &str) -> Option<u64> {
    let (minutes, seconds) = value.split_once(':')?;
    if minutes.is_empty() || !minutes.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let minutes = minutes.parse::<u64>().ok()?;
    let (whole_seconds, fraction) = seconds
        .find(['.', ':'])
        .map(|separator| seconds.split_at(separator))
        .map(|(whole, fraction)| (whole, &fraction[1..]))
        .unwrap_or((seconds, ""));
    if whole_seconds.is_empty() || !whole_seconds.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let whole_seconds = whole_seconds.parse::<u64>().ok()?;
    if whole_seconds >= 60 {
        return None;
    }
    let fraction_ms = if fraction.is_empty() {
        0
    } else if fraction.len() <= 2 && fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        let digits = fraction.parse::<u64>().ok()?;
        if fraction.len() == 1 {
            digits * 100
        } else {
            digits * 10
        }
    } else if fraction.len() == 3 && fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        fraction.parse::<u64>().ok()?
    } else {
        return None;
    };
    minutes
        .checked_mul(60_000)?
        .checked_add(whole_seconds.checked_mul(1_000)?)?
        .checked_add(fraction_ms)
}

/// A word whose end is not written yet (enhanced LRC without a closing stamp); closed by
/// [`close_open_enhanced_words`] at the next line's start.
const OPEN_WORD_END: u64 = u64::MAX;

/// Enhanced LRC (A2 extension) word stamps: `<mm:ss.xx>word <mm:ss.xx>word<mm:ss.xx>`. Each word
/// runs from its stamp to the next one; a final stamp with no text after it is the end of the last
/// word. Text before the first stamp joins the first word. Returns the line text (stamps removed,
/// the words joined), the words and the closing stamp (the line's own end) when there is one, or `None` when the text
/// has no word stamp.
fn enhanced_lrc_words(text: &str, offset_ms: i64) -> Option<(String, Vec<LyricWord>, Option<u64>)> {
    let mut stamps: Vec<(u64, String)> = Vec::new();
    let mut leading = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let found = rest.find('<').and_then(|open| {
            let close = open + rest[open..].find('>')?;
            parse_lrc_timestamp(&rest[open + 1..close]).map(|ms| (open, close, ms))
        });
        let Some((open, close, ms)) = found else {
            match stamps.last_mut() {
                Some((_, word)) => word.push_str(rest),
                None => leading.push_str(rest),
            }
            break;
        };
        match stamps.last_mut() {
            Some((_, word)) => word.push_str(&rest[..open]),
            None => leading.push_str(&rest[..open]),
        }
        stamps.push((apply_offset(ms, offset_ms), String::new()));
        rest = &rest[close + 1..];
    }
    if stamps.is_empty() {
        return None;
    }
    if let Some((_, first)) = stamps.first_mut() {
        first.insert_str(0, &leading);
    }
    let mut words = Vec::with_capacity(stamps.len());
    for (index, (start_ms, word)) in stamps.iter().enumerate() {
        let next = stamps.get(index + 1).map(|(next, _)| *next);
        if word.is_empty() {
            // The closing stamp, or the end of a word followed by a pause before the next one.
            continue;
        }
        words.push(LyricWord {
            start_ms: *start_ms,
            end_ms: next.map_or(OPEN_WORD_END, |next| next.max(*start_ms)),
            text: word.clone(),
        });
    }
    let line: String = words.iter().map(|word| word.text.as_str()).collect();
    // A closing stamp (the last stamp, with no text after it) is the line's own end.
    let end = stamps
        .last()
        .filter(|(_, word)| word.is_empty() && !words.is_empty())
        .map(|(end, _)| *end);
    Some((line, words, end))
}

/// The last word of an enhanced LRC line without a closing stamp ends at the next line's start
/// (or at its own start on the last line).
fn close_open_enhanced_words(lines: &mut [LyricLine]) {
    for index in 0..lines.len() {
        let next_start = lines.get(index + 1).and_then(|line| line.start_ms);
        let line = &mut lines[index];
        for word in line
            .words
            .iter_mut()
            .chain(line.romanization_words.iter_mut())
        {
            if word.end_ms == OPEN_WORD_END {
                word.end_ms = next_start.unwrap_or(word.start_ms).max(word.start_ms);
            }
        }
    }
}

fn apply_offset(timestamp_ms: u64, offset_ms: i64) -> u64 {
    if offset_ms >= 0 {
        timestamp_ms.saturating_add(offset_ms as u64)
    } else {
        timestamp_ms.saturating_sub(offset_ms.unsigned_abs())
    }
}

/// A `(start,length[,flag])` word timing at the start of `text`: the start, the end and the rest.
fn yrc_word_timing(text: &str) -> Option<(u64, u64, &str)> {
    let after_open = text.strip_prefix('(')?;
    let close = after_open.find(')')?;
    let values = after_open[..close].split(',').collect::<Vec<_>>();
    if values.len() < 2 || values.len() > 3 {
        return None;
    }
    let numbers = values
        .iter()
        .map(|value| value.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    Some((
        numbers[0],
        numbers[0].saturating_add(numbers[1]),
        &after_open[close + 1..],
    ))
}

fn parse_yrc_line(line: &str) -> Option<LyricLine> {
    let tagged = line.strip_prefix('[')?;
    let end = tagged.find(']')?;
    let mut parts = tagged[..end].split(',');
    let start_ms = parts.next()?.trim().parse::<u64>().ok()?;
    let duration_ms = parts.next()?.trim().parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }

    let mut remaining = &tagged[end + 1..];
    let mut words: Vec<LyricWord> = Vec::new();
    while !remaining.is_empty() {
        if let Some((word_start, word_end, rest)) = yrc_word_timing(remaining) {
            words.push(LyricWord {
                start_ms: word_start,
                end_ms: word_end,
                text: String::new(),
            });
            remaining = rest;
            continue;
        }
        let character = remaining.chars().next()?;
        match words.last_mut() {
            Some(word) => word.text.push(character),
            // Text before the first timing belongs to the line start.
            None => words.push(LyricWord {
                start_ms,
                end_ms: start_ms,
                text: character.to_string(),
            }),
        }
        remaining = &remaining[character.len_utf8()..];
    }
    let line_end = words
        .iter()
        .map(|word| word.end_ms)
        .fold(start_ms.saturating_add(duration_ms), u64::max);
    if line_end == start_ms && words.is_empty() {
        return None;
    }
    Some(LyricLine {
        start_ms: Some(start_ms),
        end_ms: Some(line_end),
        text: words.iter().map(|word| word.text.as_str()).collect(),
        words,
        ..LyricLine::default()
    })
}

/// Attach auxiliary LRC rows to the nearest primary line within `tolerance_ms` (NetEase's
/// `tlyric`/`romalrc` follow the LRC times, which differ from yrc line starts by a few ms). Each
/// auxiliary row is used once; a role that is already filled is kept.
pub fn merge_lrc_auxiliary_nearest(
    primary: &mut ParsedLyrics,
    auxiliary_text: &str,
    kind: LyricAuxiliaryKind,
    tolerance_ms: u64,
) -> Result<(), LyricsParseError> {
    if !primary.synced {
        return Ok(());
    }
    let auxiliary = parse_lrc(auxiliary_text)?;
    if !auxiliary.synced {
        return Ok(());
    }
    let mut used = vec![false; auxiliary.lines.len()];
    for line in &mut primary.lines {
        let Some(start_ms) = line.start_ms else {
            continue;
        };
        let slot = match kind {
            LyricAuxiliaryKind::Translation => &mut line.translation,
            LyricAuxiliaryKind::Romanization => &mut line.romanization,
        };
        if slot.is_some() {
            continue;
        }
        let best = auxiliary
            .lines
            .iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .filter_map(|(index, aux)| {
                let distance = aux.start_ms?.abs_diff(start_ms);
                (distance <= tolerance_ms).then_some((distance, index))
            })
            .min();
        if let Some((_, index)) = best {
            used[index] = true;
            if !auxiliary.lines[index].text.trim().is_empty() {
                *slot = Some(auxiliary.lines[index].text.clone());
            }
        }
    }
    Ok(())
}

/// Whether yrc lines can stand in for the provider's LRC: at least 80% as many lines and the
/// last starts within 2 s of each other.
pub fn yrc_matches_lrc(yrc: &ParsedLyrics, lrc: &ParsedLyrics) -> bool {
    if !yrc.synced || yrc.lines.iter().all(|line| line.words.is_empty()) {
        return false;
    }
    if !lrc.synced || lrc.lines.is_empty() {
        return true;
    }
    // LRC often starts with credit rows that yrc keeps as JSON; only rows from the first yrc
    // line on are counted.
    let first_yrc_start = yrc
        .lines
        .first()
        .and_then(|line| line.start_ms)
        .unwrap_or(0);
    let lrc_count = lrc
        .lines
        .iter()
        .filter(|line| line.start_ms.unwrap_or(0) + 500 >= first_yrc_start)
        .count();
    if yrc.lines.len() * 5 < lrc_count * 4 {
        return false;
    }
    let close = |a: Option<&LyricLine>, b: Option<&LyricLine>| match (
        a.and_then(|l| l.start_ms),
        b.and_then(|l| l.start_ms),
    ) {
        (Some(a), Some(b)) => a.abs_diff(b) < 2_000,
        _ => false,
    };
    // The first rows are credits whose times differ between the two; the last lyric line is
    // the reliable anchor.
    close(yrc.lines.last(), lrc.lines.last())
}

/// Stored results from before word timing kept the yrc only in `raw_karaoke` (the raw text, or
/// the `{"yrc": …}` JSON the service writes). Fill in the words of lines that match a yrc line by
/// start time, without touching anything else.
pub fn restore_words_from_raw_karaoke(lyrics: &mut ParsedLyrics) {
    if lyrics.lines.iter().any(|line| !line.words.is_empty()) {
        return;
    }
    let Some(raw) = lyrics.raw_karaoke.as_deref() else {
        return;
    };
    let yrc_text = if raw.trim_start().starts_with('{') && raw.contains("\"yrc\"") {
        match json_string_field(raw, "yrc") {
            Some(text) => text,
            None => return,
        }
    } else {
        raw.to_owned()
    };
    let Ok(yrc) = parse_yrc(&yrc_text) else {
        return;
    };
    let by_start = yrc
        .lines
        .into_iter()
        .filter_map(|line| Some((line.start_ms?, line)))
        .collect::<BTreeMap<_, _>>();
    for line in &mut lyrics.lines {
        if let Some(found) = line.start_ms.and_then(|start| by_start.get(&start)) {
            if found.text.trim() == line.text.trim() {
                line.text = found.text.clone();
                line.end_ms = found.end_ms;
                line.words = found.words.clone();
            }
        }
    }
}

/// The Traditional spelling of each word of a line, or `None` when nothing changes. The line is
/// converted as a whole (OpenCC matches phrases, so word-by-word conversion would lose 头发 →
/// 頭髮) and the result is cut at the same character positions as the words; when the conversion
/// changed the number of characters, each word is converted on its own instead.
pub fn traditional_word_texts(words: &[LyricWord]) -> Option<Vec<String>> {
    if words.is_empty() {
        return None;
    }
    let line: String = words.iter().map(|word| word.text.as_str()).collect();
    let converted = match player_zh::to_traditional(&line) {
        std::borrow::Cow::Borrowed(_) => return None,
        std::borrow::Cow::Owned(converted) => converted,
    };
    let converted_chars = converted.chars().collect::<Vec<_>>();
    if converted_chars.len() == line.chars().count() {
        let mut at = 0;
        return Some(
            words
                .iter()
                .map(|word| {
                    let count = word.text.chars().count();
                    let piece = converted_chars[at..at + count].iter().collect();
                    at += count;
                    piece
                })
                .collect(),
        );
    }
    Some(
        words
            .iter()
            .map(|word| player_zh::to_traditional(&word.text).into_owned())
            .collect(),
    )
}

/// A minimal reader for one top-level JSON string field (the service's own `PreservedKaraoke`).
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let after = &json[json.find(&needle)? + needle.len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    let mut chars = after.strip_prefix('"')?.chars();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    let code = u32::from_str_radix(&hex, 16).ok()?;
                    if (0xD800..0xDC00).contains(&code) {
                        // Surrogate pair.
                        if chars.next()? != '\\' || chars.next()? != 'u' {
                            return None;
                        }
                        let low: String = chars.by_ref().take(4).collect();
                        let low = u32::from_str_radix(&low, 16).ok()?;
                        out.push(char::from_u32(
                            0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00),
                        )?);
                    } else {
                        out.push(char::from_u32(code)?);
                    }
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn duration_similarity(local: u64, remote: u64) -> f32 {
    let difference = local.abs_diff(remote);
    match difference {
        0..=2_000 => 1.0,
        2_001..=5_000 => 0.5,
        _ => 0.0,
    }
}

fn text_similarity(left: &str, right: &str) -> f32 {
    let left = normalized_tokens(left);
    let right = normalized_tokens(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left == right {
        return 1.0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let max_len = left_chars.len().max(right_chars.len());
    if max_len == 0 {
        return 0.0;
    }
    let distance = levenshtein_distance(&left_chars, &right_chars);
    1.0 - distance as f32 / max_len as f32
}

fn fold_chinese_script(character: char) -> char {
    player_zh::fold_to_simplified(character)
}

fn normalized_tokens(value: &str) -> String {
    // Fold Traditional/Simplified Chinese to one script so 愛/爱 or 戀/恋 compare equal.
    let compatibility = value
        .nfkc()
        .flat_map(char::to_lowercase)
        .map(fold_chinese_script)
        .collect::<String>();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in compatibility.chars() {
        if character.is_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let feature_at = tokens
        .iter()
        .position(|token| matches!(token.as_str(), "feat" | "ft" | "featuring"));
    if let Some(index) = feature_at {
        tokens.truncate(index);
    }
    tokens.join(" ")
}

fn levenshtein_distance(left: &[char], right: &[char]) -> usize {
    if left.is_empty() {
        return right.len();
    }
    if right.is_empty() {
        return left.len();
    }
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            let substitution =
                previous[right_index] + usize::from(left_character != right_character);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(substitution);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod enhanced_lrc_tests {
    use super::*;

    #[test]
    fn enhanced_lrc_words_are_read_with_translation_and_romanization() {
        let input = "[00:01.00]<00:01.00>Hello <00:01.50>world<00:02.25>
[00:01.00]你好世界
[00:01.00]<00:01.00>ha <00:01.50>ro<00:02.00>
[00:03.00]<00:03.00>next <00:03.40>line
";
        let parsed = parse_lrc(input).unwrap();
        assert!(parsed.synced);
        assert_eq!(
            crate::detect_lyric_format(input),
            crate::lyric_formats::DetectedLyricFormat::Lrc
        );
        let first = &parsed.lines[0];
        assert_eq!(first.text, "Hello world");
        assert_eq!(first.translation.as_deref(), Some("你好世界"));
        assert_eq!(first.romanization.as_deref(), Some("ha ro"));
        assert_eq!(
            first.words,
            vec![
                LyricWord {
                    start_ms: 1000,
                    end_ms: 1500,
                    text: "Hello ".into()
                },
                LyricWord {
                    start_ms: 1500,
                    end_ms: 2250,
                    text: "world".into()
                },
            ]
        );
        assert_eq!(first.romanization_words.len(), 2);
        // No closing stamp: the last word ends at the next line, or at its start on the last line.
        assert_eq!(
            parsed.lines[1].words[1],
            LyricWord {
                start_ms: 3400,
                end_ms: 3400,
                text: "line".into()
            }
        );
    }

    #[test]
    fn plain_lrc_has_no_words_and_offset_moves_word_stamps() {
        let parsed = parse_lrc("[00:01.00]plain <not a stamp>").unwrap();
        assert!(parsed.lines[0].words.is_empty());
        assert_eq!(parsed.lines[0].text, "plain <not a stamp>");
        let parsed = parse_lrc(
            "[offset:500]
[00:01.00]<00:01.00>a<00:01.20>b<00:01.40>",
        )
        .unwrap();
        assert_eq!(parsed.lines[0].start_ms, Some(1500));
        assert_eq!(parsed.lines[0].words[0].start_ms, 1500);
        assert_eq!(parsed.lines[0].words[1].end_ms, 1900);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        candidate_id: &str,
        title: &str,
        artist: &str,
        album: Option<&str>,
        duration_ms: Option<u64>,
    ) -> LyricCandidate {
        LyricCandidate {
            candidate_id: candidate_id.to_owned(),
            provider: LyricProvider::NetEase,
            provider_track_id: Some(candidate_id.to_owned()),
            title: Some(title.to_owned()),
            artist: Some(artist.to_owned()),
            album: album.map(str::to_owned),
            duration_ms,
            lyrics: parse_lrc("[00:01.00]line").expect("fixture LRC"),
            score: 0.0,
        }
    }

    #[test]
    fn lrc_supports_offsets_metadata_and_multiple_timestamps_per_row() {
        let parsed = parse_lrc(
            "[ar:artist]\n[ti:title]\n[offset:-250]\n[00:01.20][00:03.4]line\n[00:05.000]next",
        )
        .expect("valid LRC");

        assert_eq!(parsed.format, LyricFormat::Lrc);
        assert!(parsed.synced);
        assert_eq!(
            parsed.lines,
            vec![
                LyricLine {
                    start_ms: Some(950),
                    text: "line".into(),
                    translation: None,
                    romanization: None,
                    ..LyricLine::default()
                },
                LyricLine {
                    start_ms: Some(3_150),
                    text: "line".into(),
                    translation: None,
                    romanization: None,
                    ..LyricLine::default()
                },
                LyricLine {
                    start_ms: Some(4_750),
                    text: "next".into(),
                    translation: None,
                    romanization: None,
                    ..LyricLine::default()
                },
            ]
        );
    }

    #[test]
    fn lrc_accepts_colon_separated_centiseconds_without_losing_lines() {
        let mut fixture = String::new();
        for second in 0..43 {
            let separator = if second < 3 { '.' } else { ':' };
            fixture.push_str(&format!(
                "[00:{second:02}{separator}25]synthetic line {second}\n"
            ));
        }

        let parsed = parse_lrc(&fixture).expect("synthetic LRC");

        assert_eq!(parsed.format, LyricFormat::Lrc);
        assert!(parsed.synced);
        assert_eq!(parsed.raw_text, fixture);
        assert_eq!(parsed.lines.len(), 43);
        assert!(parsed.lines.iter().all(|line| line.start_ms.is_some()));
        let expected_text = (0..43)
            .map(|second| format!("synthetic line {second}"))
            .collect::<Vec<_>>();
        let parsed_text = parsed
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            parsed_text, expected_text,
            "all lyric text must survive parsing"
        );
        assert_eq!(parsed.lines[3].start_ms, Some(3_250));
    }

    #[test]
    fn same_timestamp_rows_become_primary_and_translation() {
        let parsed = parse_lrc("[00:13.64]二番なんて望んでない\n[00:13.64]才不稀罕當第二呢")
            .expect("valid LRC");

        assert!(parsed.synced);
        assert_eq!(
            parsed.lines,
            vec![LyricLine {
                start_ms: Some(13_640),
                text: "二番なんて望んでない".into(),
                translation: Some("才不稀罕當第二呢".into()),
                romanization: None,
                ..LyricLine::default()
            }]
        );
    }

    #[test]
    fn same_timestamp_third_row_is_romanization_and_extras_are_ignored() {
        let parsed = parse_lrc(
            "[00:01.00]原文\n[00:01.00]譯文\n[00:01.00]genbun\n[00:01.00]extra\n[00:02.00]next",
        )
        .expect("valid LRC");

        assert_eq!(
            parsed.lines,
            vec![
                LyricLine {
                    start_ms: Some(1_000),
                    text: "原文".into(),
                    translation: Some("譯文".into()),
                    romanization: Some("genbun".into()),
                    ..LyricLine::default()
                },
                LyricLine {
                    start_ms: Some(2_000),
                    text: "next".into(),
                    translation: None,
                    romanization: None,
                    ..LyricLine::default()
                },
            ]
        );
    }

    #[test]
    fn same_timestamp_rows_keep_file_order_even_when_not_adjacent() {
        let parsed = parse_lrc(
            "[offset:100]\n[00:02.00]second\n[00:01.00]first\n[00:02.00]第二\n[00:01.00]第一",
        )
        .expect("valid LRC");

        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].start_ms, Some(1_100));
        assert_eq!(parsed.lines[0].text, "first");
        assert_eq!(parsed.lines[0].translation.as_deref(), Some("第一"));
        assert_eq!(parsed.lines[1].start_ms, Some(2_100));
        assert_eq!(parsed.lines[1].text, "second");
        assert_eq!(parsed.lines[1].translation.as_deref(), Some("第二"));
    }

    #[test]
    fn repeated_timestamp_tags_on_one_row_do_not_translate_themselves() {
        let parsed = parse_lrc("[00:01.00][00:01.00]once\n[00:01.00]一次").expect("valid LRC");

        assert_eq!(
            parsed.lines,
            vec![LyricLine {
                start_ms: Some(1_000),
                text: "once".into(),
                translation: Some("一次".into()),
                romanization: None,
                ..LyricLine::default()
            }]
        );
    }

    #[test]
    fn empty_same_timestamp_rows_do_not_take_a_role() {
        let parsed = parse_lrc("[00:01.00]原文\n[00:01.00]\n[00:01.00]譯文").expect("valid LRC");

        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].translation.as_deref(), Some("譯文"));
        assert_eq!(parsed.lines[0].romanization, None);
    }

    #[test]
    fn separate_auxiliary_lrc_only_fills_roles_missing_from_same_timestamp_rows() {
        let mut primary =
            parse_lrc("[00:01.00]one\n[00:01.00]內嵌譯\n[00:02.00]two").expect("primary LRC");
        merge_lrc_auxiliary(
            &mut primary,
            "[00:01.00]外部譯一\n[00:02.00]外部譯二",
            LyricAuxiliaryKind::Translation,
        )
        .expect("translation LRC");

        assert_eq!(primary.lines[0].translation.as_deref(), Some("內嵌譯"));
        assert_eq!(primary.lines[1].translation.as_deref(), Some("外部譯二"));
    }

    #[test]
    fn same_timestamp_lrc_rows_are_classified_by_content_in_any_order() {
        for body in [
            "[00:01.00]君の声\n[00:01.00]你的聲音\n[00:01.00]kimi no koe",
            "[00:01.00]君の声\n[00:01.00]kimi no koe\n[00:01.00]你的聲音",
        ] {
            let line = &parse_lrc(body).unwrap().lines[0];
            assert_eq!(line.text, "君の声");
            assert_eq!(line.translation.as_deref(), Some("你的聲音"));
            assert_eq!(line.romanization.as_deref(), Some("kimi no koe"));
        }
        let english = parse_lrc("[00:01.00]Hello my friend\n[00:01.00]你好我的朋友").unwrap();
        assert_eq!(
            english.lines[0].translation.as_deref(),
            Some("你好我的朋友")
        );
        // The enhanced-LRC writer's order (text, translation, romanization) reads back the same,
        // and romanization keeps its word stamps.
        let written = "[00:01.00]<00:01.00>君<00:01.50>の<00:02.00>\n[00:01.00]你的\n[00:01.00]<00:01.00>kimi <00:01.50>no<00:02.00>";
        let line = &parse_lrc(written).unwrap().lines[0];
        assert_eq!(line.translation.as_deref(), Some("你的"));
        assert_eq!(line.romanization.as_deref(), Some("kimi no"));
        assert_eq!(line.romanization_words.len(), 2);
    }

    #[test]
    fn legacy_rows_with_duplicate_timestamps_are_folded_and_untimed_rows_are_kept() {
        let row = |start_ms: Option<u64>, text: &str| LyricLine {
            start_ms,
            text: text.into(),
            translation: None,
            romanization: None,
            ..LyricLine::default()
        };
        let merged = merge_same_timestamp_lines(vec![
            row(Some(500), "a"),
            row(Some(500), "a-tr"),
            row(None, "plain"),
            row(None, "plain"),
            row(Some(900), "b"),
        ]);

        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].translation.as_deref(), Some("a-tr"));
        assert_eq!(merged[1].text, "plain");
        assert_eq!(merged[2].text, "plain");
        assert_eq!(merged[3].text, "b");
    }

    #[test]
    fn yrc_keeps_word_timing_and_retains_raw_payload() {
        let raw =
            "{\"t\":0,\"c\":[{\"tx\":\"作词: \"}]}\n[1200,1800](1200,300,0)早安(1500,250,0)世界";
        let parsed = parse_yrc(raw).expect("valid YRC");

        assert_eq!(parsed.format, LyricFormat::Yrc);
        assert!(parsed.synced);
        assert_eq!(parsed.raw_karaoke.as_deref(), Some(raw));
        assert_eq!(
            parsed.lines,
            vec![LyricLine {
                start_ms: Some(1_200),
                end_ms: Some(3_000),
                text: "早安世界".into(),
                words: vec![
                    LyricWord {
                        start_ms: 1_200,
                        end_ms: 1_500,
                        text: "早安".into()
                    },
                    LyricWord {
                        start_ms: 1_500,
                        end_ms: 1_750,
                        text: "世界".into()
                    },
                ],
                ..LyricLine::default()
            }]
        );
    }

    #[test]
    fn yrc_keeps_trailing_spaces_zero_length_and_empty_words() {
        let raw = "[7260,2000](7260,390,0)Will (7650,0,0)(7650,0,0)you (8000,500,0)go";
        let parsed = parse_yrc(raw).expect("valid YRC");
        let line = &parsed.lines[0];
        assert_eq!(line.text, "Will you go");
        assert_eq!(line.words.len(), 4);
        assert_eq!(line.words[0].text, "Will ");
        assert_eq!(line.words[1].text, "");
        assert_eq!(
            line.words[2],
            LyricWord {
                start_ms: 7_650,
                end_ms: 7_650,
                text: "you ".into()
            }
        );
        let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(joined, line.text);
        assert_eq!(line.end_ms, Some(9_260));
    }

    #[test]
    fn yrc_translation_prefers_exact_ytlrc_and_falls_back_to_nearest_tlyric() {
        let mut exact =
            parse_yrc("[6840,1000](6840,500,0)a(7340,500,0)b\n[9000,1000](9000,1000,0)c").unwrap();
        merge_lrc_auxiliary(
            &mut exact,
            "[00:06.840]甲\n[00:09.000]乙",
            LyricAuxiliaryKind::Translation,
        )
        .unwrap();
        assert_eq!(exact.lines[0].translation.as_deref(), Some("甲"));
        assert_eq!(exact.lines[1].translation.as_deref(), Some("乙"));

        let mut nearest = parse_yrc("[6840,1000](6840,500,0)a\n[9000,1000](9000,1000,0)c").unwrap();
        merge_lrc_auxiliary_nearest(
            &mut nearest,
            "[00:06.838]甲\n[00:09.250]乙\n[00:20.000]遠",
            LyricAuxiliaryKind::Translation,
            300,
        )
        .unwrap();
        assert_eq!(nearest.lines[0].translation.as_deref(), Some("甲"));
        assert_eq!(nearest.lines[1].translation.as_deref(), Some("乙"));
    }

    #[test]
    fn old_stored_line_level_json_still_reads_and_words_come_back_from_raw_karaoke() {
        // A row stored before word timing existed: no endMs, words or romanizationWords.
        let old = r#"{"format":"lrc","lines":[{"startMs":1200,"text":"早安世界","translation":"早","romanization":null}],"rawText":"[00:01.20]早安世界","rawKaraoke":"{\"yrc\":\"[1200,1800](1200,300,0)早安(1500,250,0)世界\",\"qrc_base64\":null}","synced":true}"#;
        let mut parsed: ParsedLyrics = serde_json::from_str(old).expect("old JSON");
        assert!(parsed.lines[0].words.is_empty());
        restore_words_from_raw_karaoke(&mut parsed);
        assert_eq!(parsed.lines[0].words.len(), 2);
        assert_eq!(parsed.lines[0].translation.as_deref(), Some("早"));
        // A line-level line serializes without the new keys.
        let line = serde_json::to_value(LyricLine::timed(5, "x")).unwrap();
        assert!(line.get("words").is_none() && line.get("endMs").is_none());
    }

    #[test]
    fn traditional_words_keep_phrase_conversion_and_word_boundaries() {
        let words = vec![
            LyricWord {
                start_ms: 0,
                end_ms: 1,
                text: "头".into(),
            },
            LyricWord {
                start_ms: 1,
                end_ms: 2,
                text: "发 ".into(),
            },
            LyricWord {
                start_ms: 2,
                end_ms: 3,
                text: "这只".into(),
            },
        ];
        let converted = traditional_word_texts(&words).expect("changed");
        assert_eq!(converted, ["頭", "髮 ", "這只"]);
        let unchanged = vec![LyricWord {
            start_ms: 0,
            end_ms: 1,
            text: "abc".into(),
        }];
        assert!(traditional_word_texts(&unchanged).is_none());
    }

    #[test]
    fn yrc_stands_in_for_lrc_only_when_lines_and_ends_agree() {
        let yrc = parse_yrc("[1000,500](1000,500,0)a\n[5000,500](5000,500,0)b").unwrap();
        let lrc = parse_lrc("[00:00.00]credits\n[00:01.00]a\n[00:05.00]b").unwrap();
        assert!(yrc_matches_lrc(&yrc, &lrc));
        let longer = parse_lrc("[00:01.00]a\n[00:02.00]x\n[00:03.00]y\n[00:05.00]b").unwrap();
        assert!(!yrc_matches_lrc(&yrc, &longer));
    }

    #[test]
    fn qrc_is_preserved_without_claiming_line_sync_or_decryption() {
        let raw = "E9056DD20F5EB670F81ED60AB3D2C4CC";
        let parsed = preserve_qrc(raw).expect("bounded QRC");

        assert_eq!(parsed.format, LyricFormat::Qrc);
        assert!(!parsed.synced);
        assert!(parsed.lines.is_empty());
        assert_eq!(parsed.raw_karaoke.as_deref(), Some(raw));
    }

    #[test]
    fn auxiliary_lrc_rows_are_matched_by_time_not_position() {
        let mut primary = parse_lrc("[00:01.00]one\n[00:02.00]two").expect("primary LRC");
        merge_lrc_auxiliary(
            &mut primary,
            "[00:02.00]第二行\n[00:01.00]第一行",
            LyricAuxiliaryKind::Translation,
        )
        .expect("translation LRC");
        merge_lrc_auxiliary(
            &mut primary,
            "[00:01.00]ichi\n[00:03.00]missing",
            LyricAuxiliaryKind::Romanization,
        )
        .expect("romanization LRC");

        assert_eq!(primary.lines[0].translation.as_deref(), Some("第一行"));
        assert_eq!(primary.lines[1].translation.as_deref(), Some("第二行"));
        assert_eq!(primary.lines[0].romanization.as_deref(), Some("ichi"));
        assert_eq!(primary.lines[1].romanization, None);
    }

    #[test]
    fn untimed_auxiliary_text_is_not_guessed_into_timed_rows() {
        let mut primary = parse_lrc("[00:01.00]one\n[00:02.00]two").expect("primary LRC");
        merge_lrc_auxiliary(
            &mut primary,
            "first\nsecond",
            LyricAuxiliaryKind::Translation,
        )
        .expect("plain translation");

        assert!(primary.lines.iter().all(|line| line.translation.is_none()));
    }

    #[test]
    fn parser_rejects_payloads_over_the_shared_bound() {
        let oversized = "x".repeat(MAX_LYRIC_PAYLOAD_BYTES + 1);
        assert_eq!(
            parse_lrc(&oversized),
            Err(LyricsParseError::PayloadTooLarge)
        );
        assert_eq!(
            parse_yrc(&oversized),
            Err(LyricsParseError::PayloadTooLarge)
        );
        assert_eq!(
            preserve_qrc(&oversized),
            Err(LyricsParseError::PayloadTooLarge)
        );
    }

    #[test]
    fn matcher_normalizes_fullwidth_and_feature_credit_but_keeps_version_markers() {
        let local = LyricsTrackMetadata {
            title: Some("ＡＢＣ Song feat. Guest".into()),
            artist: Some("Artist (feat. Guest)".into()),
            album: None,
            duration_ms: Some(180_000),
        };
        let matching = candidate("a", "ABC Song", "Artist", None, Some(180_500));
        let live_version = candidate("b", "ABC Song Live", "Artist", None, Some(180_500));

        assert!(score_lyric_candidate(&local, &matching).value > 0.94);
        assert!(score_lyric_candidate(&local, &live_version).value < 0.82);
    }

    #[test]
    fn duration_has_material_weight_and_album_has_low_weight() {
        let local = LyricsTrackMetadata {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Original Album".into()),
            duration_ms: Some(180_000),
        };
        let wrong_duration =
            candidate("a", "Song", "Artist", Some("Original Album"), Some(210_000));
        let wrong_album = candidate("b", "Song", "Artist", Some("Other Album"), Some(180_000));

        assert!(
            score_lyric_candidate(&local, &wrong_duration).value
                < score_lyric_candidate(&local, &wrong_album).value
        );
        assert!(score_lyric_candidate(&local, &wrong_album).value > 0.9);
    }

    #[test]
    fn candidate_score_weights_are_title_55_artist_25_duration_15_album_5() {
        let score_for = |metadata, candidate| score_lyric_candidate(&metadata, &candidate).value;
        let title_only = LyricsTrackMetadata {
            title: Some("Song".into()),
            artist: None,
            album: None,
            duration_ms: None,
        };
        let artist_only = LyricsTrackMetadata {
            title: None,
            artist: Some("Artist".into()),
            album: None,
            duration_ms: None,
        };
        let duration_only = LyricsTrackMetadata {
            title: None,
            artist: None,
            album: None,
            duration_ms: Some(180_000),
        };
        let album_only = LyricsTrackMetadata {
            title: None,
            artist: None,
            album: Some("Album".into()),
            duration_ms: None,
        };
        assert_eq!(
            score_for(title_only, candidate("title", "Song", "Artist", None, None)),
            0.55
        );
        assert_eq!(
            score_for(
                artist_only,
                candidate("artist", "Song", "Artist", None, None)
            ),
            0.25
        );
        assert_eq!(
            score_for(
                duration_only,
                candidate("duration", "Song", "Artist", None, Some(180_000))
            ),
            0.15
        );
        assert_eq!(
            score_for(
                album_only,
                candidate("album", "Song", "Artist", Some("Album"), None)
            ),
            0.05
        );
    }

    fn gate_local() -> LyricsTrackMetadata {
        LyricsTrackMetadata {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            duration_ms: Some(180_000),
        }
    }

    #[test]
    fn automatic_selection_requires_title_and_duration_gates_not_score() {
        let local = gate_local();
        let mut low_score_but_close =
            candidate("close", "Song", "Other Artist", None, Some(181_500));
        low_score_but_close.score = 0.1;
        assert_eq!(
            auto_lyric_candidate(&local, &[low_score_but_close])
                .map(|item| item.candidate_id.as_str()),
            Some("close")
        );

        let mut high_score_far_duration = candidate("far", "Song", "Artist", None, Some(182_001));
        high_score_far_duration.score = 0.99;
        assert!(auto_lyric_candidate(&local, &[high_score_far_duration]).is_none());

        let mut different_title = candidate("title", "Another Tune", "Artist", None, Some(180_000));
        different_title.score = 0.99;
        assert!(auto_lyric_candidate(&local, &[different_title]).is_none());

        let mut unknown_duration = candidate("unknown", "Song", "Artist", None, None);
        unknown_duration.score = 0.99;
        assert!(auto_lyric_candidate(&local, &[unknown_duration]).is_none());
    }

    #[test]
    fn automatic_selection_picks_highest_score_among_passing_candidates() {
        let local = gate_local();
        let mut failing_top = candidate("failing-top", "Song", "Artist", None, Some(200_000));
        failing_top.score = 0.99;
        let mut passing_high = candidate("passing-high", "Song", "Artist", None, Some(180_500));
        passing_high.score = 0.9;
        let mut passing_low = candidate("passing-low", "Song", "Someone", None, Some(179_000));
        passing_low.score = 0.6;
        assert_eq!(
            auto_lyric_candidate(&local, &[failing_top, passing_high, passing_low])
                .map(|item| item.candidate_id.as_str()),
            Some("passing-high")
        );
    }

    #[test]
    fn equal_scores_rank_word_timed_lyrics_first() {
        let local = gate_local();
        let mut line_level = candidate("qqmusic:mid", "Song", "Artist", None, Some(180_000));
        line_level.provider_track_id = Some("mid".into());
        line_level.lyrics = parse_lrc("[00:01.00]a").expect("fixture LRC");
        let mut worded = candidate("qqmusic:zz", "Song", "Artist", None, Some(180_000));
        worded.provider_track_id = Some("mid".into());
        worded.lyrics = crate::parse_qrc("[1000,500]a(1000,500)").expect("fixture QRC");
        let ranked = rank_lyric_candidates(&local, [line_level, worded]);
        assert_eq!(ranked[0].candidate_id, "qqmusic:zz");
        assert_eq!(
            auto_lyric_candidate(&local, &ranked).map(|item| item.candidate_id.as_str()),
            Some("qqmusic:zz")
        );
    }

    #[test]
    fn automatic_selection_still_requires_timed_lyrics() {
        let local = gate_local();
        let mut untimed = candidate("untimed", "Song", "Artist", None, Some(180_000));
        untimed.score = 0.99;
        untimed.lyrics = parse_lrc("plain lyrics").expect("fixture plain lyrics");
        assert!(!untimed.lyrics.synced);
        assert!(auto_lyric_candidate(&local, &[untimed]).is_none());
    }

    #[test]
    fn title_and_artist_similarity_ignore_simplified_traditional_differences() {
        assert_eq!(text_similarity("愛的戀歌", "爱的恋歌"), 1.0);
        assert_eq!(
            text_similarity("戀愛サーキュレーション", "恋爱サーキュレーション"),
            1.0
        );
        assert_eq!(text_similarity("張學友", "张学友"), 1.0);
        let local = LyricsTrackMetadata {
            title: Some("愛".into()),
            artist: Some("鄧紫棋".into()),
            album: None,
            duration_ms: Some(200_000),
        };
        let remote = candidate("zh", "爱", "邓紫棋", None, Some(200_500));
        assert!(passes_auto_match_gates(&local, &remote));
    }
}
