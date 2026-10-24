//! Real-path library scanning on Android under All files access (ADR 0014). Folder sources and
//! playlist media are plain directories and files, like desktop folder sources, with no SAF document tree and no
//! permission other than All files access. The scan does not query MediaStore today (it may: All files access is enough
//! to query it). Item IDs and locator keys are `player_core::unix_path_key`.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use player_core::{
    FileFingerprint, LibraryRoot, MediaLocator, MediaScanProgress, MediaScanProgressUnit,
    MediaSourceError, MediaTrackRecord, SourceId, SourceScan, SourceScanState, SyncCancellation,
    TrackIdentity, unix_path_key,
};

/// Whether Android's library scanner accepts the path. The set lives in `player-formats` (`.mp4` stays excluded
/// like the desktop scanner: video).
pub fn is_supported_audio_path(path: &Path) -> bool {
    player_formats::is_android_library_audio(path)
}

/// `/storage/emulated/<user>` or `/storage/<volume>`: its `Android/` app-data tree is skipped.
fn is_storage_volume_root(path: &Path) -> bool {
    let mut components = path.components().skip(1);
    match (
        components.next().and_then(|item| item.as_os_str().to_str()),
        components.next().and_then(|item| item.as_os_str().to_str()),
        components.next().and_then(|item| item.as_os_str().to_str()),
        components.next(),
    ) {
        (Some("storage"), Some("emulated"), Some(user), None) => {
            user.chars().all(|value| value.is_ascii_digit())
        }
        (Some("storage"), Some(volume), None, None) => volume != "emulated" && volume != "self",
        _ => false,
    }
}

fn utc_ms(time: SystemTime) -> Option<i64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn record(source_id: SourceId, item_id: String, path: PathBuf, metadata: &fs::Metadata) -> MediaTrackRecord {
    let locator_key = unix_path_key(&path);
    MediaTrackRecord {
        identity: TrackIdentity {
            source_id,
            source_item_id: item_id,
            locator_key: Some(locator_key),
        },
        locator: MediaLocator::FileSystem(path),
        fingerprint: FileFingerprint {
            size_bytes: metadata.len(),
            modified_at_utc_ms: metadata.modified().ok().and_then(utc_ms),
            created_at_utc_ms: metadata.created().ok().and_then(utc_ms),
        },
        metadata: None,
    }
}

fn unavailable(source_id: SourceId, reason: impl Into<String>) -> SourceScan {
    SourceScan {
        source_id,
        state: SourceScanState::Unavailable {
            reason: reason.into(),
        },
        tracks: Vec::new(),
        errors: Vec::new(),
    }
}

/// The `.lrc` files the last [`scan_directory`] saw beside audio files, waiting for the sync to take them (syncs run one at a
/// time, and the sync takes them right after the scan).
static LAST_SIDECARS: std::sync::Mutex<Vec<player_core::LyricSidecar>> = std::sync::Mutex::new(Vec::new());

