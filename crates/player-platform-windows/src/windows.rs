use std::{
    collections::HashSet,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use player_core::{
    windows_locator_key, FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaScanProgress,
    MediaScanProgressUnit, MediaSourceError, MediaSourceKind, MediaTrackRecord, MetadataReader,
    SourceId, SourceScan, SourceScanState, SyncCancellation, TrackIdentity, TrackMetadata,
    TrackMetadataError,
};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
/// Windows media index provider. Until SystemIndex can prove full root coverage and freshness,
/// this provider uses a complete filesystem walk. A zero-result Windows Search query is never
/// treated as a complete empty library.
#[derive(Clone, Debug, Default)]
pub struct WindowsMediaIndex {
    /// Canonical folder paths whose trees are omitted from walks and playlist media scans.
    excluded_folders: Vec<PathBuf>,
    /// The `.lrc` files the last walk saw beside audio files (see `MediaIndex::take_lyric_sidecars`).
    sidecars: Vec<player_core::LyricSidecar>,
}

impl WindowsMediaIndex {
    pub fn new() -> Self {
        Self {
            excluded_folders: Vec::new(),
            sidecars: Vec::new(),
        }
    }

    /// Prefer already-canonical paths. Non-canonical entries are resolved when possible.
    pub fn with_excluded_folders(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut excluded_folders: Vec<PathBuf> = Vec::new();
        for path in paths {
            let canonical = fs::canonicalize(&path).unwrap_or(path);
            if !excluded_folders
                .iter()
                .any(|existing| path_is_same_or_under(&canonical, existing.as_path()))
            {
                excluded_folders.push(canonical);
            }
        }
        Self {
            excluded_folders,
            sidecars: Vec::new(),
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        self.excluded_folders
            .iter()
            .any(|excluded| path_is_same_or_under(path, excluded))
    }

    /// Inspect the unique audio files referenced by a parsed M3U/M3U8 source. The playlist
    /// itself is the complete membership list; one unavailable song is reported by its stable
    /// lexical item ID so the database can preserve only that old mapping during reconciliation.
    pub fn scan_playlist_paths(
        &mut self,
        source_id: player_core::SourceId,
        paths: &[PathBuf],
    ) -> SourceScan {
        self.scan_playlist_paths_with_progress(source_id, paths, &mut |_| {})
    }

    /// As [`scan_playlist_paths`](Self::scan_playlist_paths), reporting how many of the listed files are checked.
    pub fn scan_playlist_paths_with_progress(
        &mut self,
        source_id: player_core::SourceId,
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
            if !is_supported_audio_file(path) {
                continue;
            }
            let item_id = windows_locator_key(path);
            if !seen.insert(item_id.clone()) {
                continue;
            }
            // Resolve relative `..` components before adding the Windows verbatim prefix;
            // extended paths reject dot segments even though normal Win32 paths accept them.
            let canonical_path = fs::canonicalize(path)
                .or_else(|_| to_extended_path(path).and_then(fs::canonicalize));
            let canonical_path = match canonical_path {
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
            if self.is_excluded(&canonical_path) {
                continue;
            }
            let metadata = match fs::metadata(&canonical_path) {
                Ok(metadata) if metadata.is_file() => metadata,
                Ok(_) => continue,
                Err(error) => {
                    scan.errors.push(MediaSourceError {
                        kind: player_core::MediaSourceErrorKind::for_unopenable_file(path, &error),
                        source_item_id: Some(item_id),
                        message: format!("could not inspect playlist media file: {error}"),
                    });
                    continue;
                }
            };
            if !is_supported_audio_file(&canonical_path) {
                continue;
            }
            let modified_at_utc_ms = match metadata.modified() {
                Ok(modified) => match system_time_to_utc_ms(modified) {
                    Some(value) => Some(value),
                    None => {
                        scan.errors.push(MediaSourceError {
                            kind: Default::default(),
                            source_item_id: Some(item_id.clone()),
                            message: "playlist media modification time is outside the supported UTC epoch range".to_owned(),
                        });
                        None
                    }
                },
                Err(error) => {
                    scan.errors.push(MediaSourceError {
                        kind: Default::default(),
                        source_item_id: Some(item_id.clone()),
                        message: format!(
                            "could not read playlist media modification time: {error}"
                        ),
                    });
                    None
                }
            };
            let created_at_utc_ms = metadata.created().ok().and_then(system_time_to_utc_ms);
            let canonical_key = windows_locator_key(&canonical_path);
            scan.tracks.push(MediaTrackRecord {
                identity: TrackIdentity {
                    source_id,
                    source_item_id: item_id,
                    locator_key: Some(canonical_key),
                },
                locator: MediaLocator::FileSystem(canonical_path),
                fingerprint: FileFingerprint {
                    size_bytes: metadata.len(),
                    modified_at_utc_ms,
                    created_at_utc_ms,
                },
                metadata: None,
            });
        }
        report(total);
        scan
    }
}

/// Read tags and codec properties for one file without adding it to a library or database.
/// Used by Windows shell-open playback for files outside configured library roots.
pub fn read_standalone_audio_metadata(path: &Path) -> Result<TrackMetadata, TrackMetadataError> {
    let canonical_path = fs::canonicalize(path).map_err(|error| TrackMetadataError {
        message: format!("could not resolve audio file: {error}"),
    })?;
    let metadata = fs::metadata(&canonical_path).map_err(|error| TrackMetadataError {
        message: format!("could not inspect audio file: {error}"),
    })?;
    if !metadata.is_file() || !is_supported_audio_file(&canonical_path) {
        return Err(TrackMetadataError {
            message: "file is not a supported audio format".to_owned(),
        });
    }
    let identity_key = windows_locator_key(&canonical_path);
    let record = MediaTrackRecord {
        identity: TrackIdentity {
            source_id: SourceId::new(),
            source_item_id: identity_key.clone(),
            locator_key: Some(identity_key),
        },
        locator: MediaLocator::FileSystem(canonical_path),
        fingerprint: FileFingerprint {
            size_bytes: metadata.len(),
            modified_at_utc_ms: None,
            created_at_utc_ms: metadata.created().ok().and_then(system_time_to_utc_ms),
        },
        metadata: None,
    };
    WindowsMediaIndex::new().read_metadata(&record)
}

/// Parsing a file keeps no state in the index, so tracks can be parsed on several threads at once.
impl MetadataReader for WindowsMediaIndex {
    fn read(&self, track: &MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError> {
        WindowsMediaIndex::new().read_metadata(track)
    }
}

impl MediaIndex for WindowsMediaIndex {
    fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
        Some(self)
    }

    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.scan_with_progress(root, &mut |_| {}, &SyncCancellation::default())
    }

    fn take_lyric_sidecars(&mut self) -> Vec<player_core::LyricSidecar> {
        std::mem::take(&mut self.sidecars)
    }

    fn scans_lyric_sidecars(&self) -> bool {
        true
    }

    fn scan_with_progress(
        &mut self,
        root: &LibraryRoot,
        progress: &mut dyn FnMut(MediaScanProgress),
        cancellation: &SyncCancellation,
    ) -> SourceScan {
        self.sidecars.clear();
        let mut lyric_files: std::collections::HashMap<(PathBuf, String), PathBuf> =
            std::collections::HashMap::new();
        let mut scan = empty_scan(root.id);
        if cancellation.is_cancelled() {
            scan.state = SourceScanState::Incomplete {
                reason: "filesystem scan cancelled before traversal started".to_owned(),
            };
            return scan;
        }
        if !root.enabled {
            scan.state = SourceScanState::Unavailable {
                reason: "library root is disabled".to_owned(),
            };
            return scan;
        }

        if !matches!(
            root.kind,
            MediaSourceKind::WindowsFilesystem | MediaSourceKind::WindowsSystemIndex
        ) {
            scan.state = SourceScanState::Unavailable {
                reason: "WindowsMediaIndex requires a Windows filesystem library root".to_owned(),
            };
            return scan;
        }

        let configured_path = match &root.locator {
            MediaLocator::FileSystem(path) => path,
            MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => {
                scan.state = SourceScanState::Unavailable {
                    reason: "Windows filesystem root has a non-filesystem locator".to_owned(),
                };
                return scan;
            }
        };

        let root_path = match to_extended_path(configured_path) {
            Ok(path) => path,
            Err(error) => {
                scan.state = SourceScanState::Unavailable {
                    reason: format!("could not resolve library root: {error}"),
                };
                return scan;
            }
        };

        match fs::metadata(&root_path) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                scan.state = SourceScanState::Unavailable {
                    reason: "library root is not a directory".to_owned(),
                };
                return scan;
            }
            Err(error) => {
                scan.state = SourceScanState::Unavailable {
                    reason: format!("library root is unavailable: {error}"),
                };
                return scan;
            }
        }

        let canonical_root = match fs::canonicalize(&root_path) {
            Ok(path) => path,
            Err(error) => {
                scan.state = SourceScanState::Unavailable {
                    reason: format!("could not canonicalize library root: {error}"),
                };
                return scan;
            }
        };

        if self.is_excluded(&canonical_root) {
            scan.state = SourceScanState::Complete;
            return scan;
        }

        if root.kind == MediaSourceKind::WindowsSystemIndex {
            use std::io::Write as _;
            let diagnostic = crate::system_index::diagnostic(&canonical_root);
            let _ = writeln!(std::io::stderr(), "{diagnostic}");
        }

        let mut directories = vec![canonical_root.clone()];
        let mut opened_root = false;
        let mut complete = true;
        let mut processed_entries = 0_u64;
        while let Some(directory) = directories.pop() {
            if cancellation.is_cancelled() {
                scan.state = SourceScanState::Incomplete {
                    reason: "filesystem scan cancelled before traversal completed".to_owned(),
                };
                scan.errors.push(MediaSourceError {
                    kind: Default::default(),
                    source_item_id: None,
                    message: "filesystem scan cancelled".to_owned(),
                });
                return scan;
            }
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => {
                    if directory == canonical_root {
                        opened_root = true;
                    }
                    entries
                }
                Err(error) if directory == canonical_root && !opened_root => {
                    scan.state = SourceScanState::Unavailable {
                        reason: format!("could not enumerate library root: {error}"),
                    };
                    scan.tracks.clear();
                    return scan;
                }
                Err(error) => {
                    complete = false;
                    scan.errors.push(MediaSourceError {
                        kind: Default::default(),
                        source_item_id: Some(windows_locator_key(&directory)),
                        message: format!("could not enumerate a subdirectory: {error}"),
                    });
                    continue;
                }
            };

            for entry in entries {
                processed_entries = processed_entries.saturating_add(1);
                progress(MediaScanProgress {
                    processed: processed_entries,
                    total: None,
                    unit: MediaScanProgressUnit::FilesystemEntries,
                });
                if cancellation.is_cancelled() {
                    scan.state = SourceScanState::Incomplete {
                        reason: "filesystem scan cancelled before traversal completed".to_owned(),
                    };
                    scan.errors.push(MediaSourceError {
                        kind: Default::default(),
                        source_item_id: None,
                        message: "filesystem scan cancelled".to_owned(),
                    });
                    return scan;
                }

                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
                            kind: Default::default(),
                            source_item_id: Some(windows_locator_key(&directory)),
                            message: format!("could not read a directory entry: {error}"),
                        });
                        continue;
                    }
                };

                let entry_path = entry.path();
                let entry_type = match entry.file_type() {
                    Ok(entry_type) => entry_type,
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
                            kind: Default::default(),
                            source_item_id: Some(windows_locator_key(&entry_path)),
                            message: format!("could not inspect a filesystem entry: {error}"),
                        });
                        continue;
                    }
                };

                // Do not follow links or reparse points: they can lead outside the configured root
                // or create cycles. Their targets can be added as their own configured roots.
                if entry_type.is_symlink() {
                    continue;
                }
                if entry_type.is_dir() {
                    if self.is_excluded(&entry_path) {
                        continue;
                    }
                    directories.push(entry_path);
                    continue;
                }
                let sidecar_rank = entry_path
                    .file_name()
                    .and_then(|name| player_core::lyric_sidecar_rank(&name.to_string_lossy()));
                if entry_type.is_file()
                    && sidecar_rank.is_some()
                    && !self.is_excluded(&entry_path)
                {
                    if let (Some(parent), Some(stem)) = (entry_path.parent(), entry_path.file_stem()) {
                        // One sidecar per song: `.lrc` over `.ass` over `.srt` (ADR 0041).
                        let key = (parent.to_path_buf(), stem.to_string_lossy().to_lowercase());
                        let better = lyric_files.get(&key).is_none_or(|kept: &std::path::PathBuf| {
                            kept.file_name()
                                .and_then(|name| player_core::lyric_sidecar_rank(&name.to_string_lossy()))
                                > sidecar_rank
                        });
                        if better {
                            lyric_files.insert(key, entry_path.clone());
                        }
                    }
                    continue;
                }
                if !entry_type.is_file() || !is_supported_audio_file(&entry_path) {
                    continue;
                }
                if self.is_excluded(&entry_path) {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
                            kind: Default::default(),
                            source_item_id: Some(windows_locator_key(&entry_path)),
                            message: format!("could not inspect audio file: {error}"),
                        });
                        continue;
                    }
                };
                let modified_at_utc_ms = match metadata.modified() {
                    Ok(modified) => match system_time_to_utc_ms(modified) {
                        Some(value) => Some(value),
                        None => {
                            complete = false;
                            scan.errors.push(MediaSourceError {
                                kind: Default::default(),
                                source_item_id: Some(windows_locator_key(&entry_path)),
                                message: "file modification time is outside the supported UTC epoch range".to_owned(),
                            });
                            None
                        }
                    },
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
                            kind: Default::default(),
                            source_item_id: Some(windows_locator_key(&entry_path)),
                            message: format!("could not read file modification time: {error}"),
                        });
                        None
                    }
                };
                let created_at_utc_ms = metadata.created().ok().and_then(system_time_to_utc_ms);

                let identity_key = windows_locator_key(&entry_path);
                scan.tracks.push(MediaTrackRecord {
                    identity: TrackIdentity {
                        source_id: root.id,
                        source_item_id: identity_key.clone(),
                        locator_key: Some(identity_key),
                    },
                    locator: MediaLocator::FileSystem(entry_path),
                    fingerprint: FileFingerprint {
                        size_bytes: metadata.len(),
                        modified_at_utc_ms,
                        created_at_utc_ms,
                    },
                    metadata: None,
                });
            }
        }

        // The lyrics beside the songs: a sidecar with the song's name in the song's folder (only its name is kept).
        if !lyric_files.is_empty() {
            for track in &scan.tracks {
                let MediaLocator::FileSystem(path) = &track.locator else {
                    continue;
                };
                let (Some(parent), Some(stem)) = (path.parent(), path.file_stem()) else {
                    continue;
                };
                let Some(lyric_path) = lyric_files.get(&(parent.to_path_buf(), stem.to_string_lossy().to_lowercase())) else {
                    continue;
                };
                self.sidecars.push(player_core::LyricSidecar {
                    track_item_id: track.identity.source_item_id.clone(),
                    name: lyric_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                });
            }
        }

        scan.state = if complete {
            SourceScanState::Complete
        } else {
            SourceScanState::Incomplete {
                reason: "one or more filesystem entries could not be enumerated or fingerprinted"
                    .to_owned(),
            }
        };
        scan
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        let path = match &track.locator {
            MediaLocator::FileSystem(path) => path,
            MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => {
                return Err(TrackMetadataError {
                    message: "Windows metadata reader requires a filesystem path".to_owned(),
                });
            }
        };
        player_track_metadata::read_track_metadata(&player_track_metadata::PathSource(path))
    }
}

