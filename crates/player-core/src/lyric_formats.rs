//! Subtitle formats used as lyrics (SRT, the ASS subset of ADR 0041) and content-based format
//! detection shared by every lyric text entry point (embedded tags, sidecars, paste/import).
//! The parsers are written for this project; no subtitle library is used.

use crate::lyric_roles::{contrast_role, fold_extra_rows, role_from_style, ExtraRow};
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
    Qrc,
    Lrc,
    Plain,
    /// A single hex or Base64 blob: encrypted QRC, which is never decoded.
    Encrypted,
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
/// more than one for the same song (ADR 0041): `.lrc`, `.qrc`, `.ass`, then `.srt`.
pub const LYRIC_SIDECAR_EXTENSIONS: [&str; 4] = ["lrc", "qrc", "ass", "srt"];

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

/// Classify a lyric text: ASS markers, then SRT cue timing (`-->`), then yrc, then QRC, then LRC
/// timestamps, else plain text.
pub fn detect_lyric_format(input: &str) -> DetectedLyricFormat {
    let text = normalized(input);
    if looks_like_encrypted_blob(&text) {
        DetectedLyricFormat::Encrypted
    } else if looks_like_ass(&text) {
        DetectedLyricFormat::Ass
    } else if looks_like_srt(&text) {
        DetectedLyricFormat::Srt
    } else if looks_like_yrc(&text) {
        DetectedLyricFormat::Yrc
    } else if looks_like_qrc(&text) {
        DetectedLyricFormat::Qrc
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
        DetectedLyricFormat::Qrc => parse_qrc(input)?,
        DetectedLyricFormat::Encrypted => return Ok(LyricsContent::Unusable),
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
        // Rows of one cue are a wrapped line unless a row is in a clearly different script from
        // the first (romaji under Japanese, Chinese under English...): then the rows from it on
        // are extra rows, each judged by content.
        let rows = cue_rows
            .iter()
            .map(|row| row.trim())
            .filter(|row| !row.is_empty())
            .collect::<Vec<_>>();
        let split = rows
            .iter()
            .skip(1)
            .position(|row| contrast_role(rows[0], row).is_some())
            .map(|index| index + 1);
        let (main_rows, extra_rows) = rows.split_at(split.unwrap_or(rows.len()));
        let joined = join_text_lines(main_rows.iter().copied());
        if !joined.is_empty() {
            let mut line = LyricLine {
                start_ms: Some(start_ms),
                end_ms: Some(end_ms),
                text: joined,
                ..LyricLine::default()
            };
            fold_extra_rows(
                &mut line,
                extra_rows
                    .iter()
                    .map(|row| ExtraRow {
                        line: LyricLine::timed(start_ms, *row),
                        hint: None,
                    })
                    .collect(),
            );
            lines.push(line);
        }
    }
    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());
    // Separate cues with the same start are one line with extra rows (paired cues).
    let lines = merge_same_timestamp_lines(lines);
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

    // Events at the same time (±50 ms start and end) in another style are the line's extra rows;
    // an explicit style name (roma…, trans…, 中文…) gives the role, otherwise the content does.
    let mut groups: Vec<(LyricLine, String, Vec<ExtraRow>)> = Vec::new();
    for event in events {
        let paired = groups.iter_mut().rev().find(|(line, style, _)| {
            line.start_ms
                .is_some_and(|s| s.abs_diff(event.start_ms) <= 50)
                && line.end_ms.is_some_and(|e| e.abs_diff(event.end_ms) <= 50)
                && *style != event.style
        });
        let line = LyricLine {
            start_ms: Some(event.start_ms),
            end_ms: Some(event.end_ms),
            text: event.text,
            words: event.words,
            ..LyricLine::default()
        };
        if let Some((_, _, extras)) = paired {
            extras.push(ExtraRow {
                hint: role_from_style(&event.style),
                line,
            });
            continue;
        }
        groups.push((line, event.style, Vec::new()));
    }
    let lines = groups
        .into_iter()
        .map(|(mut primary, _, extras)| {
            fold_extra_rows(&mut primary, extras);
            primary
        })
        .collect::<Vec<_>>();
    let lines = merge_same_timestamp_lines(lines);
    Ok(ParsedLyrics {
        format: LyricFormat::Ass,
        synced: !lines.is_empty(),
        lines,
        raw_text: input.to_owned(),
        raw_karaoke: None,
    })
}