/// Hand over (and forget) the sidecars of the last scan.
pub fn take_sidecars() -> Vec<player_core::LyricSidecar> {
    std::mem::take(
        &mut *LAST_SIDECARS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// Pair every `.lrc` with the audio file of the same name in the same folder (case-insensitive).
fn pair_sidecars(
    tracks: &[MediaTrackRecord],
    lyric_files: &std::collections::HashMap<(PathBuf, String), PathBuf>,
) -> Vec<player_core::LyricSidecar> {
    let mut found = Vec::new();
    for track in tracks {
        let MediaLocator::FileSystem(path) = &track.locator else {
            continue;
        };
        let (Some(parent), Some(stem)) = (path.parent(), path.file_stem()) else {
            continue;
        };
        let Some(lyric_path) =
            lyric_files.get(&(parent.to_path_buf(), stem.to_string_lossy().to_lowercase()))
        else {
            continue;
        };
        let Ok(metadata) = fs::metadata(lyric_path) else {
            continue;
        };
        found.push(player_core::LyricSidecar {
            track_item_id: track.identity.source_item_id.clone(),
            locator: MediaLocator::FileSystem(lyric_path.clone()),
            name: lyric_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            size_bytes: metadata.len(),
            modified_at_utc_ms: metadata.modified().ok().and_then(utc_ms),
        });
    }
    found
}

/// Walk a folder source. Hidden entries, symlinks, `.nomedia` folders (MediaStore parity), and a
/// storage volume's `Android/` tree are skipped. An unreadable root is `Unavailable` and any
/// unreadable subfolder makes the scan `Incomplete`, so neither ever deletes library mappings.
/// Folders in `excluded` (canonical real paths, 「排除資料夾」) and everything below them are
/// skipped like the Windows scanner, so their tracks leave the library on a complete scan.
pub fn scan_directory(
    root: &LibraryRoot,
    excluded: &[PathBuf],
    progress: &mut dyn FnMut(MediaScanProgress),
    cancellation: &SyncCancellation,
) -> SourceScan {
    let MediaLocator::FileSystem(configured) = &root.locator else {
        return unavailable(root.id, "Android folder source has no filesystem path");
    };
    if !root.enabled {
        return unavailable(root.id, "library root is disabled");
    }
    let canonical_root = match fs::canonicalize(configured) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => return unavailable(root.id, "library root is not a directory"),
        Err(error) => return unavailable(root.id, format!("library root is unavailable: {error}")),
    };
    let mut scan = SourceScan {
        source_id: root.id,
        state: SourceScanState::Complete,
        tracks: Vec::new(),
        errors: Vec::new(),
    };
    let mut directories = vec![canonical_root.clone()];
    let mut complete = true;
    let mut processed = 0_u64;
    let mut lyric_files: std::collections::HashMap<(PathBuf, String), PathBuf> =
        std::collections::HashMap::new();
    LAST_SIDECARS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    while let Some(directory) = directories.pop() {
        if cancellation.is_cancelled() {
            scan.state = SourceScanState::Incomplete {
                reason: "filesystem scan cancelled before traversal completed".to_owned(),
            };
            return scan;
        }
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if directory == canonical_root => {
                return unavailable(root.id, format!("could not enumerate library root: {error}"));
            }
            Err(error) => {
                complete = false;
                scan.errors.push(MediaSourceError {
                    kind: Default::default(),
                    source_item_id: Some(unix_path_key(&directory)),
                    message: format!("could not enumerate a subdirectory: {error}"),
                });
                continue;
            }
        };
        if directory != canonical_root && directory.join(".nomedia").exists() {
            continue;
        }
        let skip_android_tree = is_storage_volume_root(&directory);
        for entry in entries {
            processed = processed.saturating_add(1);
            progress(MediaScanProgress {
                processed,
                total: None,
                unit: MediaScanProgressUnit::FilesystemEntries,
            });
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    complete = false;
                    scan.errors.push(MediaSourceError {
                        kind: Default::default(),
                        source_item_id: Some(unix_path_key(&directory)),
                        message: format!("could not read a directory entry: {error}"),
                    });
                    continue;
                }
            };
            let name = entry.file_name();
            if name.to_str().is_some_and(|name| name.starts_with('.')) {
                continue;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                complete = false;
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if !(skip_android_tree && name == "Android") && !is_excluded(&path, excluded) {
                    directories.push(path);
                }
                continue;
            }
            let sidecar_rank = player_core::lyric_sidecar_rank(&name.to_string_lossy());
            if file_type.is_file() && sidecar_rank.is_some() {
                if let (Some(parent), Some(stem)) = (path.parent(), path.file_stem()) {
                    // One sidecar per song: `.lrc` over `.ass` over `.srt` (ADR 0041).
                    let key = (parent.to_path_buf(), stem.to_string_lossy().to_lowercase());
                    let better = lyric_files.get(&key).is_none_or(|kept: &PathBuf| {
                        kept.file_name()
                            .and_then(|kept| player_core::lyric_sidecar_rank(&kept.to_string_lossy()))
                            > sidecar_rank
                    });
                    if better {
                        lyric_files.insert(key, path.clone());
                    }
                }
                continue;
            }
            if !file_type.is_file() || !is_supported_audio_path(&path) {
                continue;
            }
            match entry.metadata() {
                Ok(metadata) => {
                    let item_id = unix_path_key(&path);
                    scan.tracks.push(record(root.id, item_id, path, &metadata));
                }
                Err(error) => {
                    complete = false;
                    scan.errors.push(MediaSourceError {
                        kind: Default::default(),
                        source_item_id: Some(unix_path_key(&path)),
                        message: format!("could not inspect audio file: {error}"),
                    });
                }
            }
        }
    }
    if !complete {
        scan.state = SourceScanState::Incomplete {
            reason: "one or more filesystem entries could not be enumerated or fingerprinted"
                .to_owned(),
        };
    }
    if !lyric_files.is_empty() {
        *LAST_SIDECARS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = pair_sidecars(&scan.tracks, &lyric_files);
    }
    scan
}

