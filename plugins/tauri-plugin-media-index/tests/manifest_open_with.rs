//! The app is a 「開啟方式」 target for audio files (ADR 0054): `AndroidManifest.xml` declares `ACTION_VIEW` filters for the
//! types and the file names the format table knows, and `ExternalOpen.kt` names the extension of a type for a copy it
//! makes. The source of truth is `player-formats`; these tests fail when the copies drift.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn read(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .replace("\r\n", "\n")
}

/// Every `android:<name>="..."` value of the `<data>` elements of the manifest's VIEW intent filters.
fn view_filter_data(manifest: &str, name: &str) -> BTreeSet<String> {
    let needle = format!("android:{name}=\"");
    let mut values = BTreeSet::new();
    for filter in manifest.split("<intent-filter>").skip(1) {
        let Some(filter) = filter.split("</intent-filter>").next() else {
            continue;
        };
        if !filter.contains("android.intent.action.VIEW") {
            continue;
        }
        for line in filter.lines().filter(|line| line.trim_start().starts_with("<data ")) {
            if let Some(start) = line.find(&needle) {
                let rest = &line[start + needle.len()..];
                values.insert(rest[..rest.find('"').expect("closing quote")].to_owned());
            }
        }
    }
    values
}

fn manifest() -> String {
    read("../../src-tauri/gen/android/app/src/main/AndroidManifest.xml")
}

#[test]
fn the_manifest_declares_the_format_tables_mime_types() {
    let declared = view_filter_data(&manifest(), "mimeType");
    let mut expected: BTreeSet<String> = player_formats::android_open_mime_types()
        .into_iter()
        .map(str::to_owned)
        .collect();
    // A wildcard for any audio type, and any type for the filters that match by file name.
    expected.insert("audio/*".to_owned());
    expected.insert("*/*".to_owned());
    assert_eq!(
        declared, expected,
        "update the VIEW intent filters of AndroidManifest.xml to match player-formats::android_open_mime_types()"
    );
}

/// `.*\\.[fF][lL][aA][cC]` and the variants for names with more dots: Android's glob does not backtrack over a dot.
fn expected_path_patterns() -> BTreeSet<String> {
    let mut patterns = BTreeSet::new();
    for extension in player_formats::android_library_extensions() {
        let cased: String = extension
            .chars()
            .map(|char| {
                if char.is_ascii_alphabetic() {
                    format!("[{}{}]", char.to_ascii_lowercase(), char.to_ascii_uppercase())
                } else {
                    char.to_string()
                }
            })
            .collect();
        for dots in 0..4 {
            patterns.insert(format!(".*{}\\\\.{cased}", "\\\\..*".repeat(dots)));
        }
    }
    patterns
}

#[test]
fn the_manifest_declares_a_file_name_pattern_for_every_library_extension() {
    let declared = view_filter_data(&manifest(), "pathPattern");
    assert_eq!(
        declared,
        expected_path_patterns(),
        "update the pathPattern entries of AndroidManifest.xml to match player-formats::android_library_extensions()"
    );
}

#[test]
fn the_view_filters_are_browsable_defaults_for_content_and_file() {
    let manifest = manifest();
    let views: Vec<&str> = manifest
        .split("<intent-filter>")
        .skip(1)
        .filter_map(|filter| filter.split("</intent-filter>").next())
        .filter(|filter| filter.contains("android.intent.action.VIEW"))
        .collect();
    assert_eq!(views.len(), 3, "typed, untyped by name, and any type by name");
    for filter in views {
        assert!(filter.contains("android.intent.category.DEFAULT"));
        assert!(filter.contains("android.intent.category.BROWSABLE"));
        assert!(filter.contains("android:scheme=\"content\""));
        assert!(filter.contains("android:scheme=\"file\""));
    }
    // No share-sheet entry (the design opens a file, it does not receive one).
    assert!(!manifest.contains("android.intent.action.SEND"));
    // One running instance takes a new request (`onNewIntent`) without restarting the playback service.
    assert!(manifest.contains("android:launchMode=\"singleTask\""));
}

#[test]
fn kotlin_names_the_same_extension_for_a_type_as_the_format_table() {
    let source = read("android/src/main/java/com/moemusicplayer/mediaindex/ExternalOpen.kt");
    let start = source
        .find("MIME_EXTENSIONS: Map<String, String> = mapOf(")
        .expect("MIME_EXTENSIONS in ExternalOpen.kt");
    let body = &source[start..];
    let body = &body[body.find("mapOf(").unwrap() + "mapOf(".len()..];
    let body = &body[..body.find("\n    )").expect("end of MIME_EXTENSIONS")];
    let mut kotlin = BTreeSet::new();
    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (mime, extension) = line.trim_end_matches(',').split_once(" to ").expect("`mime` to `extension`");
        kotlin.insert((mime.trim_matches('"').to_owned(), extension.trim_matches('"').to_owned()));
    }
    let table: BTreeSet<(String, String)> = player_formats::android_open_mime_types()
        .into_iter()
        .map(|mime| {
            (
                mime.to_owned(),
                player_formats::extension_for_mime(mime)
                    .expect("every declared type has an extension")
                    .to_owned(),
            )
        })
        .collect();
    assert_eq!(
        kotlin, table,
        "update MIME_EXTENSIONS in ExternalOpen.kt to match player-formats::extension_for_mime()"
    );
}
