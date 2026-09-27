use std::{
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use lofty::{
    file::EXTENSIONS,
    prelude::{Accessor, AudioFile, TaggedFileExt},
    tag::ItemKey,
};
use player_core::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaSourceError, MediaSourceKind,
    MediaTrackRecord, SourceScan, SourceScanState, TrackIdentity, TrackMetadata,
    TrackMetadataError,
};
use std::os::windows::ffi::{OsStrExt, OsStringExt};

/// Windows media index provider. Until SystemIndex can prove full root coverage and freshness,
/// this provider uses a complete filesystem walk. A zero-result Windows Search query is never
/// treated as a complete empty library.
#[derive(Default)]
pub struct WindowsMediaIndex;

impl WindowsMediaIndex {
    pub fn new() -> Self {
        Self
    }
}

impl MediaIndex for WindowsMediaIndex {
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        let mut scan = empty_scan(root.id);
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
            MediaLocator::ContentUri(_) => {
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

        if root.kind == MediaSourceKind::WindowsSystemIndex {
            scan.errors.push(MediaSourceError {
                source_item_id: None,
                message: crate::system_index::diagnostic(&canonical_root),
            });
        }

        let mut directories = vec![canonical_root.clone()];
        let mut opened_root = false;
        let mut complete = true;
        while let Some(directory) = directories.pop() {
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
                        source_item_id: Some(windows_locator_key(&directory)),
                        message: format!("could not enumerate a subdirectory: {error}"),
                    });
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
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
                    directories.push(entry_path);
                    continue;
                }
                if !entry_type.is_file() || !is_supported_audio_file(&entry_path) {
                    continue;
                }

                let metadata = match entry.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
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
                                source_item_id: Some(windows_locator_key(&entry_path)),
                                message: "file modification time is outside the supported UTC epoch range".to_owned(),
                            });
                            None
                        }
                    },
                    Err(error) => {
                        complete = false;
                        scan.errors.push(MediaSourceError {
                            source_item_id: Some(windows_locator_key(&entry_path)),
                            message: format!("could not read file modification time: {error}"),
                        });
                        None
                    }
                };

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
                    },
                    metadata: None,
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
            MediaLocator::ContentUri(_) => {
                return Err(TrackMetadataError {
                    message: "Windows metadata reader requires a filesystem path".to_owned(),
                })
            }
        };

        let tagged_file = lofty::read_from_path(path).map_err(|error| TrackMetadataError {
            message: format!("could not parse audio metadata: {error}"),
        })?;
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        let properties = tagged_file.properties();
        let text = |value: Option<std::borrow::Cow<'_, str>>| value.map(|value| value.into_owned());

        Ok(TrackMetadata {
            title: tag.and_then(|tag| text(tag.title())),
            artist: tag.and_then(|tag| text(tag.artist())),
            album: tag.and_then(|tag| text(tag.album())),
            album_artist: tag
                .and_then(|tag| tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)),
            track_number: tag.and_then(|tag| tag.track()),
            disc_number: tag.and_then(|tag| tag.disk()),
            duration_ms: Some(properties.duration().as_millis().min(u64::MAX as u128) as u64),
            codec: Some(format!("{:?}", tagged_file.file_type())),
            bitrate_bps: properties
                .audio_bitrate()
                .map(|kbps| kbps.saturating_mul(1000)),
            sample_rate_hz: properties.sample_rate(),
        })
    }
}

/// Return a stable, lossless Windows locator key. This is an adapter comparison token, not a
/// display path or a path that can be passed to a filesystem API. UTF-16 code units are hex
/// encoded so even unpaired surrogates remain representable in SQLite's UTF-8 text column.
///
/// Callers should pass absolute paths with `.` and `..` already resolved. Separators and the
/// extended-length prefix are normalized; component case is preserved to avoid collisions in
/// Windows directories configured for case-sensitive lookup.
pub fn windows_locator_key(path: &Path) -> String {
    let mut units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    for unit in &mut units {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
    normalize_verbatim_prefix(&mut units);
    if units.len() >= 2 && units[1] == b':' as u16 && units[0] <= 0x7f {
        units[0] = (units[0] as u8).to_ascii_uppercase() as u16;
    }

    let mut key = String::with_capacity("windows-u16-v1:".len() + units.len() * 4);
    key.push_str("windows-u16-v1:");
    for unit in units {
        use std::fmt::Write as _;
        let _ = write!(key, "{unit:04x}");
    }
    key
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

fn is_supported_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

fn system_time_to_utc_ms(time: SystemTime) -> Option<i64> {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_millis()).ok(),
        Err(error) => i64::try_from(error.duration().as_millis())
            .ok()
            .and_then(i64::checked_neg),
    }
}

fn to_extended_path(path: &Path) -> io::Result<PathBuf> {
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

fn normalize_verbatim_prefix(units: &mut Vec<u16>) {
    const VERBATIM: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    if units.len() >= 8
        && units[..4] == VERBATIM
        && units[4..8]
            .iter()
            .zip(UNC)
            .all(|(left, right)| ascii_u16_eq_ignore_case(*left, right))
    {
        let remainder = units[8..].to_vec();
        *units = [b'\\' as u16, b'\\' as u16]
            .into_iter()
            .chain(remainder)
            .collect();
    } else if units.len() >= 4 && units[..4] == VERBATIM {
        units.drain(..4);
    }
}

fn ascii_u16_eq_ignore_case(left: u16, right: u16) -> bool {
    left <= 0x7f && right <= 0x7f && (left as u8).eq_ignore_ascii_case(&(right as u8))
}

fn has_prefix(units: &[u16], prefix: &[u16]) -> bool {
    units.len() >= prefix.len() && units[..prefix.len()] == *prefix
}

#[cfg(test)]
mod tests {
    use super::{to_extended_path, windows_locator_key, WindowsMediaIndex};
    use player_core::{
        LibraryRoot, MediaIndex, MediaLocator, MediaSourceKind, SourceScanState, UserMetadataField,
    };
    use player_db::Database;
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

    fn write_tagged_mp3(path: &Path, title: &str) {
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
            .list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
            .expect("page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title.as_deref(), Some("花の歌"));
        assert_eq!(page.items[0].artist.as_deref(), Some("測試演出者"));
        assert!(page.items[0].duration_ms.unwrap_or_default() > 0);

        fs::remove_dir_all(to_extended_path(&base).expect("extended root")).expect("cleanup");
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
            .list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
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
        assert_eq!(after_delete.applied.removed_source_mappings, 1);
        assert_eq!(after_delete.metadata_reads, 1);
        assert!(after_delete.metadata_errors.is_empty());
        let remaining = db
            .list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
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
            db.list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
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
            .list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
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
        assert!(fallback
            .source_errors
            .iter()
            .any(|error| error.message.contains("filesystem fallback")));

        let after_fallback = db
            .list_tracks_page(player_core::ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
            })
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
        fs::remove_dir_all(base).expect("cleanup");
    }
}
