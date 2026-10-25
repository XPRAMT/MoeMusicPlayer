//! Saving tags keeps the file's creation time on Windows (the library shows it as 新增時間; ADR 0013 寫入安全).
//! Every write path is covered: lofty (FLAC, MP3), the APE family (WavPack) and DSF. Only fixture copies in a temp
//! folder are edited.
#![cfg(windows)]

use std::fs;
use std::os::windows::fs::FileTimesExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use player_dsd::testing::build_dsf;
use player_tags::{read_details, write_edit, TagEdit};

fn temp_dir() -> PathBuf {
    let directory = std::env::temp_dir().join(format!("moe-tags-created-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    directory
}

fn manifest(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// 2001-02-03 04:05:06.7 UTC: far from now, so a fresh creation time cannot pass for it.
fn old_time() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(981_173_106_700)
}

fn set_created(path: &Path, created: SystemTime) {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_created(created))
        .unwrap();
}

fn created(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().created().unwrap()
}

fn assert_save_keeps_creation_time(path: &Path) {
    set_created(path, old_time());
    assert_eq!(
        created(path),
        old_time(),
        "fixture copy took the old creation time"
    );
    let modified_before = fs::metadata(path).unwrap().modified().unwrap();
    let mut edit = TagEdit {
        fields: read_details(path).unwrap().fields,
        ..TagEdit::default()
    };
    edit.fields.title = "新增時間不變".to_owned();
    let details = write_edit(path, &edit).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(details.fields.title, "新增時間不變");
    assert_eq!(
        created(path),
        old_time(),
        "{}: creation time changed",
        path.display()
    );
    // The details panel reads the same value.
    let old_ms = old_time().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    assert_eq!(details.created_unix_ms, Some(old_ms));
    // mtime still moves to the save (ADR 0013 D2).
    assert!(fs::metadata(path).unwrap().modified().unwrap() >= modified_before);
    let temp = path.with_file_name(format!(
        ".moetmp-{}",
        path.file_name().unwrap().to_string_lossy()
    ));
    assert!(!temp.exists(), "temp file left behind");
}

#[test]
fn saving_tags_keeps_the_creation_time_on_every_write_path() {
    let directory = temp_dir();
    let copies = [
        (
            "../player-audio-windows/tests/fixtures/seek-tone.flac",
            "tone.flac",
        ),
        (
            "../player-audio-windows/tests/fixtures/seek-tone.mp3",
            "tone.mp3",
        ),
        ("../player-wavpack/tests/fixtures/s16.wv", "tone.wv"),
    ];
    for (source, name) in copies {
        let path = directory.join(name);
        fs::copy(manifest(source), &path).unwrap();
        assert_save_keeps_creation_time(&path);
    }
    let dsf = directory.join("tone.dsf");
    fs::write(
        &dsf,
        build_dsf(2_822_400, 8_000, 4096, true, &[vec![0x69; 1000]]),
    )
    .unwrap();
    assert_save_keeps_creation_time(&dsf);
    let _ = fs::remove_dir_all(directory);
}