/// Inspect the unique audio files referenced by a playlist. One missing song is reported by its
/// lexical item ID so reconciliation preserves only that old mapping.
pub fn scan_paths(source_id: SourceId, paths: &[PathBuf]) -> SourceScan {
    scan_paths_with_progress(source_id, paths, &mut |_| {})
}

/// As [`scan_paths`], reporting how many of the listed files are checked (the same report a folder scan and a remote
/// playlist give, so the sync banner treats them alike).
pub fn scan_paths_with_progress(
    source_id: SourceId,
    paths: &[PathBuf],
    progress: &mut dyn FnMut(MediaScanProgress),
) -> SourceScan {
    let total = paths.len() as u64;
    let mut report = |done: u64| {
        progress(MediaScanProgress {
            processed: done,
            total: Some(total),
            unit: MediaScanProgressUnit::Tracks,
        });
    };
    report(0);
    let mut scan = SourceScan {
        source_id,
        state: SourceScanState::Complete,
        tracks: Vec::with_capacity(paths.len()),
        errors: Vec::new(),
    };
    let mut seen = HashSet::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        report(index as u64);
        if !is_supported_audio_path(path) {
            continue;
        }
        let item_id = unix_path_key(path);
        if !seen.insert(item_id.clone()) {
            continue;
        }
        let canonical = match fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) => {
                scan.errors.push(MediaSourceError {
                    kind: player_core::MediaSourceErrorKind::for_unopenable_file(path, &error),
                    source_item_id: Some(item_id),
                    message: format!("playlist media file is unavailable: {error}"),
                });
                continue;
            }
        };
        match fs::metadata(&canonical) {
            Ok(metadata) if metadata.is_file() => {
                scan.tracks.push(record(source_id, item_id, canonical, &metadata));
            }
            Ok(_) => {}
            Err(error) => scan.errors.push(MediaSourceError {
                kind: player_core::MediaSourceErrorKind::for_unopenable_file(path, &error),
                source_item_id: Some(item_id),
                message: format!("could not inspect playlist media file: {error}"),
            }),
        }
    }
    report(total);
    scan
}

/// Sibling file with the same stem and a different extension (e.g. `song.lrc` for `song.flac`).
/// True when `path` is an excluded folder or lies under one (component-wise prefix).
pub fn is_excluded(path: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().any(|folder| path.starts_with(folder))
}

pub fn sibling_with_extension(path: &Path, extension: &str) -> Option<PathBuf> {
    let candidate = path.with_extension(extension);
    candidate.is_file().then_some(candidate)
}