fn path_is_same_or_under(path: &Path, ancestor: &Path) -> bool {
    path == ancestor || path.starts_with(ancestor)
}

fn empty_scan(source_id: player_core::SourceId) -> SourceScan {
    SourceScan {
        source_id,
        state: SourceScanState::Unavailable {
            reason: "scan did not start".to_owned(),
        },
        tracks: Vec::new(),
        errors: Vec::new(),
    }
}

/// Whether the path has a supported audio extension, excluding MP4/MKV video files.
/// M4A and MKA remain accepted as audio containers; this filter does not infer or constrain their codec.
/// The accepted set lives in `player-formats`.
pub fn is_supported_audio_file(path: &Path) -> bool {
    player_formats::is_windows_library_audio(path)
}

/// MP4 files are treated as video candidates and excluded from Windows library projection.
pub fn is_video_mp4(path: &Path) -> bool {
    player_formats::is_video_mp4(path)
}

/// File extensions accepted by the library scanner, formatted for Windows association keys.
pub fn supported_audio_extensions() -> Vec<String> {
    player_formats::windows_library_extensions()
}

fn system_time_to_utc_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).ok(),
        Err(error) => i64::try_from(error.duration().as_millis())
            .ok()
            .and_then(i64::checked_neg),
    }
}

pub(crate) fn to_extended_path(path: &Path) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path)?
    };
    let units = absolute.as_os_str().encode_wide().collect::<Vec<_>>();
    if has_prefix(
        &units,
        &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16],
    ) || has_prefix(
        &units,
        &[b'\\' as u16, b'\\' as u16, b'.' as u16, b'\\' as u16],
    ) {
        return Ok(absolute);
    }

    let mut extended = "\\\\?\\".encode_utf16().collect::<Vec<_>>();
    if units.len() >= 2 && units[0] == b'\\' as u16 && units[1] == b'\\' as u16 {
        extended.extend("UNC\\".encode_utf16());
        extended.extend_from_slice(&units[2..]);
    } else {
        extended.extend(units);
    }
    Ok(PathBuf::from(OsString::from_wide(&extended)))
}

