//! Subtitle formats used as lyrics (SRT, the ASS subset of ADR 0041) and content-based format
//! detection shared by every lyric text entry point (embedded tags, sidecars, paste/import).
//! The parsers are written for this project; no subtitle library is used.

use crate::lyrics::{
    merge_same_timestamp_lines, parse_lrc, parse_yrc, LyricFormat, LyricLine, LyricWord,
    LyricsParseError, ParsedLyrics, MAX_LYRIC_PAYLOAD_BYTES,
};

/// What a lyric text looks like, judged from its content (the file extension is not used).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectedLyricFormat {
    Ass,
    Srt,
    Yrc,
    Lrc,
    Plain,
}

/// The result of reading a lyric text of unknown format.
#[derive(Clone, Debug, PartialEq)]
pub enum LyricsContent {
    Parsed(ParsedLyrics),
    /// It looks like a subtitle (SRT/ASS) but nothing usable came out of it. It must not be shown
    /// as plain text, and counts as "no lyrics here" so the next source is tried.
    Unusable,
}

impl LyricsContent {
    /// The parsed lyrics when there is at least one line.
    pub fn usable(self) -> Option<ParsedLyrics> {
        match self {
            Self::Parsed(lyrics) if !lyrics.lines.is_empty() => Some(lyrics),
            _ => None,
        }
    }
}

/// The sidecar extensions read beside a song, in the order they are preferred when a folder has
/// more than one for the same song (ADR 0041): `.lrc`, then `.ass`, then `.srt`.
pub const LYRIC_SIDECAR_EXTENSIONS: [&str; 3] = ["lrc", "ass", "srt"];

/// The preference rank of a sidecar file name (lower wins), or `None` when it is not a lyric
/// sidecar. Case does not matter.
pub fn lyric_sidecar_rank(file_name: &str) -> Option<usize> {
    let extension = file_name.rsplit_once('.')?.1;
    LYRIC_SIDECAR_EXTENSIONS
        .iter()
        .position(|candidate| extension.eq_ignore_ascii_case(candidate))
}

fn normalized(input: &str) -> String {
    input
        .trim_start_matches('\u{feff}')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn is_srt_timing_line(line: &str) -> bool {
    srt_timing(line).is_some()
}

fn looks_like_ass(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case("[script info]")
            || line.eq_ignore_ascii_case("[events]")
            || line.starts_with("Dialogue:")
    })
}

fn looks_like_srt(text: &str) -> bool {
    text.lines().any(|line| line.contains("-->"))
}

fn looks_like_yrc(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        let Some((timing, rest)) = line.strip_prefix('[').and_then(|l| l.split_once(']')) else {
            return false;
        };
        let mut parts = timing.split(',');
        let line_timing = matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None)
            if a.trim().parse::<u64>().is_ok() && b.trim().parse::<u64>().is_ok());
        line_timing && rest.starts_with('(')
    })
}

/// Classify a lyric text: ASS markers, then SRT cue timing (`-->`), then yrc, then LRC
/// timestamps, else plain text.
pub fn detect_lyric_format(input: &str) -> DetectedLyricFormat {
    let text = normalized(input);
    if looks_like_ass(&text) {
        DetectedLyricFormat::Ass
    } else if looks_like_srt(&text) {
        DetectedLyricFormat::Srt
    } else if looks_like_yrc(&text) {
        DetectedLyricFormat::Yrc
    } else if parse_lrc(&text).is_ok_and(|parsed| parsed.synced) {
        DetectedLyricFormat::Lrc
    } else {
        DetectedLyricFormat::Plain
    }
}

/// Read a lyric text of any supported format. Plain text is a valid result; a text that looks
/// like a subtitle but does not parse is [`LyricsContent::Unusable`].
pub fn parse_any_lyrics(input: &str) -> Result<LyricsContent, LyricsParseError> {
    if input.len() > MAX_LYRIC_PAYLOAD_BYTES {
        return Err(LyricsParseError::PayloadTooLarge);
    }
    let parsed = match detect_lyric_format(input) {
        DetectedLyricFormat::Ass => parse_ass(input)?,
        DetectedLyricFormat::Srt => parse_srt(input)?,
        DetectedLyricFormat::Yrc => parse_yrc(input)?,
        DetectedLyricFormat::Lrc | DetectedLyricFormat::Plain => {
            return Ok(LyricsContent::Parsed(parse_lrc(input)?));
        }
    };
    Ok(if parsed.lines.is_empty() {
        LyricsContent::Unusable
    } else {
        LyricsContent::Parsed(parsed)
    })
}