/// Folder artwork next to an audio file, in desktop/SAF priority order.
pub fn folder_artwork(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    ["cover.jpg", "folder.jpg", "cover.png", "folder.png"]
        .iter()
        .map(|name| parent.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_core::MediaSourceKind;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-fs-scan-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::canonicalize(dir).unwrap()
    }

    fn root_for(path: &Path) -> LibraryRoot {
        LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::AndroidFilesystem,
            display_name: "Music".into(),
            locator: MediaLocator::FileSystem(path.to_path_buf()),
            enabled: true,
        }
    }

    #[test]
    fn directory_scan_finds_audio_and_skips_hidden_nomedia_and_video() {
        let dir = temp_dir("walk");
        fs::create_dir_all(dir.join("Album/Disc 1")).unwrap();
        fs::create_dir_all(dir.join(".thumbnails")).unwrap();
        fs::create_dir_all(dir.join("Private")).unwrap();
        fs::write(dir.join("Album/Disc 1/春.flac"), b"x").unwrap();
        fs::write(dir.join("Album/cover.jpg"), b"x").unwrap();
        fs::write(dir.join("clip.mp4"), b"x").unwrap();
        fs::write(dir.join(".thumbnails/a.mp3"), b"x").unwrap();
        fs::write(dir.join("Private/.nomedia"), b"").unwrap();
        fs::write(dir.join("Private/b.mp3"), b"x").unwrap();
        let root = root_for(&dir);
        let scan = scan_directory(&root, &[], &mut |_| {}, &SyncCancellation::default());
        assert_eq!(scan.state, SourceScanState::Complete);
        assert_eq!(scan.tracks.len(), 1);
        let track = &scan.tracks[0];
        let expected = dir.join("Album/Disc 1/春.flac");
        assert_eq!(track.locator, MediaLocator::FileSystem(expected.clone()));
        assert_eq!(track.identity.source_item_id, unix_path_key(&expected));
        assert_eq!(track.identity.locator_key.as_deref(), Some(unix_path_key(&expected).as_str()));
        assert_eq!(
            folder_artwork(&expected),
            None,
            "cover.jpg is in the parent of Disc 1, not next to the file"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn excluded_folders_and_their_subfolders_are_skipped() {
        let dir = temp_dir("excluded");
        fs::create_dir_all(dir.join("Keep")).unwrap();
        fs::create_dir_all(dir.join("Skip/Deeper")).unwrap();
        fs::create_dir_all(dir.join("SkipNot")).unwrap();
        fs::write(dir.join("Keep/a.mp3"), b"x").unwrap();
        fs::write(dir.join("Skip/b.mp3"), b"x").unwrap();
        fs::write(dir.join("Skip/Deeper/c.mp3"), b"x").unwrap();
        fs::write(dir.join("SkipNot/d.mp3"), b"x").unwrap();
        let root = root_for(&dir);
        let excluded = vec![dir.join("Skip")];
        let scan = scan_directory(&root, &excluded, &mut |_| {}, &SyncCancellation::default());
        assert_eq!(scan.state, SourceScanState::Complete);
        let mut names = scan
            .tracks
            .iter()
            .map(|track| match &track.locator {
                MediaLocator::FileSystem(path) => path.file_name().unwrap().to_string_lossy().into_owned(),
                MediaLocator::ContentUri(uri) => uri.clone(),
            })
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, vec!["a.mp3".to_owned(), "d.mp3".to_owned()]);
        assert!(is_excluded(&dir.join("Skip/Deeper"), &excluded));
        assert!(!is_excluded(&dir.join("SkipNot"), &excluded));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_root_is_unavailable_and_never_complete() {
        let root = root_for(Path::new("/definitely/not/here"));
        let scan = scan_directory(&root, &[], &mut |_| {}, &SyncCancellation::default());
        assert!(matches!(scan.state, SourceScanState::Unavailable { .. }));
    }

    #[test]
    fn storage_volume_roots_are_recognized() {
        assert!(is_storage_volume_root(Path::new("/storage/emulated/0")));
        assert!(is_storage_volume_root(Path::new("/storage/1A2B-3C4D")));
        assert!(!is_storage_volume_root(Path::new("/storage/emulated/0/Music")));
        assert!(!is_storage_volume_root(Path::new("/storage/emulated")));
    }

    #[test]
    fn playlist_paths_report_missing_entries_and_dedupe() {
        let dir = temp_dir("paths");
        fs::write(dir.join("a.mp3"), b"x").unwrap();
        fs::write(dir.join("song.lrc"), b"x").unwrap();
        fs::write(dir.join("song.flac"), b"x").unwrap();
        let scan = scan_paths(
            SourceId::new(),
            &[dir.join("a.mp3"), dir.join("a.mp3"), dir.join("gone.mp3"), dir.join("x.txt")],
        );
        assert_eq!(scan.tracks.len(), 1);
        assert_eq!(scan.errors.len(), 1);
        assert_eq!(sibling_with_extension(&dir.join("song.flac"), "lrc"), Some(dir.join("song.lrc")));
        fs::remove_dir_all(dir).unwrap();
    }
}