fn has_prefix(units: &[u16], prefix: &[u16]) -> bool {
    units.len() >= prefix.len() && units[..prefix.len()] == *prefix
}

#[cfg(test)]
mod tests {
    use super::{
        is_supported_audio_file, supported_audio_extensions, to_extended_path, windows_locator_key,
        WindowsMediaIndex,
    };
    use lofty::file::FileType;
    use lofty::mpeg::Layer;
    use player_core::{
        FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaScanProgressUnit,
        MediaSourceKind, MediaTrackRecord, SourceId, SourceScanState, SyncCancellation,
        TrackIdentity, UserMetadataField,
    };
    use player_db::Database;
    use player_track_metadata::{
        matroska_bitrate_bps, mpeg_codec_name, read_lossless_wav_bit_depth, read_wavpack_summary,
        reliable_source_bit_depth,
    };
    use std::{
        fs,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!("moemusic-{label}-{}-{nonce}", std::process::id()))
    }

    fn add_root(db: &Database, path: PathBuf) -> LibraryRoot {
        db.add_library_root(
            MediaSourceKind::WindowsFilesystem,
            "測試曲庫",
            MediaLocator::FileSystem(path),
        )
        .expect("add root")
    }

    #[test]
    fn system_index_diagnostic_is_not_reported_as_a_scan_error() {
        let base = temp_path("system-index-diagnostic");
        fs::create_dir_all(&base).expect("create empty test root");
        let root = LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsSystemIndex,
            display_name: "SystemIndex test".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let mut index = WindowsMediaIndex::new();

        let scan = index.scan(&root);

        assert_eq!(scan.state, SourceScanState::Complete);
        assert!(
            scan.errors.is_empty(),
            "a WSearch capability diagnostic must not become a source item error when filesystem traversal completed"
        );
        fs::remove_dir_all(&base).expect("remove empty test root");
    }

    fn write_tagged_mp3(path: &Path, title: &str) {
        write_tagged_mp3_with_date(path, title, "2024-02-29");
    }

    fn write_tagged_mp3_with_date(path: &Path, title: &str, recording_date: &str) {
        fn add_frame(body: &mut Vec<u8>, name: &[u8; 4], text: &str) {
            let mut value = vec![3]; // ID3v2.4 UTF-8 text encoding
            value.extend(text.as_bytes());
            value.push(0);
            body.extend(name);
            let size = value.len() as u32;
            body.extend([
                ((size >> 21) & 0x7f) as u8,
                ((size >> 14) & 0x7f) as u8,
                ((size >> 7) & 0x7f) as u8,
                (size & 0x7f) as u8,
                0,
                0,
            ]);
            body.extend(value);
        }

        let mut body = Vec::new();
        add_frame(&mut body, b"TIT2", title);
        add_frame(&mut body, b"TPE1", "測試演出者");
        add_frame(&mut body, b"TPE2", "專輯演出者");
        add_frame(&mut body, b"TRCK", "4");
        add_frame(&mut body, b"TDRC", recording_date);
        let size = body.len() as u32;
        let header = [
            b'I',
            b'D',
            b'3',
            4,
            0,
            0,
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ];
        let mut file = header.to_vec();
        file.extend(body);
        // A small, locally generated MPEG-1 Layer III frame sequence makes the fixture a valid
        // media container without copying any third-party audio sample.
        for _ in 0..3 {
            file.extend([0xff, 0xfb, 0x90, 0x64]);
            file.resize(file.len() + 413, 0);
        }
        fs::write(path, file).expect("write ID3 fixture");
    }

    fn long_file_path(base: &Path) -> PathBuf {
        let mut path = base.to_path_buf();
        for index in 0..12 {
            path.push(format!("資料夾-{index:02}-長路徑測試-1234567890"));
        }
        path.push("音樂-東京🌸-混合文字-長檔名-測試.mp3");
        path
    }

    #[test]
    fn windows_locator_key_is_lossless_for_unicode_and_verbatim_prefixes() {
        let ordinary = PathBuf::from(r"C:\音樂\東京🌸\track.mp3");
        let verbatim = PathBuf::from(r"\\?\C:\音樂\東京🌸\track.mp3");
        assert_eq!(
            windows_locator_key(&ordinary),
            windows_locator_key(&verbatim)
        );
        assert_eq!(
            windows_locator_key(Path::new(r"C:/音樂/東京🌸/track.mp3")),
            windows_locator_key(&ordinary)
        );
        assert_eq!(
            windows_locator_key(Path::new(r"\\server\share\music\track.mp3")),
            windows_locator_key(Path::new(r"\\?\UNC\server\share\music\track.mp3"))
        );

        let source = ordinary.as_os_str().encode_wide().collect::<Vec<_>>();
        let key_units = windows_locator_key(&ordinary)
            .strip_prefix("windows-u16-v1:")
            .expect("key prefix")
            .as_bytes()
            .chunks_exact(4)
            .map(|chunk| {
                u16::from_str_radix(std::str::from_utf8(chunk).expect("hex"), 16).expect("u16")
            })
            .collect::<Vec<_>>();
        let mut expected = source;
        expected[0] = b'C' as u16;
        assert_eq!(key_units, expected);
    }

    #[test]
    fn windows_locator_key_preserves_unpaired_utf16_code_units() {
        use std::os::windows::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xd800,
            b'\\' as u16,
            b'x' as u16,
        ]));
        assert!(windows_locator_key(&path).contains("d800"));
    }

    #[test]
    fn scans_unicode_long_path_and_extracts_metadata() {
        let base = temp_path("unicode");
        let file_path = long_file_path(&base);
        let extended = to_extended_path(&file_path).expect("extended path");
        fs::create_dir_all(extended.parent().expect("parent")).expect("long parent");
        write_tagged_mp3(&extended, "花の歌");

        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, base.clone());
        let mut index = WindowsMediaIndex::new();
        let report = player_core::SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000)
            .expect("sync");

        assert_eq!(report.state, Some(SourceScanState::Complete));
        assert_eq!(report.observed, 1);
        assert!(
            report.metadata_errors.is_empty(),
            "{:?}",
            report.metadata_errors
        );
        let page = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title.as_deref(), Some("花の歌"));
        assert_eq!(page.items[0].artist.as_deref(), Some("測試演出者"));
        assert!(page.items[0].duration_ms.unwrap_or_default() > 0);

        fs::remove_dir_all(to_extended_path(&base).expect("extended root")).expect("cleanup");
    }

    /// Scans the folder of official-encoder files into an in-memory library (`MOE_WV_MANIFEST` is in it; run by hand
    /// with `-- --ignored`) and checks every column the list shows: title, artist, album, year, codec, sample rate,
    /// bit depth and duration.
    #[test]
    #[ignore = "needs MOE_WV_MANIFEST"]
    fn official_encoder_files_scan_into_the_library() {
        let manifest_path =
            PathBuf::from(std::env::var("MOE_WV_MANIFEST").unwrap().replace('/', "\\"));
        let manifest = fs::read_to_string(&manifest_path).unwrap();
        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, manifest_path.parent().unwrap().to_path_buf());
        let mut index = WindowsMediaIndex::new();
        let report = player_core::SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000)
            .expect("sync");
        assert_eq!(report.state, Some(SourceScanState::Complete));
        assert!(
            report.metadata_errors.is_empty(),
            "{:?}",
            report.metadata_errors
        );
        let page = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 200,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("page");
        for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
            let cols: Vec<&str> = line.split('\t').collect();
            let name = cols[0];
            let track = page
                .items
                .iter()
                .find(|track| track.title.as_deref() == Some(name))
                .unwrap_or_else(|| panic!("{name}: not in the library"));
            let expected_depth = if cols[5] != "1" {
                None
            } else if name.contains("float32") || name.contains("int32") {
                Some(32)
            } else if name.starts_with("cd16") {
                Some(16)
            } else {
                Some(24)
            };
            let rate = if name.starts_with("cd16") || name.contains("44k") {
                44_100
            } else {
                96_000
            };
            assert_eq!(track.artist.as_deref(), Some("MoeTest"), "{name}");
            assert!(
                track
                    .album
                    .as_deref()
                    .is_some_and(|album| album.starts_with("WavPack ")),
                "{name}"
            );
            assert_eq!(
                track.codec.as_deref(),
                Some(if cols[5] == "1" { "WVLL" } else { "WVL" }),
                "{name}"
            );
            assert_eq!(track.sample_rate_hz, Some(rate), "{name}");
            assert_eq!(track.bit_depth, expected_depth, "{name}: bit depth");
            assert!(track.duration_ms.unwrap_or_default() > 60_000, "{name}");
            println!(
                "{name}: {} / {} — {:?} {} Hz {:?}-bit, {:.1} s, {:?} bps",
                track.artist.as_deref().unwrap_or(""),
                track.album.as_deref().unwrap_or(""),
                track.codec,
                track.sample_rate_hz.unwrap_or(0),
                track.bit_depth,
                track.duration_ms.unwrap_or(0) as f64 / 1000.0,
                track.bitrate_bps
            );
        }
        // DSD inside WavPack (made with `wavpack song.dsf`, in the `dsd` folder when it exists).
        if let Some(track) = page
            .items
            .iter()
            .find(|track| track.title.as_deref() == Some("dsd_lossless"))
        {
            assert_eq!(track.codec.as_deref(), Some("WVDSD"));
            assert_eq!(track.sample_rate_hz, Some(11_289_600));
            assert_eq!(track.bit_depth, Some(1));
            println!(
                "dsd_lossless: {:?} {:?} Hz {:?}-bit",
                track.codec, track.sample_rate_hz, track.bit_depth
            );
        }
        // A second scan finds nothing to re-read.
        let again = player_core::SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000)
            .expect("second sync");
        assert!(again.metadata_errors.is_empty());
    }

    #[test]
    fn playlist_path_scan_deduplicates_unicode_entries_and_reports_missing_individually() {
        let base = temp_path("playlist-source");
        fs::create_dir_all(&base).expect("create playlist media directory");
        let media_path = base.join("音樂 東京 🎧.mp3");
        write_tagged_mp3(&media_path, "Playlist source track");
        let missing_path = base.join("暫時缺席.flac");
        let mut index = WindowsMediaIndex::new();
        let scan = index.scan_playlist_paths(
            SourceId::new(),
            &[media_path.clone(), media_path, missing_path],
        );
        assert_eq!(scan.state, SourceScanState::Complete);
        assert_eq!(scan.tracks.len(), 1);
        assert_eq!(scan.errors.len(), 1);
        assert!(scan.errors[0].source_item_id.is_some());
        assert!(matches!(
            &scan.tracks[0].locator,
            MediaLocator::FileSystem(path) if path.file_name().is_some_and(|name| name == "音樂 東京 🎧.mp3")
        ));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn bad_file_isolated_and_only_complete_scan_removes_missing_items() {
        let base = temp_path("diff");
        fs::create_dir_all(&base).expect("root");
        let first = base.join("first.mp3");
        let second = base.join("second.mp3");
        fs::write(&first, b"bad mp3").expect("first");
        write_tagged_mp3(&second, "second");

        let mut db = Database::open_in_memory().expect("database");
        let root = add_root(&db, base.clone());
        let mut index = WindowsMediaIndex::new();
        let initial =
            player_core::SyncEngine::sync(&root, &mut index, &mut db, 1).expect("initial sync");
        assert_eq!(initial.observed, 2);
        assert_eq!(initial.metadata_errors.len(), 1);
        assert_eq!(initial.state, Some(SourceScanState::Complete));
        let unchanged =
            player_core::SyncEngine::sync(&root, &mut index, &mut db, 2).expect("unchanged sync");
        assert_eq!(unchanged.unchanged, 1);
        assert_eq!(unchanged.metadata_reads, 1); // only the malformed file is retried
        assert_eq!(unchanged.metadata_errors.len(), 1);
        let original_page = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("initial page");
        assert_eq!(original_page.items.len(), 2);
        let retained_id = original_page
            .items
            .iter()
            .find(|track| track.title.as_deref() == Some("second"))
            .expect("tagged track")
            .id;
        db.set_user_override(retained_id, UserMetadataField::Title, Some("手動標題"))
            .expect("override");

        use std::io::Write as _;
        fs::OpenOptions::new()
            .append(true)
            .open(&second)
            .expect("open changed file")
            .write_all(b"\0\0\0\0")
            .expect("change file size");
        fs::remove_file(&first).expect("delete first file");
        let after_delete = player_core::SyncEngine::sync(&root, &mut index, &mut db, 3)
            .expect("complete scan after delete");
        assert_eq!(after_delete.state, Some(SourceScanState::Complete));
        // Folder files that vanish are hidden, not removed.
        assert_eq!(after_delete.applied.removed_source_mappings, 0);
        assert_eq!(after_delete.applied.hidden_missing_mappings, 1);
        assert_eq!(after_delete.metadata_reads, 1);
        assert!(after_delete.metadata_errors.is_empty());
        let remaining = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("remaining page");
        assert_eq!(remaining.items.len(), 1);
        assert_eq!(remaining.items[0].id, retained_id);
        assert_eq!(remaining.items[0].title.as_deref(), Some("手動標題"));

        fs::remove_dir_all(&base).expect("remove root");
        let offline =
            player_core::SyncEngine::sync(&root, &mut index, &mut db, 4).expect("offline sync");
        assert!(matches!(
            offline.state,
            Some(SourceScanState::Unavailable { .. })
        ));
        assert_eq!(offline.applied.removed_source_mappings, 0);
        assert_eq!(
            db.list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                }
            )
            .expect("retained offline track")
            .items[0]
                .id,
            retained_id
        );
    }

    #[test]
    fn scanner_records_utc_epoch_millisecond_fingerprint() {
        let base = temp_path("mtime");
        fs::create_dir_all(&base).expect("root");
        let file = base.join("track.FLAC");
        fs::write(&file, b"placeholder").expect("file");

        let root = LibraryRoot {
            id: player_core::SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "mtime".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let scan = WindowsMediaIndex::new().scan(&root);
        assert_eq!(scan.state, SourceScanState::Complete);
        let fingerprint = scan.tracks[0].fingerprint;
        assert!(fingerprint.modified_at_utc_ms.is_some());
        assert!(fingerprint.size_bytes > 0);
        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn the_scan_pairs_each_lrc_with_the_song_of_the_same_name_in_its_folder() {
        let base = temp_path("lrc-pairs");
        fs::create_dir_all(base.join("sub")).expect("root");
        fs::write(base.join("Track.FLAC"), b"placeholder").expect("song");
        fs::write(base.join("track.LRC"), b"[00:01.00]hi\n").expect("sidecar");
        // A `.lrc` without its song, and a song of another folder with the same name, are not paired.
        fs::write(base.join("lone.lrc"), b"x").expect("lone");
        fs::write(base.join("sub").join("track.mp3"), b"placeholder").expect("same name elsewhere");

        let root = LibraryRoot {
            id: player_core::SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "lrc".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let mut index = WindowsMediaIndex::new();
        let scan = index.scan(&root);
        assert_eq!(scan.tracks.len(), 2);
        let sidecars = index.take_lyric_sidecars();
        assert_eq!(sidecars.len(), 1);
        assert_eq!(sidecars[0].name, "track.LRC");
        let song = scan
            .tracks
            .iter()
            .find(|track| track.identity.source_item_id == sidecars[0].track_item_id)
            .expect("the paired song");
        assert!(matches!(&song.locator, MediaLocator::FileSystem(path) if path.ends_with("Track.FLAC")));
        // Taken once: the next call has nothing, and a new scan starts clean.
        assert!(index.take_lyric_sidecars().is_empty());
        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn complete_folder_sync_removes_old_mp4_mapping_but_incomplete_sync_retains_it() {
        use player_core::{
            FileFingerprint, LibraryRepository, MediaTrackRecord, TrackIdentity, TrackMetadata,
        };

        let base = temp_path("mp4-mapping-reconcile");
        fs::create_dir_all(&base).expect("create scan root");
        let video = base.join("舊影片.mp4");
        fs::write(&video, b"video candidate").expect("write MP4 candidate");
        let mut database = Database::open_in_memory().expect("database");
        let root = add_root(&database, base.clone());
        let source_item_id = windows_locator_key(&video);
        let file_metadata = fs::metadata(&video).expect("MP4 metadata");
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: root.id,
                source_item_id: source_item_id.clone(),
                locator_key: Some(source_item_id),
            },
            locator: MediaLocator::FileSystem(video),
            fingerprint: FileFingerprint {
                size_bytes: file_metadata.len(),
                modified_at_utc_ms: Some(1_800_000_000_000),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some("legacy video mapping".to_owned()),
                ..TrackMetadata::default()
            }),
        };
        database
            .apply_source_scan(
                &root,
                &SourceScanState::Complete,
                std::slice::from_ref(&record),
                std::slice::from_ref(&record),
                &[],
                1_800_000_000_000,
            )
            .expect("seed pre-filter MP4 mapping");
        assert_eq!(
            database
                .count_tracks(player_db::LibrarySourceScope::All, None)
                .expect("legacy visible track"),
            1
        );

        database
            .apply_source_scan(
                &root,
                &SourceScanState::Incomplete {
                    reason: "partial enumeration".to_owned(),
                },
                &[],
                &[],
                &[],
                1_800_000_000_001,
            )
            .expect("incomplete sync");
        assert_eq!(
            database
                .count_tracks(player_db::LibrarySourceScope::All, None)
                .expect("incomplete retains track"),
            1
        );

        let scan = WindowsMediaIndex::new().scan(&root);
        assert_eq!(scan.state, SourceScanState::Complete);
        assert!(scan.errors.is_empty());
        assert!(scan.tracks.is_empty());
        database
            .apply_source_scan(
                &root,
                &scan.state,
                &scan.tracks,
                &scan.tracks,
                &scan.errors,
                1_800_000_000_002,
            )
            .expect("complete folder reconciliation");
        assert_eq!(
            database
                .count_tracks(player_db::LibrarySourceScope::All, None)
                .expect("video is hidden"),
            0
        );
        let page = database
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("read library");
        assert_eq!(page.total_count, 0);

        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn folder_and_playlist_scans_skip_mp4_without_errors_and_keep_m4a_aac() {
        let base = temp_path("mp4-filter");
        fs::create_dir_all(&base).expect("create scan root");
        let video = base.join("演唱會片段.mp4");
        let m4a = base.join("歌曲.M4A");
        let aac = base.join("語音.AAC");
        let missing_video = base.join("不存在.mp4");
        fs::write(&video, b"video candidate").expect("write MP4 candidate");
        fs::write(&m4a, b"M4A audio candidate").expect("write M4A candidate");
        fs::write(&aac, b"AAC audio candidate").expect("write AAC candidate");

        let source_id = player_core::SourceId::new();
        let root = LibraryRoot {
            id: source_id,
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "mp4-filter".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let folder_scan = WindowsMediaIndex::new().scan(&root);
        assert_eq!(folder_scan.state, SourceScanState::Complete);
        assert!(
            folder_scan.errors.is_empty(),
            "excluded video is not a metadata error"
        );
        assert_eq!(folder_scan.tracks.len(), 2);
        assert!(folder_scan.tracks.iter().any(|track| matches!(
            &track.locator,
            MediaLocator::FileSystem(path)
                if path.file_name().is_some_and(|name| name == "歌曲.M4A")
        )));
        assert!(folder_scan.tracks.iter().any(|track| matches!(
            &track.locator,
            MediaLocator::FileSystem(path)
                if path.file_name().is_some_and(|name| name == "語音.AAC")
        )));

        let playlist_scan = WindowsMediaIndex::new().scan_playlist_paths(
            source_id,
            &[video.clone(), missing_video, m4a.clone(), aac.clone()],
        );
        assert_eq!(playlist_scan.state, SourceScanState::Complete);
        assert!(
            playlist_scan.errors.is_empty(),
            "excluded video must not preserve an old mapping as an item error"
        );
        assert_eq!(playlist_scan.tracks.len(), 2);
        assert!(!playlist_scan.tracks.iter().any(|track| matches!(
            &track.locator,
            MediaLocator::FileSystem(path)
                if path.file_name().is_some_and(|name| name == "演唱會片段.mp4")
        )));
        assert!(playlist_scan.tracks.iter().any(|track| matches!(
            &track.locator,
            MediaLocator::FileSystem(path)
                if path.file_name().is_some_and(|name| name == "歌曲.M4A")
        )));
        assert!(playlist_scan.tracks.iter().any(|track| matches!(
            &track.locator,
            MediaLocator::FileSystem(path)
                if path.file_name().is_some_and(|name| name == "語音.AAC")
        )));

        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn filesystem_and_unverified_systemindex_fallback_share_internal_track_id() {
        let base = temp_path("identity");
        fs::create_dir_all(&base).expect("root");
        write_tagged_mp3(&base.join("same.mp3"), "Indexed track");

        let mut db = Database::open_in_memory().expect("database");
        let filesystem_root = add_root(&db, base.clone());
        let mut index = WindowsMediaIndex::new();
        player_core::SyncEngine::sync(&filesystem_root, &mut index, &mut db, 1)
            .expect("filesystem sync");
        let first = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("first page");
        assert_eq!(first.items.len(), 1);
        let track_id = first.items[0].id;
        db.set_user_override(track_id, UserMetadataField::Title, Some("手動資料"))
            .expect("user override");

        let systemindex_root = db
            .add_library_root(
                MediaSourceKind::WindowsSystemIndex,
                "Windows Search root",
                MediaLocator::FileSystem(base.clone()),
            )
            .expect("SystemIndex root");
        let fallback = player_core::SyncEngine::sync(&systemindex_root, &mut index, &mut db, 2)
            .expect("filesystem fallback sync");
        assert_eq!(fallback.state, Some(SourceScanState::Complete));
        assert!(
            fallback.source_errors.is_empty(),
            "SystemIndex capability diagnostics do not count as source errors"
        );

        let after_fallback = db
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                player_core::ListTracksQuery {
                    offset: 0,
                    limit: 10,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .expect("fallback page");
        assert_eq!(after_fallback.items.len(), 1);
        assert_eq!(after_fallback.items[0].id, track_id);
        assert_eq!(after_fallback.items[0].title.as_deref(), Some("手動資料"));
        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn metadata_shape_keeps_available_fields() {
        let base = temp_path("metadata");
        fs::create_dir_all(&base).expect("root");
        let path = base.join("track.mp3");
        write_tagged_mp3(&path, "Title");
        let root = LibraryRoot {
            id: player_core::SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "metadata".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let record = WindowsMediaIndex::new().scan(&root).tracks.pop();
        assert!(record.is_some());
        let mut index = WindowsMediaIndex::new();
        let metadata = index
            .read_metadata(&record.expect("record"))
            .expect("metadata");
        assert_eq!(metadata.title.as_deref(), Some("Title"));
        assert_eq!(metadata.artist.as_deref(), Some("測試演出者"));
        assert_eq!(metadata.track_number, Some(4));
        assert_eq!(metadata.album_artist.as_deref(), Some("專輯演出者"));
        assert_eq!(metadata.release_date.as_deref(), Some("2024-02-29"));
        assert_eq!(metadata.year, Some(2024));
        assert_eq!(metadata.codec.as_deref(), Some("MP3"));
        assert_eq!(metadata.bit_depth, None, "MP3 is lossy");

        let unpadded_path = base.join("unpadded.mp3");
        write_tagged_mp3_with_date(&unpadded_path, "Unpadded", "2024-2-9");
        let record = WindowsMediaIndex::new()
            .scan(&root)
            .tracks
            .into_iter()
            .find(|record| {
                matches!(
                    &record.locator,
                    MediaLocator::FileSystem(path)
                        if path.file_name().is_some_and(|name| {
                            name.to_string_lossy().eq_ignore_ascii_case("unpadded.mp3")
                        })
                )
            })
            .expect("unpadded-date record");
        let unpadded = index.read_metadata(&record).expect("unpadded metadata");
        assert_eq!(unpadded.release_date.as_deref(), Some("2024-02-09"));
        assert_eq!(unpadded.year, Some(2024));
        fs::remove_dir_all(base).expect("cleanup");
    }

    #[test]
    fn mpeg_layer_names_remain_distinct() {
        assert_eq!(mpeg_codec_name(Layer::Layer1), "MP1");
        assert_eq!(mpeg_codec_name(Layer::Layer2), "MP2");
        assert_eq!(mpeg_codec_name(Layer::Layer3), "MP3");
    }

    #[test]
    fn source_bit_depth_is_reported_only_for_unambiguous_lossless_formats() {
        assert_eq!(
            reliable_source_bit_depth(FileType::Flac, Some(24)),
            Some(24)
        );
        assert_eq!(reliable_source_bit_depth(FileType::Ape, Some(16)), Some(16));
        assert_eq!(reliable_source_bit_depth(FileType::Mpeg, Some(16)), None);
        assert_eq!(reliable_source_bit_depth(FileType::Aac, Some(24)), None);
        assert_eq!(reliable_source_bit_depth(FileType::WavPack, Some(24)), None);
        assert_eq!(reliable_source_bit_depth(FileType::Mp4, Some(24)), None);
        assert_eq!(reliable_source_bit_depth(FileType::Flac, Some(4)), None);
    }

    /// WavPack depths come from the decoder; a lossy hybrid file has none.
    #[test]
    fn wavpack_labels_and_depths_come_from_the_decoder() {
        let reference = |name: &str| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../player-wavpack/tests/reference")
                .join(name)
        };
        let summary = |name: &str| read_wavpack_summary(&reference(name));
        let lossless = |depth: u8, rate: u32| {
            Some(player_wavpack::WavPackSummary {
                codec: "WVLL",
                sample_rate_hz: rate,
                bit_depth: Some(depth),
            })
        };
        assert_eq!(summary("int16_stereo.wv"), lossless(16, 48_000));
        assert_eq!(summary("int24_stereo.wv"), lossless(24, 48_000));
        assert_eq!(summary("int32_stereo.wv"), lossless(32, 48_000));
        assert_eq!(summary("f32_stereo.wv"), lossless(32, 48_000));
        // A hybrid lossy file is `WVL` and has no depth.
        assert_eq!(
            summary("hybrid_lossy_int16_stereo.wv"),
            Some(player_wavpack::WavPackSummary {
                codec: "WVL",
                sample_rate_hz: 48_000,
                bit_depth: None
            })
        );
        assert_eq!(summary("missing.wv"), None);
    }

    /// Whole songs from the official encoder (`MOE_WV_MANIFEST`, run by hand with `-- --ignored`).
    #[test]
    #[ignore = "needs MOE_WV_MANIFEST"]
    fn official_encoder_files_report_their_bit_depth() {
        let manifest = fs::read_to_string(std::env::var("MOE_WV_MANIFEST").unwrap()).unwrap();
        for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
            let cols: Vec<&str> = line.split('\t').collect();
            let (name, lossless_wv) = (cols[0], cols[5] == "1");
            let expected = if !lossless_wv {
                None
            } else if name.contains("float32") || name.contains("int32") {
                Some(32)
            } else if name.starts_with("cd16") {
                Some(16)
            } else {
                Some(24)
            };
            let summary = read_wavpack_summary(Path::new(cols[1])).unwrap();
            println!(
                "{name}: {} {} Hz, bit depth {:?}",
                summary.codec, summary.sample_rate_hz, summary.bit_depth
            );
            assert_eq!(summary.bit_depth, expected, "{name}");
            assert_eq!(
                summary.codec,
                if cols[5] == "1" { "WVLL" } else { "WVL" },
                "{name}"
            );
        }
    }

    fn write_wav_with_format(path: &Path, format: &[u8]) {
        let mut bytes = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        bytes.extend_from_slice(&(format.len() as u32).to_le_bytes());
        bytes.extend_from_slice(format);
        if !format.len().is_multiple_of(2) {
            bytes.push(0);
        }
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        fs::write(path, bytes).expect("write WAV fixture");
    }

    fn pcm_wav_format(format_tag: u16, container_depth: u16) -> Vec<u8> {
        let mut format = Vec::new();
        format.extend_from_slice(&format_tag.to_le_bytes());
        format.extend_from_slice(&2_u16.to_le_bytes()); // channels
        format.extend_from_slice(&48_000_u32.to_le_bytes());
        format.extend_from_slice(&192_000_u32.to_le_bytes());
        format.extend_from_slice(&4_u16.to_le_bytes());
        format.extend_from_slice(&container_depth.to_le_bytes());
        format
    }

    #[test]
    fn wav_bit_depth_is_reported_only_for_pcm_and_ieee_float() {
        let base = temp_path("wav-bit-depth");
        fs::create_dir_all(&base).expect("create fixture directory");

        let pcm = base.join("pcm.wav");
        write_wav_with_format(&pcm, &pcm_wav_format(1, 24));
        assert_eq!(read_lossless_wav_bit_depth(&pcm), Some(24));

        let float = base.join("float.wav");
        write_wav_with_format(&float, &pcm_wav_format(3, 32));
        assert_eq!(read_lossless_wav_bit_depth(&float), Some(32));

        let adpcm = base.join("adpcm.wav");
        write_wav_with_format(&adpcm, &pcm_wav_format(0x0011, 4));
        assert_eq!(read_lossless_wav_bit_depth(&adpcm), None);

        fs::remove_dir_all(base).expect("cleanup fixture directory");
    }

    #[test]
    fn wav_extensible_uses_valid_bits_and_rejects_unknown_subtypes() {
        let base = temp_path("wav-extensible-bit-depth");
        fs::create_dir_all(&base).expect("create fixture directory");
        let path = base.join("valid-20-in-24.wav");

        let mut format = pcm_wav_format(0xfffe, 24);
        format.extend_from_slice(&22_u16.to_le_bytes()); // cbSize
        format.extend_from_slice(&20_u16.to_le_bytes()); // valid bits per sample
        format.extend_from_slice(&3_u32.to_le_bytes()); // channel mask
        format.extend_from_slice(&1_u32.to_le_bytes()); // PCM subtype
        format.extend_from_slice(&[0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113]);
        write_wav_with_format(&path, &format);
        assert_eq!(read_lossless_wav_bit_depth(&path), Some(20));

        format[24..28].copy_from_slice(&2_u32.to_le_bytes()); // ADPCM subtype
        write_wav_with_format(&path, &format);
        assert_eq!(read_lossless_wav_bit_depth(&path), None);
        fs::remove_dir_all(base).expect("cleanup fixture directory");
    }

    #[test]
    fn wav_metadata_scan_carries_pcm_bit_depth_to_track_metadata() {
        let base = temp_path("wav-metadata-bit-depth");
        fs::create_dir_all(&base).expect("create fixture directory");
        let wav_path = base.join("pcm-24bit.wav");
        write_wav_with_format(&wav_path, &pcm_wav_format(1, 24));
        let track = player_core::MediaTrackRecord {
            identity: TrackIdentity {
                source_id: SourceId::new(),
                source_item_id: wav_path.to_string_lossy().into_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(wav_path),
            fingerprint: FileFingerprint {
                size_bytes: 0,
                modified_at_utc_ms: None,
                created_at_utc_ms: None,
            },
            metadata: None,
        };
        let mut index = WindowsMediaIndex::new();
        let metadata = index.read_metadata(&track).expect("WAV metadata");
        assert_eq!(metadata.bit_depth, Some(24));
        fs::remove_dir_all(base).expect("cleanup fixture directory");
    }

    #[test]
    fn audio_extension_filter_accepts_mka_and_ogg_audio_but_rejects_video() {
        for path in ["track.mka", "track.OGG", "track.oga", "track.opus"] {
            assert!(is_supported_audio_file(Path::new(path)), "{path}");
        }
        for path in ["movie.mkv", "movie.mp4"] {
            assert!(!is_supported_audio_file(Path::new(path)), "{path}");
        }
        let extensions = supported_audio_extensions();
        for extension in [".mka", ".ogg", ".oga", ".opus"] {
            assert!(extensions
                .iter()
                .any(|item| item.eq_ignore_ascii_case(extension)));
        }
        assert!(!extensions
            .iter()
            .any(|item| item.eq_ignore_ascii_case(".mkv")));
        assert!(!extensions
            .iter()
            .any(|item| item.eq_ignore_ascii_case(".mp4")));
    }

    #[test]
    fn matroska_audio_metadata_reads_container_tags_and_codec_properties() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("player-audio-windows")
            .join("tests")
            .join("fixtures")
            .join("metadata-smoke.mka");
        let track = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: SourceId::new(),
                source_item_id: fixture.to_string_lossy().into_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(fixture),
            fingerprint: FileFingerprint {
                size_bytes: 0,
                modified_at_utc_ms: None,
                created_at_utc_ms: None,
            },
            metadata: None,
        };
        let metadata = WindowsMediaIndex::new()
            .read_metadata(&track)
            .expect("Matroska audio metadata");
        assert_eq!(metadata.title.as_deref(), Some("Matroska test title"));
        assert_eq!(metadata.artist.as_deref(), Some("Matroska test artist"));
        assert_eq!(metadata.album.as_deref(), Some("Matroska test album"));
        assert_eq!(metadata.track_number, Some(3));
        assert_eq!(metadata.year, Some(2026));
        assert_eq!(metadata.codec.as_deref(), Some("FLAC"));
        assert_eq!(metadata.sample_rate_hz, Some(24_000));
        assert_eq!(metadata.duration_ms, Some(500));
    }

    #[test]
    fn matroska_bps_tag_is_reported_as_bits_per_second() {
        let tags = [matroska::Tag {
            targets: None,
            simple: vec![matroska::SimpleTag {
                name: "BPS".to_owned(),
                language: None,
                default: true,
                value: Some(matroska::TagValue::String("91353".to_owned())),
            }],
        }];
        assert_eq!(matroska_bitrate_bps(&tags, &[]), Some(91_353));
    }

    #[test]
    fn ogg_vorbis_metadata_remains_readable() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("player-audio-windows")
            .join("tests")
            .join("fixtures")
            .join("metadata-smoke.ogg");
        let track = MediaTrackRecord {
            identity: TrackIdentity {
                source_id: SourceId::new(),
                source_item_id: fixture.to_string_lossy().into_owned(),
                locator_key: None,
            },
            locator: MediaLocator::FileSystem(fixture),
            fingerprint: FileFingerprint {
                size_bytes: 0,
                modified_at_utc_ms: None,
                created_at_utc_ms: None,
            },
            metadata: None,
        };
        let metadata = WindowsMediaIndex::new()
            .read_metadata(&track)
            .expect("OGG Vorbis metadata");
        assert_eq!(metadata.title.as_deref(), Some("Ogg test title"));
        assert_eq!(metadata.artist.as_deref(), Some("Ogg test artist"));
        assert_eq!(metadata.codec.as_deref(), Some("VORBIS"));
        assert_eq!(metadata.sample_rate_hz, Some(24_000));
        assert_eq!(metadata.duration_ms, Some(500));
    }

    #[test]
    fn filesystem_scan_skips_excluded_subfolders() {
        let base = temp_path("scan-exclude");
        let keep = base.join("keep");
        let skip = base.join("skip");
        fs::create_dir_all(&keep).expect("keep");
        fs::create_dir_all(&skip).expect("skip");
        write_tagged_mp3(&keep.join("keep.mp3"), "Keep");
        write_tagged_mp3(&skip.join("skip.mp3"), "Skip");

        let root = LibraryRoot {
            id: player_core::SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "exclude test".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let scan = WindowsMediaIndex::with_excluded_folders([skip.clone()]).scan(&root);
        fs::remove_dir_all(&base).expect("cleanup");

        assert!(matches!(scan.state, SourceScanState::Complete));
        assert_eq!(scan.tracks.len(), 1);
        assert!(matches!(
            &scan.tracks[0].locator,
            MediaLocator::FileSystem(path) if path.file_name().is_some_and(|name| name == "keep.mp3")
        ));
    }

    #[test]
    fn filesystem_scan_reports_multiple_unknown_total_progress_updates() {
        let base = temp_path("scan-progress");
        fs::create_dir_all(&base).expect("root");
        for index in 0..512 {
            fs::write(base.join(format!("track-{index:04}.mp3")), []).expect("audio placeholder");
        }

        let root = LibraryRoot {
            id: player_core::SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "progress test".to_owned(),
            locator: MediaLocator::FileSystem(base.clone()),
            enabled: true,
        };
        let mut progress = Vec::new();
        let cancellation = SyncCancellation::default();
        let scan = WindowsMediaIndex::new().scan_with_progress(
            &root,
            &mut |value| progress.push(value),
            &cancellation,
        );
        fs::remove_dir_all(&base).expect("cleanup");

        assert_eq!(scan.tracks.len(), 512);
        assert!(
            progress.len() > 1,
            "expected intermediate enumeration updates, got {progress:?}"
        );
        assert!(progress.iter().all(|value| value.total.is_none()));
        assert!(progress
            .iter()
            .all(|value| value.unit == MediaScanProgressUnit::FilesystemEntries));
        assert_eq!(progress.last().map(|value| value.processed), Some(512));
    }
}
