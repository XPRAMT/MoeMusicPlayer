//! 「導入使用中歌詞」 with word timing (ADR 0041): the details panel writes enhanced LRC into the embedded
//! lyrics; saving and reading the file back must give the same words, with timings within 10 ms.

use std::path::PathBuf;

use player_core::{parse_any_lyrics, LyricWord, LyricsContent};
use player_opus::testing::write_opus;
use player_tags::{read_details, write_edit, TagEdit};

/// What `lyricsToText` (src/lib/lyrics-import.js) writes for these words (tests/lyrics-import.test.mjs checks
/// the same text).
const IMPORTED: &str = "[00:01.00]<00:01.00>Hello <00:01.50>world<00:02.25>\n[00:01.00]你好世界\n[00:03.00]<00:03.00>a <00:03.20><00:03.50>b<00:03.90>\n[00:05.00]plain line";

#[test]
fn imported_word_timed_lyrics_survive_save_and_reread() {
    let directory: PathBuf =
        std::env::temp_dir().join(format!("moe-enhanced-lrc-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("track.opus");
    std::fs::write(&path, write_opus(48_000, 2, 1_000.0, 0, 0)).unwrap();

    let details = read_details(&path).unwrap();
    let mut edit = TagEdit {
        fields: details.fields.clone(),
        ..TagEdit::default()
    };
    edit.fields.lyrics = IMPORTED.to_owned();
    write_edit(&path, &edit).unwrap();
    let reread = read_details(&path).unwrap().fields.lyrics;
    assert_eq!(reread, IMPORTED);

    let LyricsContent::Parsed(parsed) = parse_any_lyrics(&reread).unwrap() else {
        panic!("enhanced LRC must parse");
    };
    assert!(parsed.synced);
    let expected: [&[(u64, u64, &str)]; 3] = [
        &[(1000, 1500, "Hello "), (1500, 2253, "world")],
        &[(3000, 3200, "a "), (3500, 3900, "b")],
        &[],
    ];
    assert_eq!(parsed.lines.len(), 3);
    for (line, words) in parsed.lines.iter().zip(expected) {
        assert_eq!(line.words.len(), words.len(), "{line:?}");
        for (
            LyricWord {
                start_ms,
                end_ms,
                text,
            },
            (s, e, t),
        ) in line.words.iter().zip(words)
        {
            assert_eq!(text, t);
            assert!(
                start_ms.abs_diff(*s) <= 10 && end_ms.abs_diff(*e) <= 10,
                "{start_ms}-{end_ms} vs {s}-{e}"
            );
        }
    }
    assert_eq!(parsed.lines[0].text, "Hello world");
    assert_eq!(parsed.lines[0].translation.as_deref(), Some("你好世界"));
    assert_eq!(parsed.lines[2].text, "plain line");
    std::fs::remove_dir_all(&directory).ok();
}