// ---------------------------------------------------------------- SRT

/// `hh:mm:ss,mmm` (also `.` and an optional hour part, as WebVTT writes it).
fn subtitle_time(value: &str) -> Option<u64> {
    let value = value.trim();
    let (clock, fraction) = match value.rfind([',', '.']) {
        Some(index) => (&value[..index], &value[index + 1..]),
        None => (value, ""),
    };
    let parts = clock.split(':').collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let numbers = parts
        .iter()
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    let (hours, minutes, seconds) = match numbers.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    if minutes >= 60 || seconds >= 60 {
        return None;
    }
    let millis = if fraction.is_empty() {
        0
    } else if fraction.len() <= 3 && fraction.bytes().all(|b| b.is_ascii_digit()) {
        fraction.parse::<u64>().ok()? * 10_u64.pow(3 - fraction.len() as u32)
    } else {
        return None;
    };
    Some(((hours * 60 + minutes) * 60 + seconds) * 1_000 + millis)
}

fn srt_timing(line: &str) -> Option<(u64, u64)> {
    let (start, rest) = line.split_once("-->")?;
    // WebVTT cue settings may follow the end time.
    let end = rest.split_whitespace().next()?;
    let start = subtitle_time(start)?;
    let end = subtitle_time(end)?;
    Some((start, end.max(start)))
}