// ---------------------------------------------------------------- QRC (plain text)

/// Decode the XML entities a QRC wrapper uses (`&lt;`, `&amp;`, `&#10;`, `&#x4e2d;`, ...). An
/// entity it does not know is kept as written.
fn xml_unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let decoded = tail.find(';').filter(|end| *end <= 10).and_then(|end| {
            let name = &tail[1..end];
            let character = match name {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => name.strip_prefix('#').and_then(|number| {
                    let code = match number.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => number.parse::<u32>().ok()?,
                    };
                    char::from_u32(code)
                }),
            }?;
            Some((character, end + 1))
        });
        match decoded {
            Some((character, consumed)) => {
                out.push(character);
                rest = &tail[consumed..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The lyric text of a QRC document: the plain text itself, or the decoded `LyricContent`
/// attribute of the `<Lyric_1 ...>` wrapper QQ Music writes.
fn qrc_body(input: &str) -> String {
    const ATTRIBUTE: &str = "LyricContent=\"";
    let trimmed = input.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('<') {
        if let Some(at) = trimmed.find(ATTRIBUTE) {
            let rest = &trimmed[at + ATTRIBUTE.len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            return xml_unescape(&rest[..end]);
        }
    }
    input.to_owned()
}

/// `[start,duration]` at the start of a line, then the rest of the line.
fn qrc_line_timing(line: &str) -> Option<(u64, u64, &str)> {
    let tagged = line.strip_prefix('[')?;
    let end = tagged.find(']')?;
    let mut parts = tagged[..end].split(',');
    let start = parts.next()?.trim().parse::<u64>().ok()?;
    let duration = parts.next()?.trim().parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((start, duration, &tagged[end + 1..]))
}

/// `(start,duration)` at the start of `text`: the two numbers and the bytes it covers.
fn qrc_marker(text: &str) -> Option<(u64, u64, usize)> {
    let inner = text.strip_prefix('(')?;
    let end = inner.find(')')?;
    let (start, duration) = inner[..end].split_once(',')?;
    let digits = |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    if !digits(start) || !digits(duration) {
        return None;
    }
    Some((start.parse().ok()?, duration.parse().ok()?, end + 2))
}

fn qrc_has_marker(text: &str) -> bool {
    text.char_indices()
        .any(|(at, character)| character == '(' && qrc_marker(&text[at..]).is_some())
}

fn looks_like_qrc(text: &str) -> bool {
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('<') && trimmed.contains("LyricContent=") {
        return true;
    }
    text.lines().any(|line| {
        qrc_line_timing(line.trim())
            .is_some_and(|(_, _, rest)| !rest.starts_with('(') && qrc_has_marker(rest))
    })
}

/// Words of one QRC line as `text(start,duration)` pairs. A `(0,0)` marker carries no timing:
/// the text before it (usually a space) joins the previous word, or the next one at the start.
fn qrc_line_words(rest: &str) -> Vec<(String, u64, u64)> {
    let mut words: Vec<(String, u64, u64)> = Vec::new();
    let mut text = String::new();
    let mut prefix = String::new();
    let mut at = 0;
    while at < rest.len() {
        let character = rest[at..].chars().next().unwrap_or('\0');
        if character == '(' {
            if let Some((start, duration, consumed)) = qrc_marker(&rest[at..]) {
                if start == 0 && duration == 0 {
                    match words.last_mut() {
                        Some(last) => last.0.push_str(&text),
                        None => prefix.push_str(&text),
                    }
                } else {
                    let mut word = std::mem::take(&mut prefix);
                    word.push_str(&text);
                    if !word.is_empty() {
                        words.push((word, start, start.saturating_add(duration)));
                    }
                }
                text.clear();
                at += consumed;
                continue;
            }
        }
        text.push(character);
        at += character.len_utf8();
    }
    if let Some(last) = words.last_mut() {
        last.0.push_str(&text);
    }
    words
}

/// Read plain-text QRC (ADR 0041 D6): `[lineStart,lineDuration]` then `text(wordStart,wordDuration)`
/// words, in absolute milliseconds. Word starts relative to the line start are accepted when
/// they only fit that reading. The encrypted form is not decoded; it yields no lines.
pub fn parse_qrc(input: &str) -> Result<ParsedLyrics, LyricsParseError> {
    if input.len() > MAX_LYRIC_PAYLOAD_BYTES {
        return Err(LyricsParseError::PayloadTooLarge);
    }
    const SLACK_MS: u64 = 100;
    let body = qrc_body(input);
    let mut lines = Vec::new();
    for physical in body.lines() {
        let line = physical
            .trim_start_matches('\u{feff}')
            .trim_end_matches('\r')
            .trim_start();
        let Some((line_start, line_duration, rest)) = qrc_line_timing(line) else {
            continue;
        };
        let mut words = qrc_line_words(rest);
        if words.is_empty() {
            continue;
        }
        let line_end = line_start.saturating_add(line_duration);
        let fits_absolute = words
            .iter()
            .all(|word| word.1 + SLACK_MS >= line_start && word.1 <= line_end + SLACK_MS);
        let fits_relative = words.iter().all(|word| word.1 <= line_duration + SLACK_MS);
        if line_start > 0 && !fits_absolute && fits_relative {
            for word in &mut words {
                word.1 = word.1.saturating_add(line_start);
                word.2 = word.2.saturating_add(line_start);
            }
        }
        let words: Vec<LyricWord> = words
            .into_iter()
            .map(|(text, start_ms, end_ms)| LyricWord {
                start_ms,
                end_ms,
                text,
            })
            .collect();
        let text: String = words.iter().map(|word| word.text.as_str()).collect();
        if text.trim().is_empty() {
            continue;
        }
        let end_ms = words
            .iter()
            .map(|word| word.end_ms)
            .fold(line_end, u64::max);
        lines.push(LyricLine {
            start_ms: Some(line_start),
            end_ms: Some(end_ms),
            text,
            words,
            ..LyricLine::default()
        });
    }
    lines.sort_by_key(|line| line.start_ms.unwrap_or_default());
    Ok(ParsedLyrics {
        format: LyricFormat::Qrc,
        synced: !lines.is_empty(),
        lines,
        raw_text: body,
        raw_karaoke: None,
    })
}

/// A single unbroken hex or Base64 blob: the encrypted form of QRC, which is never decoded.
fn looks_like_encrypted_blob(text: &str) -> bool {
    let blob = text.trim_start_matches('\u{feff}').trim();
    if blob.chars().any(char::is_whitespace) {
        return false;
    }
    let hex = blob.len() >= 32
        && blob.len().is_multiple_of(2)
        && blob.bytes().all(|byte| byte.is_ascii_hexdigit());
    let base64 = blob.len() >= 64
        && blob
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
        && blob.bytes().any(|byte| byte.is_ascii_digit())
        && blob.bytes().any(|byte| byte.is_ascii_uppercase())
        && blob.bytes().any(|byte| byte.is_ascii_lowercase());
    hex || base64
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

    fn line_ends(text: &str) -> Vec<Option<u64>> {
        match parse_any_lyrics(text).unwrap() {
            LyricsContent::Parsed(parsed) => parsed.lines.iter().map(|line| line.end_ms).collect(),
            other => panic!("{other:?}"),
        }
    }

    /// ADR 0041 D10: every format that carries a line end yields it (embedded lyrics go through the same parsers).
    #[test]
    fn every_format_reads_its_line_end() {
        // SRT: the cue end.
        assert_eq!(
            line_ends(
                "1\n00:00:01,000 --> 00:00:03,500\nOne\n\n2\n00:00:05,000 --> 00:00:06,000\nTwo\n"
            ),
            [Some(3_500), Some(6_000)]
        );
        // ASS: the event end.
        assert_eq!(
            line_ends(
                "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:01.00,0:00:04.20,A,Hi\n"
            ),
            [Some(4_200)]
        );
        // yrc: [start,duration] -> start + duration.
        assert_eq!(
            line_ends("[1000,2000](1000,500,0)a(1500,300,0)b\n"),
            [Some(3_000)]
        );
        // Plain QRC: [start,duration] -> start + duration.
        assert_eq!(
            line_ends("[1000,2000]a(1000,500)b(1500,300)\n"),
            [Some(3_000)]
        );
        // Enhanced LRC: the closing stamp; without one, no end; plain LRC: no end.
        assert_eq!(
            line_ends(
                "[00:01.00]<00:01.00>a<00:01.50>b<00:02.50>\n[00:04.00]<00:04.00>c<00:04.50>d\n[00:06.00]plain\n"
            ),
            [Some(2_500), None, None]
        );
        // The exported form of a line with an end but no words reads back with its end.
        assert_eq!(
            line_ends("[00:01.00]<00:01.00>Hello world<00:03.25>\n"),
            [Some(3_250)]
        );
    }

    #[test]
    fn merged_extra_rows_keep_the_main_line_end() {
        // SRT paired cues with other ends: the first cue's end stays.
        let srt = "1\n00:00:01,000 --> 00:00:03,000\n사랑해\n\n2\n00:00:01,000 --> 00:00:02,000\n我愛你\n";
        let line = &parse_srt(srt).unwrap().lines[0];
        assert_eq!(line.end_ms, Some(3_000));
        assert_eq!(line.translation.as_deref(), Some("我愛你"));
        // Enhanced LRC with a same-time translation row.
        let lrc = "[00:01.00]<00:01.00>君の声<00:02.00>\n[00:01.00]你的聲音\n";
        let parsed = parse_lrc(lrc).unwrap();
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].end_ms, Some(2_000));
        assert_eq!(parsed.lines[0].translation.as_deref(), Some("你的聲音"));
        // ASS events of another style at the same time.
        let ass = "[Events]\nFormat: Start, End, Style, Text\nDialogue: 0:00:01.00,0:00:03.00,JP,君の声\nDialogue: 0:00:01.00,0:00:03.02,ZH,你的聲音\n";
        let line = &parse_ass(ass).unwrap().lines[0];
        assert_eq!(line.end_ms, Some(3_000));
        assert_eq!(line.translation.as_deref(), Some("你的聲音"));
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
    fn srt_rows_in_another_script_are_extra_rows_and_same_script_rows_wrap() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\n君の声が\n聞こえる\nkimi no koe ga kikoeru\n聽見你的聲音\n\n2\n00:00:04,000 --> 00:00:05,000\nI walk alone\non the road\n";
        let parsed = parse_srt(srt).unwrap();
        let first = &parsed.lines[0];
        assert_eq!(first.text, "君の声が 聞こえる");
        assert_eq!(
            first.romanization.as_deref(),
            Some("kimi no koe ga kikoeru")
        );
        assert_eq!(first.translation.as_deref(), Some("聽見你的聲音"));
        assert_eq!(parsed.lines[1].text, "I walk alone on the road");
        assert!(parsed.lines[1].translation.is_none());
    }

    #[test]
    fn srt_paired_cues_are_classified_by_content() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\n사랑해\n\n2\n00:00:01,000 --> 00:00:02,000\n我愛你\n\n3\n00:00:01,000 --> 00:00:02,000\nsaranghae\n";
        let line = &parse_srt(srt).unwrap().lines[0];
        assert_eq!(line.text, "사랑해");
        assert_eq!(line.romanization.as_deref(), Some("saranghae"));
        assert_eq!(line.translation.as_deref(), Some("我愛你"));
    }

    #[test]
    fn ass_style_names_give_roles_and_content_decides_otherwise() {
        let head = "[Events]\nFormat: Start, End, Style, Text\n";
        // Content: the romaji event comes before the Chinese one.
        let ass = format!("{head}Dialogue: 0:00:01.00,0:00:02.00,JP,君の声\nDialogue: 0:00:01.00,0:00:02.00,A,{{\\k50}}kimi {{\\k50}}no koe\nDialogue: 0:00:01.00,0:00:02.00,B,你的聲音\n");
        let line = &parse_ass(&ass).unwrap().lines[0];
        assert_eq!(line.romanization.as_deref(), Some("kimi no koe"));
        assert_eq!(line.romanization_words.len(), 2);
        assert_eq!(line.translation.as_deref(), Some("你的聲音"));
        // Explicit style names win.
        let ass = format!("{head}Dialogue: 0:00:01.00,0:00:02.00,JP,君の声\nDialogue: 0:00:01.00,0:00:02.00,Romaji,x y\nDialogue: 0:00:01.00,0:00:02.00,中文,你的聲音\n");
        let line = &parse_ass(&ass).unwrap().lines[0];
        assert_eq!(line.romanization.as_deref(), Some("x y"));
        assert_eq!(line.translation.as_deref(), Some("你的聲音"));
        let ass = format!("{head}Dialogue: 0:00:01.00,0:00:02.00,JP,君の声\nDialogue: 0:00:01.00,0:00:02.00,Translation,kimi no koe\n");
        let line = &parse_ass(&ass).unwrap().lines[0];
        assert_eq!(line.translation.as_deref(), Some("kimi no koe"));
        assert!(line.romanization.is_none());
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
    fn sidecar_preference_is_lrc_then_qrc_then_ass_then_srt() {
        assert_eq!(lyric_sidecar_rank("a.LRC"), Some(0));
        assert_eq!(lyric_sidecar_rank("a.Qrc"), Some(1));
        assert_eq!(lyric_sidecar_rank("a.ass"), Some(2));
        assert_eq!(lyric_sidecar_rank("a.Srt"), Some(3));
        assert_eq!(lyric_sidecar_rank("a.txt"), None);
        assert_eq!(lyric_sidecar_rank("noext"), None);
    }

    const AMLL_SAMPLE: &str = include_str!("../../../tests/fixtures/amll-qq-sample.qrc");

    #[test]
    fn qrc_sample_reads_absolute_words_and_space_placeholders() {
        let parsed = parse_qrc(AMLL_SAMPLE).unwrap();
        assert_eq!(parsed.format, LyricFormat::Qrc);
        assert!(parsed.synced);
        assert_eq!(parsed.lines.len(), 6);
        let first = &parsed.lines[0];
        assert_eq!(first.start_ms, Some(17669));
        assert_eq!(first.text, "除刺的玫瑰");
        assert_eq!(first.words[0], word(17669, 17911, "除"));
        assert_eq!(first.words[4], word(18759, 19237, "瑰"));
        assert_eq!(first.end_ms, Some(19237));
        // `柜(…) (0,0)`: the space joins the previous word.
        let third = &parsed.lines[2];
        assert_eq!(third.words.last().unwrap().text, "柜 ");
        assert_eq!(third.text, "我踏实放在床头柜 ");
        assert_eq!(third.words.len(), 8);
    }

    #[test]
    fn qrc_relative_word_starts_are_moved_to_the_line() {
        let parsed = parse_qrc("[10000,900]a(0,300)b(300,300)c(600,300)\n").unwrap();
        let words = &parsed.lines[0].words;
        assert_eq!(words[0], word(10000, 10300, "a"));
        assert_eq!(words[2], word(10600, 10900, "c"));
        let absolute = parse_qrc("[10000,900]a(10000,300)b(10300,300)\n").unwrap();
        assert_eq!(absolute.lines[0].words[1], word(10300, 10600, "b"));
    }

    #[test]
    fn qrc_leading_placeholders_prefix_the_next_word() {
        let parsed = parse_qrc("[1000,500] (0,0)hi(1000,200) there(1200,300)\n").unwrap();
        let line = &parsed.lines[0];
        assert_eq!(line.words[0].text, " hi");
        assert_eq!(line.words[1].text, " there");
        assert_eq!(line.text, " hi there");
    }

    #[test]
    fn qrc_keeps_parentheses_that_are_not_timings_and_skips_metadata() {
        let text =
            "[ti:Song]\r\n[ar:Moe]\r\n[1000,600](oh)(1000,300)yes(1300,300)\r\n[2000,100]\r\n\r\n";
        let parsed = parse_qrc(text).unwrap();
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].words[0].text, "(oh)");
        assert_eq!(parsed.lines[0].text, "(oh)yes");
    }

    #[test]
    fn qrc_xml_wrapper_is_unescaped() {
        let wrapped = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<QrcInfos><LyricInfo LyricCount=\"1\"><Lyric_1 LyricType=\"1\" LyricContent=\"[ti:A &amp; B]&#10;[500,400]你(500,200)好(700,200)&#10;\"/></LyricInfo></QrcInfos>";
        assert_eq!(detect_lyric_format(wrapped), DetectedLyricFormat::Qrc);
        let parsed = parse_qrc(wrapped).unwrap();
        assert_eq!(parsed.lines.len(), 1);
        assert_eq!(parsed.lines[0].text, "你好");
        assert_eq!(parsed.lines[0].words[1], word(700, 900, "好"));
    }

    #[test]
    fn qrc_empty_and_huge_inputs() {
        assert!(parse_qrc("").unwrap().lines.is_empty());
        assert!(!parse_qrc("").unwrap().synced);
        assert!(matches!(
            parse_qrc(&"x".repeat(MAX_LYRIC_PAYLOAD_BYTES + 1)),
            Err(LyricsParseError::PayloadTooLarge)
        ));
        assert!(matches!(
            parse_any_lyrics(&"x".repeat(MAX_LYRIC_PAYLOAD_BYTES + 1)),
            Err(LyricsParseError::PayloadTooLarge)
        ));
    }

    #[test]
    fn qrc_is_detected_by_content_and_encrypted_blobs_are_unusable() {
        assert_eq!(detect_lyric_format(AMLL_SAMPLE), DetectedLyricFormat::Qrc);
        match parse_any_lyrics(AMLL_SAMPLE).unwrap() {
            LyricsContent::Parsed(parsed) => assert_eq!(parsed.format, LyricFormat::Qrc),
            other => panic!("{other:?}"),
        }
        // yrc keeps priority for its own `(start,duration,flag)` form.
        assert_eq!(
            detect_lyric_format("[100,200](100,200,0)x"),
            DetectedLyricFormat::Yrc
        );
        let hex = "A1B2C3D4E5F60718293A4B5C6D7E8F90".repeat(4);
        assert_eq!(detect_lyric_format(&hex), DetectedLyricFormat::Encrypted);
        assert_eq!(parse_any_lyrics(&hex).unwrap(), LyricsContent::Unusable);
        let base64 = "Qm9ndXNFbmNyeXB0ZWRQYXlsb2FkMDEyMzQ1Njc4OWFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6";
        assert_eq!(parse_any_lyrics(base64).unwrap(), LyricsContent::Unusable);
        let wrapped_blob = "<Lyric_1 LyricType=\"1\" LyricContent=\"ABCDEF0123456789\"/>";
        assert_eq!(
            parse_any_lyrics(wrapped_blob).unwrap(),
            LyricsContent::Unusable
        );
        // Ordinary one-word plain text is still plain.
        assert_eq!(detect_lyric_format("hello"), DetectedLyricFormat::Plain);
    }
}
