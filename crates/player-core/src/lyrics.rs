use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

pub const MAX_LYRIC_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;
pub const AUTO_MATCH_MIN_SCORE: f32 = 0.82;
pub const AUTO_MATCH_MIN_MARGIN: f32 = 0.10;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum LyricProvider {
    #[serde(rename = "sidecar")]
    Sidecar,
    #[serde(rename = "embedded")]
    Embedded,
    #[serde(rename = "netease")]
    NetEase,
    #[serde(rename = "qq")]
    Qq,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LyricFormat {
    Lrc,
    Yrc,
    Qrc,
    Plain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub start_ms: Option<u64>,
    pub text: String,
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

        let text = remainder.trim().to_owned();
        if !timestamps.is_empty() {
            if !text.is_empty() {
                lines.extend(timestamps.into_iter().map(|start_ms| LyricLine {
                    start_ms: Some(start_ms),
                    text: text.clone(),
                }));
            }
        } else if !text.is_empty() && !is_lrc_metadata_tag(text.as_str()) {
            plain_lines.push(LyricLine {
                start_ms: None,
                text,
            });
        }
    }

    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());
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

/// Reduce NetEase's timed YRC words to line-level timestamps and text. The full YRC payload is
/// retained so a future renderer can add word highlighting without fetching it again.
pub fn parse_yrc(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    check_payload_size(input)?;

    let mut lines = Vec::new();
    for physical in input.lines() {
        let line = physical.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('{') {
            continue;
        }
        if let Some((start_ms, content)) = parse_yrc_line(line) {
            let text = content.trim().to_owned();
            if !text.is_empty() {
                lines.push(LyricLine {
                    start_ms: Some(start_ms),
                    text,
                });
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
            .then_with(|| left.provider_track_id.cmp(&right.provider_track_id))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    ranked
}

/// Return an automatic choice only when the highest score is strong and clearly separated from
/// the next materially different recording.
pub fn auto_lyric_candidate(ranked: &[LyricCandidate]) -> Option<&LyricCandidate> {
    let best = ranked.first()?;
    if best.score < AUTO_MATCH_MIN_SCORE || !best.lyrics.synced {
        return None;
    }
    if let Some(second) = ranked
        .iter()
        .skip(1)
        .find(|candidate| !same_recording(best, candidate))
    {
        if best.score - second.score < AUTO_MATCH_MIN_MARGIN {
            return None;
        }
    }
    Some(best)
}

pub fn score_lyric_candidate(
    local: &LyricsTrackMetadata,
    candidate: &LyricCandidate,
) -> LyricMatchScore {
    let mut score = 0.0_f32;
    if let (Some(local), Some(remote)) = (&local.title, &candidate.title) {
        score += 0.45 * text_similarity(local, remote);
    }
    if let (Some(local), Some(remote)) = (&local.artist, &candidate.artist) {
        score += 0.35 * text_similarity(local, remote);
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
    let (whole_seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
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

fn apply_offset(timestamp_ms: u64, offset_ms: i64) -> u64 {
    if offset_ms >= 0 {
        timestamp_ms.saturating_add(offset_ms as u64)
    } else {
        timestamp_ms.saturating_sub(offset_ms.unsigned_abs())
    }
}

fn parse_yrc_line(line: &str) -> Option<(u64, String)> {
    let tagged = line.strip_prefix('[')?;
    let end = tagged.find(']')?;
    let mut parts = tagged[..end].split(',');
    let start_ms = parts.next()?.parse::<u64>().ok()?;
    let duration_ms = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || duration_ms == 0 {
        return None;
    }

    let mut remaining = &tagged[end + 1..];
    let mut text = String::with_capacity(remaining.len());
    while !remaining.is_empty() {
        if let Some(after_open) = remaining.strip_prefix('(') {
            if let Some(close) = after_open.find(')') {
                let timing = &after_open[..close];
                let values = timing.split(',').collect::<Vec<_>>();
                if values.len() >= 2 && values.iter().all(|value| value.parse::<u64>().is_ok()) {
                    remaining = &after_open[close + 1..];
                    continue;
                }
            }
        }
        let character = remaining.chars().next()?;
        text.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    Some((start_ms, text))
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

fn normalized_tokens(value: &str) -> String {
    let compatibility = value
        .nfkc()
        .flat_map(char::to_lowercase)
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

fn same_recording(left: &LyricCandidate, right: &LyricCandidate) -> bool {
    let Some(left_title) = left.title.as_deref() else {
        return false;
    };
    let Some(right_title) = right.title.as_deref() else {
        return false;
    };
    let Some(left_artist) = left.artist.as_deref() else {
        return false;
    };
    let Some(right_artist) = right.artist.as_deref() else {
        return false;
    };
    if normalized_tokens(left_title) != normalized_tokens(right_title)
        || normalized_tokens(left_artist) != normalized_tokens(right_artist)
    {
        return false;
    }
    matches!((left.duration_ms, right.duration_ms), (Some(a), Some(b)) if a.abs_diff(b) <= 2_000)
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
                },
                LyricLine {
                    start_ms: Some(3_150),
                    text: "line".into(),
                },
                LyricLine {
                    start_ms: Some(4_750),
                    text: "next".into(),
                },
            ]
        );
    }

    #[test]
    fn yrc_collapses_word_timing_to_line_rows_and_retains_raw_payload() {
        let raw = "{\"t\":0,\"c\":[]}\n[1200,1800](1200,300,0)早安(1500,250,0)世界";
        let parsed = parse_yrc(raw).expect("valid YRC");

        assert_eq!(parsed.format, LyricFormat::Yrc);
        assert!(parsed.synced);
        assert_eq!(parsed.raw_karaoke.as_deref(), Some(raw));
        assert_eq!(
            parsed.lines,
            vec![LyricLine {
                start_ms: Some(1_200),
                text: "早安世界".into(),
            }]
        );
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
    fn automatic_selection_requires_high_score_and_a_clear_recording_margin() {
        let local = LyricsTrackMetadata {
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: Some("Album".into()),
            duration_ms: Some(180_000),
        };
        let ranked = rank_lyric_candidates(
            &local,
            [
                candidate("exact", "Song", "Artist", Some("Album"), Some(180_000)),
                candidate("live", "Song Live", "Artist", Some("Album"), Some(180_000)),
            ],
        );
        assert_eq!(
            auto_lyric_candidate(&ranked).map(|item| item.candidate_id.as_str()),
            Some("exact")
        );

        let ambiguous = rank_lyric_candidates(
            &local,
            [
                candidate("exact-a", "Song", "Artist", Some("Album"), Some(180_000)),
                candidate("exact-b", "Song", "Artist", Some("Album"), Some(180_000)),
            ],
        );
        assert_eq!(
            auto_lyric_candidate(&ambiguous).map(|item| item.candidate_id.as_str()),
            Some("exact-a")
        );

        let weak = rank_lyric_candidates(
            &local,
            [candidate(
                "weak",
                "Song",
                "Different Artist",
                Some("Album"),
                Some(180_000),
            )],
        );
        assert!(auto_lyric_candidate(&weak).is_none());
    }
}