/// Remove `<i>`-style markup and `{\an8}`-style override blocks, and decode the few entities
/// subtitle editors write.
fn strip_subtitle_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                // Only a tag-like run is removed; a lone `<` stays.
                let rest: String = chars
                    .clone()
                    .take_while(|&c| c != '>' && c != '<')
                    .collect();
                if chars.clone().nth(rest.chars().count()) == Some('>')
                    && rest
                        .chars()
                        .next()
                        .is_some_and(|first| first == '/' || first.is_ascii_alphabetic())
                {
                    for _ in 0..=rest.chars().count() {
                        chars.next();
                    }
                } else {
                    out.push(c);
                }
            }
            '{' => {
                let rest: String = chars.clone().take_while(|&c| c != '}').collect();
                if rest.starts_with('\\') && chars.clone().nth(rest.chars().count()) == Some('}') {
                    for _ in 0..=rest.chars().count() {
                        chars.next();
                    }
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

fn join_text_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    lines
        .into_iter()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// SRT (and the WebVTT cue layout): every cue is one line with its start and end. The lines of a
/// multi-line cue are joined with a space; markup is removed.
pub fn parse_srt(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    if input.len() > MAX_LYRIC_PAYLOAD_BYTES {
        return Err(LyricsParseError::PayloadTooLarge);
    }
    let text = normalized(input);
    let rows = text.lines().collect::<Vec<_>>();
    let mut lines = Vec::new();
    let mut index = 0;
    while index < rows.len() {
        let Some((start_ms, end_ms)) = srt_timing(rows[index]) else {
            index += 1;
            continue;
        };
        index += 1;
        let mut cue_rows = Vec::new();
        while index < rows.len() && !rows[index].trim().is_empty() {
            // A missing blank line before the next cue: its number and timing end this cue.
            if is_srt_timing_line(rows[index])
                || (index + 1 < rows.len()
                    && rows[index].trim().bytes().all(|b| b.is_ascii_digit())
                    && is_srt_timing_line(rows[index + 1]))
            {
                break;
            }
            cue_rows.push(strip_subtitle_markup(rows[index]));
            index += 1;
        }
        let joined = join_text_lines(cue_rows.iter().map(String::as_str));
        if !joined.is_empty() {
            lines.push(LyricLine {
                start_ms: Some(start_ms),
                end_ms: Some(end_ms),
                text: joined,
                ..LyricLine::default()
            });
        }
    }
    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());
    Ok(ParsedLyrics {
        format: LyricFormat::Srt,
        synced: !lines.is_empty(),
        lines,
        raw_text: input.to_owned(),
        raw_karaoke: None,
    })
}

// ---------------------------------------------------------------- ASS

struct AssEvent {
    start_ms: u64,
    end_ms: u64,
    style: String,
    text: String,
    words: Vec<LyricWord>,
}

/// One `Text` field: override blocks removed, `\N`/`\n`/`\h` turned into spaces, drawing
/// (`\p1`…) dropped, and the karaoke tags `\k` `\K` `\kf` `\ko` (centiseconds) turned into words.
fn parse_ass_text(raw: &str, event_start: u64, event_end: u64) -> (String, Vec<LyricWord>) {
    let mut words: Vec<LyricWord> = Vec::new();
    let mut plain = String::new();
    let mut karaoke = false;
    let mut cursor = event_start;
    let mut drawing = false;
    let mut rest = raw;
    let push_text = |text: &str, words: &mut Vec<LyricWord>, plain: &mut String, karaoke: bool| {
        if text.is_empty() {
            return;
        }
        if karaoke {
            if let Some(word) = words.last_mut() {
                word.text.push_str(text);
                return;
            }
        }
        plain.push_str(text);
    };
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('{') {
            let Some(close) = after.find('}') else {
                push_text(rest, &mut words, &mut plain, karaoke);
                break;
            };
            let block = &after[..close];
            rest = &after[close + 1..];
            for tag in block.split('\\').skip(1) {
                let tag = tag.trim();
                let lower = tag.to_ascii_lowercase();
                if let Some(value) = lower
                    .strip_prefix("kf")
                    .or_else(|| lower.strip_prefix("ko"))
                    .or_else(|| lower.strip_prefix('k'))
                {
                    if let Ok(centis) = value.trim().parse::<u64>() {
                        if !karaoke {
                            karaoke = true;
                            // Text before the first syllable belongs to the event start.
                            if !plain.is_empty() {
                                words.push(LyricWord {
                                    start_ms: event_start,
                                    end_ms: event_start,
                                    text: std::mem::take(&mut plain),
                                });
                            }
                        }
                        let end = cursor
                            .saturating_add(centis * 10)
                            .min(event_end.max(cursor));
                        words.push(LyricWord {
                            start_ms: cursor,
                            end_ms: end,
                            text: String::new(),
                        });
                        cursor = end;
                    }
                } else if let Some(value) = lower.strip_prefix('p') {
                    if let Ok(level) = value.trim().parse::<u32>() {
                        drawing = level > 0;
                    }
                }
            }
            continue;
        }
        let next = rest.find(['{', '\\']).unwrap_or(rest.len());
        let (chunk, tail) = if next == 0 {
            // A `\N`, `\n` or `\h` escape outside an override block.
            let mut chars = rest.chars();
            chars.next();
            match chars.next() {
                Some('N' | 'n' | 'h') => (" ", &rest[2..]),
                _ => ("\\", &rest[1..]),
            }
        } else {
            (&rest[..next], &rest[next..])
        };
        if !drawing {
            push_text(chunk, &mut words, &mut plain, karaoke);
        }
        rest = tail;
    }
    if karaoke {
        words.retain(|word| !word.text.is_empty() || word.end_ms > word.start_ms);
        // Collapse spaces the same way as the line text, so the words still join to the text.
        let text: String = words.iter().map(|word| word.text.as_str()).collect();
        if text.trim().is_empty() {
            return (String::new(), Vec::new());
        }
        trim_word_edges(&mut words);
        let text = words.iter().map(|word| word.text.as_str()).collect();
        (text, words)
    } else {
        (
            plain.split_whitespace().collect::<Vec<_>>().join(" "),
            Vec::new(),
        )
    }
}

/// Drop leading spaces of the first word and trailing spaces of the last.
fn trim_word_edges(words: &mut Vec<LyricWord>) {
    while words
        .first()
        .is_some_and(|word| word.text.trim().is_empty())
    {
        words.remove(0);
    }
    while words.last().is_some_and(|word| word.text.trim().is_empty()) {
        words.pop();
    }
    if let Some(first) = words.first_mut() {
        first.text = first.text.trim_start().to_owned();
    }
    if let Some(last) = words.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
}

/// `h:mm:ss.cc`.
fn ass_time(value: &str) -> Option<u64> {
    subtitle_time(value)
}

