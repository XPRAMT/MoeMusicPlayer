//! The Kotlin side keeps its own copy of the audio extension list (`MediaIndexPlugin.kt`, used when the content
//! resolver gives no MIME type). The source of truth is `player-formats`; this test fails when they drift.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

#[test]
fn kotlin_audio_extensions_match_the_format_table() {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("android/src/main/java/com/moemusicplayer/mediaindex/MediaIndexPlugin.kt");
    let text = fs::read_to_string(&source).expect("read MediaIndexPlugin.kt");
    let start = text
        .find("AUDIO_EXTENSIONS = setOf(")
        .expect("AUDIO_EXTENSIONS in MediaIndexPlugin.kt");
    let body = &text[start..];
    let body = &body[body.find('(').unwrap() + 1..];
    let body = &body[..body.find(')').unwrap()];
    let kotlin: BTreeSet<String> = body
        .split(',')
        .map(|item| item.trim().trim_matches('"').to_owned())
        .filter(|item| !item.is_empty())
        .collect();
    let table: BTreeSet<String> = player_formats::android_candidate_extensions()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        kotlin, table,
        "update AUDIO_EXTENSIONS in MediaIndexPlugin.kt to match player-formats::android_candidate_extensions()"
    );
}