/// The ASS/SSA subset of ADR 0041: `[Events]` `Dialogue` lines, read through the section's
/// `Format:` line (`Start`, `End`, `Style`, `Text`); karaoke tags give word timing; all other
/// override tags are removed. Events with the same start and end (±50 ms) but another style are
/// the translation and romanization of the first, in file order.
pub fn parse_ass(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    if input.len() > MAX_LYRIC_PAYLOAD_BYTES {
        return Err(LyricsParseError::PayloadTooLarge);
    }
    let text = normalized(input);
    let mut in_events = false;
    let mut format: Option<Vec<String>> = None;
    let mut events = Vec::new();
    for row in text.lines() {
        let row = row.trim();
        if row.starts_with('[') && row.ends_with(']') {
            in_events = row.eq_ignore_ascii_case("[events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(fields) = row.strip_prefix("Format:") {
            format = Some(
                fields
                    .split(',')
                    .map(|field| field.trim().to_ascii_lowercase())
                    .collect(),
            );
            continue;
        }
        let Some(values) = row.strip_prefix("Dialogue:") else {
            continue;
        };
        // Without a Format line, the standard v4+ order.
        let fields = format.clone().unwrap_or_else(|| {
            [
                "layer", "start", "end", "style", "name", "marginl", "marginr", "marginv",
                "effect", "text",
            ]
            .iter()
            .map(|field| (*field).to_owned())
            .collect()
        });
        let Some(text_index) = fields.iter().position(|field| field == "text") else {
            continue;
        };
        if text_index + 1 != fields.len() {
            continue;
        }
        let values = values
            .trim_start()
            .splitn(fields.len(), ',')
            .collect::<Vec<_>>();
        if values.len() != fields.len() {
            continue;
        }
        let field = |name: &str| {
            fields
                .iter()
                .position(|field| field == name)
                .map(|index| values[index])
        };
        let (Some(start), Some(end)) = (
            field("start").and_then(ass_time),
            field("end").and_then(ass_time),
        ) else {
            continue;
        };
        let end = end.max(start);
        let (line_text, words) = parse_ass_text(values[text_index], start, end);
        if line_text.trim().is_empty() {
            continue;
        }
        events.push(AssEvent {
            start_ms: start,
            end_ms: end,
            style: field("style").unwrap_or_default().trim().to_owned(),
            text: line_text,
            words,
        });
    }
    events.sort_by_key(|event| event.start_ms);

    let mut lines: Vec<LyricLine> = Vec::new();
    let mut styles: Vec<String> = Vec::new();
    let mut roles: Vec<usize> = Vec::new();
    for event in events {
        let paired = lines.iter().enumerate().rev().find(|(index, line)| {
            line.start_ms
                .is_some_and(|s| s.abs_diff(event.start_ms) <= 50)
                && line.end_ms.is_some_and(|e| e.abs_diff(event.end_ms) <= 50)
                && styles[*index] != event.style
        });
        if let Some((index, _)) = paired {
            roles[index] += 1;
            let line = &mut lines[index];
            match roles[index] {
                1 if line.translation.is_none() => line.translation = Some(event.text),
                2 if line.romanization.is_none() => {
                    line.romanization = Some(event.text);
                    line.romanization_words = event.words;
                }
                _ => {}
            }
            continue;
        }
        styles.push(event.style);
        roles.push(0);
        lines.push(LyricLine {
            start_ms: Some(event.start_ms),
            end_ms: Some(event.end_ms),
            text: event.text,
            words: event.words,
            ..LyricLine::default()
        });
    }
    let lines = merge_same_timestamp_lines(lines);
    Ok(ParsedLyrics {
        format: LyricFormat::Ass,
        synced: !lines.is_empty(),
        lines,
        raw_text: input.to_owned(),
        raw_karaoke: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(start_ms: u64, end_ms: u64, text: &str) -> LyricWord {
        LyricWord {
            start_ms,
            end_ms,
            text: text.to_owned(),
        }
    }

    #[test]
    fn srt_cues_become_lines_with_start_and_end() {
        let srt = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>First</i> line\r\nsecond row\r\n\r\n2\r\n00:00:03.000 --> 00:00:04,000\r\n{\\an8}Top &amp; more\r\n";
        let parsed = parse_srt(srt).unwrap();
        assert_eq!(parsed.format, LyricFormat::Srt);
        assert!(parsed.synced);
        assert_eq!(parsed.lines.len(), 2);
        assert_eq!(parsed.lines[0].start_ms, Some(1_000));
        assert_eq!(parsed.lines[0].end_ms, Some(2_500));
        assert_eq!(parsed.lines[0].text, "First line second row");
        assert_eq!(parsed.lines[1].text, "Top & more");
        assert_eq!(parsed.lines[1].start_ms, Some(3_000));
    }

    #[test]
    fn srt_without_blank_lines_between_cues_still_splits_them() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\none\n2\n00:00:02,000 --> 00:00:03,000\ntwo\n";
        let parsed = parse_srt(srt).unwrap();
        let texts = parsed
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(texts, ["one", "two"]);
    }

    #[test]
    fn ass_dialogue_follows_the_format_line_and_strips_tags() {
        let ass = "[Script Info]\nTitle: x\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nComment: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,ignored\nDialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\pos(10,10)\\c&HFFFFFF&}Hello, world\\Nagain\nDialogue: 0,0:00:01.50,0:00:03.00,Trans,,0,0,0,,你好\n";
        let parsed = parse_ass(ass).unwrap();
        assert_eq!(parsed.format, LyricFormat::Ass);
        assert_eq!(parsed.lines.len(), 1);
        let line = &parsed.lines[0];
        assert_eq!(line.start_ms, Some(1_500));
        assert_eq!(line.end_ms, Some(3_000));
        assert_eq!(line.text, "Hello, world again");
        assert_eq!(line.translation.as_deref(), Some("你好"));
        assert!(line.words.is_empty());
    }

    #[test]
    fn ass_karaoke_tags_become_words() {
        let ass = "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:10.00,0:00:12.00,K,{\\k50}Ka{\\kf30}ra {\\K20}o{\\ko0}ke\n";
        let parsed = parse_ass(ass).unwrap();
        let line = &parsed.lines[0];
        assert_eq!(line.text, "Kara oke");
        assert_eq!(
            line.words,
            vec![
                word(10_000, 10_500, "Ka"),
                word(10_500, 10_800, "ra "),
                word(10_800, 11_000, "o"),
                word(11_000, 11_000, "ke"),
            ]
        );
        let joined: String = line.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(joined, line.text);
    }

    #[test]
    fn ass_drawing_commands_are_not_text() {
        let ass = "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:01.00,0:00:02.00,D,{\\p1}m 0 0 l 10 10{\\p0}Shown\n";
        assert_eq!(parse_ass(ass).unwrap().lines[0].text, "Shown");
    }

    #[test]
    fn detection_uses_the_content_not_the_name() {
        assert_eq!(detect_lyric_format("[00:01.00]x"), DetectedLyricFormat::Lrc);
        assert_eq!(
            detect_lyric_format("just words"),
            DetectedLyricFormat::Plain
        );
        assert_eq!(
            detect_lyric_format("1\n00:00:01,000 --> 00:00:02,000\nx"),
            DetectedLyricFormat::Srt
        );
        assert_eq!(
            detect_lyric_format("[Script Info]\n[Events]\n"),
            DetectedLyricFormat::Ass
        );
        assert_eq!(
            detect_lyric_format("[100,200](100,200,0)x"),
            DetectedLyricFormat::Yrc
        );
    }

    #[test]
    fn broken_subtitles_are_unusable_and_plain_text_is_kept() {
        assert_eq!(
            parse_any_lyrics("1\n00:00:xx,000 --> broken\ntext").unwrap(),
            LyricsContent::Unusable
        );
        assert_eq!(
            parse_any_lyrics("[Events]\nDialogue: nonsense").unwrap(),
            LyricsContent::Unusable
        );
        match parse_any_lyrics("plain words\nmore").unwrap() {
            LyricsContent::Parsed(parsed) => {
                assert_eq!(parsed.format, LyricFormat::Plain);
                assert_eq!(parsed.lines.len(), 2);
                assert!(!parsed.synced);
            }
            other => panic!("{other:?}"),
        }
        match parse_any_lyrics("1\n00:00:01,000 --> 00:00:02,000\nok\n").unwrap() {
            LyricsContent::Parsed(parsed) => assert!(parsed.synced),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn sidecar_preference_is_lrc_then_ass_then_srt() {
        assert_eq!(lyric_sidecar_rank("a.LRC"), Some(0));
        assert_eq!(lyric_sidecar_rank("a.ass"), Some(1));
        assert_eq!(lyric_sidecar_rank("a.Srt"), Some(2));
        assert_eq!(lyric_sidecar_rank("a.txt"), None);
        assert_eq!(lyric_sidecar_rank("noext"), None);
    }
}
