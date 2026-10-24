//! Playlist-file sources on real filesystem paths: Windows, and Android under All files access
//! (ADR 0004, ADR 0014). The playlist file is the complete membership list.

#[cfg(any(target_os = "windows", target_os = "android"))]
use std::path::PathBuf;
use std::{fs, time::UNIX_EPOCH};

use player_core::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaTrackRecord, SourceId, SyncEngine,
    SyncReport,
};
use player_db::{Database, PlaylistFileSyncState};
use sha2::{Digest, Sha256};

use crate::{
    playlist_exchange::read_playlist_bytes,
    settings::{SourceEntry, SourceEntryKind},
};

#[cfg(any(target_os = "windows", target_os = "android"))]
use player_core::{SourceScan, TrackMetadata, TrackMetadataError};
#[cfg(target_os = "windows")]
use player_platform_windows::{is_supported_audio_file, is_video_mp4, WindowsMediaIndex};

/// Stable identity key of a native path (Windows UTF-16 key, Android `unix_path_key`).
#[cfg(target_os = "windows")]
pub fn path_key(path: &std::path::Path) -> String {
    player_core::windows_locator_key(path)
}

#[cfg(target_os = "android")]
pub fn path_key(path: &std::path::Path) -> String {
    player_core::unix_path_key(path)
}

#[cfg(target_os = "android")]
fn is_supported_audio_file(path: &std::path::Path) -> bool {
    tauri_plugin_media_index::fs_scan::is_supported_audio_path(path)
}

#[cfg(target_os = "android")]
fn is_video_mp4(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

/// Platform media access used while syncing one playlist file.
#[cfg(any(target_os = "windows", target_os = "android"))]
pub trait PlaylistMediaReader {
    /// Look the listed files up, reporting how many are checked.
    fn scan_paths(
        &mut self,
        source_id: SourceId,
        paths: &[PathBuf],
        progress: &mut dyn FnMut(player_core::MediaScanProgress),
    ) -> SourceScan;
    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError>;
}

#[cfg(target_os = "windows")]
struct WindowsPlaylistReader(WindowsMediaIndex);

#[cfg(target_os = "windows")]
impl PlaylistMediaReader for WindowsPlaylistReader {
    fn scan_paths(
        &mut self,
        source_id: SourceId,
        paths: &[PathBuf],
        progress: &mut dyn FnMut(player_core::MediaScanProgress),
    ) -> SourceScan {
        self.0
            .scan_playlist_paths_with_progress(source_id, paths, progress)
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        self.0.read_metadata(track)
    }
}

#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn source_ids_for_path(
    sources: &[SourceEntry],
    requested_path: &std::path::Path,
) -> Result<(SourceId, Option<player_core::PlaylistId>, PathBuf), String> {
    let canonical = fs::canonicalize(requested_path)
        .map_err(|error| format!("無法確認播放清單路徑：{error}"))?;
    let requested_key = path_key(&canonical);
    if let Some(source) = sources.iter().find(|source| {
        let SourceEntryKind::PlaylistFile { .. } = &source.kind else {
            return false;
        };
        source
            .path()
            .to_path_buf()
            .ok()
            .map(|path| fs::canonicalize(&path).unwrap_or(path))
            .is_some_and(|path| path_key(&path) == requested_key)
    }) {
        if let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind {
            return Ok((source.id, Some(*playlist_id), canonical));
        }
    }
    Ok((SourceId::new(), None, canonical))
}

/// Synchronize a registered playlist-file source on Windows. `None` means the playlist file was
/// unavailable or invalid; in that case neither its saved playlist nor its source mappings are
/// reconciled, and a future startup will retry because the successful fingerprint is unchanged.
#[cfg(target_os = "windows")]
pub fn sync_playlist_file_source(
    database: &mut Database,
    source: &SourceEntry,
    excluded_folders: &[PathBuf],
    synced_at_utc_ms: i64,
    progress: &mut dyn FnMut(player_core::SyncProgress),
) -> Result<Option<SyncReport>, String> {
    let mut reader = WindowsPlaylistReader(WindowsMediaIndex::with_excluded_folders(
        excluded_folders.iter().cloned(),
    ));
    sync_playlist_file_source_with(database, source, synced_at_utc_ms, &mut reader, progress)
}

/// Same as the Windows entry point, with the platform's media reader (Android: real paths).
#[cfg(any(target_os = "windows", target_os = "android"))]
pub fn sync_playlist_file_source_with(
    database: &mut Database,
    source: &SourceEntry,
    synced_at_utc_ms: i64,
    reader: &mut dyn PlaylistMediaReader,
    progress: &mut dyn FnMut(player_core::SyncProgress),
) -> Result<Option<SyncReport>, String> {
    let SourceEntryKind::PlaylistFile {
        playlist_id, path, ..
    } = &source.kind
    else {
        return Ok(None);
    };
    let playlist_path = path.to_path_buf().map_err(|error| error.to_string())?;
    let Ok(metadata) = fs::metadata(&playlist_path) else {
        return Ok(None);
    };
    if !metadata.is_file() {
        return Ok(None);
    }
    let Ok(bytes) = fs::read(&playlist_path) else {
        return Ok(None);
    };
    let modified_at_utc_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    let fingerprint = FileFingerprint {
        size_bytes: metadata.len(),
        modified_at_utc_ms,
        created_at_utc_ms: None,
    };
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let prior_state = database
        .playlist_file_sync_state(source.id)
        .map_err(|error| error.to_string())?;
    let changed = prior_state.as_ref().is_none_or(|state| {
        state.playlist_id != *playlist_id
            || state.locator != MediaLocator::FileSystem(playlist_path.clone())
            || state.fingerprint != fingerprint
            || state.content_sha256 != digest
    });

    let playlist = if changed {
        let Ok(mut playlist) = read_playlist_bytes(&bytes, &playlist_path) else {
            return Ok(None);
        };
        playlist.id = *playlist_id;
        Some(playlist)
    } else {
        None
    };
    let mut playlist = match playlist {
        Some(playlist) => playlist,
        None => database
            .get_playlist(*playlist_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "registered playlist source has no saved playlist projection".to_owned()
            })?,
    };

    let root = LibraryRoot {
        id: source.id,
        kind: player_core::MediaSourceKind::PlaylistFile,
        display_name: source.display_name.clone(),
        locator: MediaLocator::FileSystem(playlist_path.clone()),
        enabled: source.enabled,
    };
    database
        .save_library_root(&root)
        .map_err(|error| error.to_string())?;

    let media_paths = playlist
        .entries
        .iter()
        .filter_map(|entry| match &entry.locator {
            MediaLocator::FileSystem(path) => Some(path.clone()),
            // A playlist file lists local files; a remote track in it is not scanned here.
            MediaLocator::ContentUri(_) | MediaLocator::Remote { .. } => None,
        })
        .collect::<Vec<_>>();
    // The lookup of the listed files runs inside the engine's scan, so a playlist reports its progress the way a folder
    // does (and the tags are read afterwards by the same reader).
    let reader = std::cell::RefCell::new(reader);
    let source_id = source.id;
    let mut index = crate::playlist_scan_index::PlaylistScanIndex::new(
        Box::new(|report| reader.borrow_mut().scan_paths(source_id, &media_paths, report)),
        Box::new(lyric_sidecars_beside),
        Box::new(|track| reader.borrow_mut().read_metadata(track)),
        None,
    );
    let report = SyncEngine::sync_with_progress(&root, &mut index, database, synced_at_utc_ms, |update| {
        progress(update)
    })
    .map_err(|error| error.to_string())?;
    drop(index);

    // Canonical paths are used only to attach the shared TrackId. Keep imported locators intact
    // for lossless export and relinking after a temporarily missing file returns. Persist only
    // when the playlist file changed or a formerly missing entry became resolvable.
    let mut playlist_needs_save = changed;
    let mut resolved = std::collections::HashMap::new();
    for entry in &mut playlist.entries {
        let MediaLocator::FileSystem(path) = &entry.locator else {
            continue;
        };
        if is_video_mp4(path) {
            if entry.track_id.take().is_some() {
                playlist_needs_save = true;
            }
            continue;
        }
        if entry.track_id.is_some() {
            continue;
        }
        if !is_supported_audio_file(path) {
            continue;
        }
        let Ok(canonical) = fs::canonicalize(path) else {
            continue;
        };
        let key = path_key(&canonical);
        let track_id = if let Some(track_id) = resolved.get(&key) {
            *track_id
        } else {
            let track_id = database
                .resolve_track_id_for_locator(&MediaLocator::FileSystem(canonical))
                .map_err(|error| error.to_string())?;
            resolved.insert(key, track_id);
            track_id
        };
        if let Some(track_id) = track_id {
            entry.track_id = Some(track_id);
            playlist_needs_save = true;
        }
    }
    if playlist_needs_save {
        database
            .save_playlist(&playlist)
            .map_err(|error| error.to_string())?;
    }

    if changed {
        database
            .record_playlist_file_sync_state(
                source.id,
                &PlaylistFileSyncState {
                    playlist_id: *playlist_id,
                    locator: MediaLocator::FileSystem(playlist_path),
                    fingerprint,
                    content_sha256: digest,
                },
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(report))
}

/// The `.lrc` files beside these songs (each folder is read once), paired by name without the extension.
#[cfg(any(target_os = "windows", target_os = "android"))]
fn lyric_sidecars_beside(tracks: &[MediaTrackRecord]) -> Vec<player_core::LyricSidecar> {
    let mut by_folder: std::collections::HashMap<PathBuf, Vec<&MediaTrackRecord>> =
        std::collections::HashMap::new();
    for track in tracks {
        if let MediaLocator::FileSystem(path) = &track.locator {
            if let Some(folder) = path.parent() {
                by_folder.entry(folder.to_path_buf()).or_default().push(track);
            }
        }
    }
    let mut found = Vec::new();
    for (folder, songs) in by_folder {
        let Ok(entries) = fs::read_dir(&folder) else {
            continue;
        };
        // One sidecar per song: `.lrc` over `.ass` over `.srt` (ADR 0041).
        let mut lyrics: std::collections::HashMap<String, (usize, PathBuf)> =
            std::collections::HashMap::new();
        for path in entries.flatten().map(|entry| entry.path()) {
            let Some(rank) = path
                .file_name()
                .and_then(|name| player_core::lyric_sidecar_rank(&name.to_string_lossy()))
            else {
                continue;
            };
            let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()) else {
                continue;
            };
            if lyrics.get(&stem).is_none_or(|(kept, _)| *kept > rank) {
                lyrics.insert(stem, (rank, path));
            }
        }
        let lyrics: std::collections::HashMap<String, PathBuf> =
            lyrics.into_iter().map(|(stem, (_, path))| (stem, path)).collect();
        for track in songs {
            let MediaLocator::FileSystem(path) = &track.locator else {
                continue;
            };
            let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()) else {
                continue;
            };
            let Some(lyric_path) = lyrics.get(&stem) else {
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
                modified_at_utc_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .and_then(|duration| i64::try_from(duration.as_millis()).ok()),
            });
        }
    }
    found
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
pub fn sync_playlist_file_source(
    _database: &mut Database,
    _source: &SourceEntry,
    _excluded_folders: &[std::path::PathBuf],
    _synced_at_utc_ms: i64,
    _progress: &mut dyn FnMut(player_core::SyncProgress),
) -> Result<Option<SyncReport>, String> {
    Ok(None)
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use std::{fs, path::PathBuf, time::SystemTime};

    use player_core::{
        FileFingerprint, LibraryRepository, LibraryRoot, ListTracksQuery, MediaLocator,
        MediaSourceKind, MediaTrackRecord, Playlist, PlaylistId, SourceId, SourceScanState,
        TrackIdentity, TrackMetadata,
    };
    use player_db::Database;

    use crate::settings::{SourceEntry, SourceEntryKind, StoredPath};

    use super::{source_ids_for_path, sync_playlist_file_source};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "moemusic-playlist-source-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_tagged_mp3(path: &std::path::Path, title: &str) {
        fn frame(body: &mut Vec<u8>, name: &[u8; 4], text: &str) {
            let mut value = vec![3];
            value.extend(text.as_bytes());
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
        frame(&mut body, b"TIT2", title);
        frame(&mut body, b"TPE1", "測試演出者");
        let size = body.len() as u32;
        let mut bytes = vec![b'I', b'D', b'3', 4, 0, 0];
        bytes.extend([
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ]);
        bytes.extend(body);
        for _ in 0..3 {
            bytes.extend([0xff, 0xfb, 0x90, 0x64]);
            bytes.resize(bytes.len() + 413, 0);
        }
        fs::write(path, bytes).expect("write local MP3 fixture");
    }

    fn playlist_source(
        path: &std::path::Path,
        source_id: SourceId,
        playlist_id: PlaylistId,
    ) -> SourceEntry {
        SourceEntry {
            id: source_id,
            display_name: "來源清單 🎵".to_owned(),
            enabled: true,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id,
                path: StoredPath::from_path(path).expect("store Unicode playlist path"),
                tree_uri: None,
                remote_account: None,
            },
        }
    }

    #[test]
    fn startup_sync_deduplicates_media_updates_metadata_and_preserves_offline_projection() {
        let directory = TestDirectory::new();
        let music = directory.0.join("夜の音樂 東京");
        fs::create_dir_all(&music).expect("create Unicode music directory");
        let song = music.join("花の歌 🌸.mp3");
        write_tagged_mp3(&song, "第一版");
        let playlist_folder = directory.0.join("清單資料夾");
        fs::create_dir_all(&playlist_folder).expect("create playlist folder");
        let playlist_path = playlist_folder.join("我的 清單.m3u8");
        fs::write(
            &playlist_path,
            "#EXTM3U\n#PLAYLIST:測試\n../夜の音樂 東京/花の歌 🌸.mp3\n../夜の音樂 東京/花の歌 🌸.mp3\n../夜の音樂 東京/暫時缺席.mp3\n",
        )
        .expect("write M3U8");
        let source_id = SourceId::new();
        let playlist_id = PlaylistId::new();
        let source = playlist_source(&playlist_path, source_id, playlist_id);
        let mut database = Database::open_in_memory().expect("database");

        let first = sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_000, &mut |_| {})
            .expect("initial sync")
            .expect("playlist is readable");
        assert_eq!(
            first.observed, 1,
            "duplicate positions share one source mapping: {first:#?}"
        );
        assert_eq!(database.count_tracks(None).expect("count tracks"), 1);
        let saved = database
            .get_playlist(playlist_id)
            .expect("read playlist")
            .expect("saved playlist");
        assert_eq!(saved.entries.len(), 3);
        assert_eq!(saved.entries[0].track_id, saved.entries[1].track_id);
        assert_eq!(saved.entries[2].track_id, None);
        let stable_track_id = saved.entries[0].track_id.expect("matched song");

        // Simulate a previously persisted TrackId link from before MP4 filtering was introduced.
        // Seed an old source mapping as well, so the next complete sync must remove the mapping
        // before a playlist page reconciliation can try to attach it again.
        let video_path = music.join("舞台影像.mp4");
        fs::write(&video_path, b"video candidate").expect("write MP4 candidate");
        let video_locator = MediaLocator::FileSystem(video_path.clone());
        let video_file_metadata = fs::metadata(&video_path).expect("MP4 metadata");
        let video_source_item_id = player_core::windows_locator_key(&video_path);
        let video_record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id,
                source_item_id: video_source_item_id.clone(),
                locator_key: Some(video_source_item_id),
            },
            locator: video_locator.clone(),
            fingerprint: FileFingerprint {
                size_bytes: video_file_metadata.len(),
                modified_at_utc_ms: Some(1_800_000_000_000),
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some("legacy video mapping".to_owned()),
                ..TrackMetadata::default()
            }),
        };
        let playlist_root = LibraryRoot {
            id: source_id,
            kind: MediaSourceKind::PlaylistFile,
            display_name: source.display_name.clone(),
            locator: MediaLocator::FileSystem(playlist_path.clone()),
            enabled: true,
        };
        database
            .apply_source_scan(
                &playlist_root,
                &SourceScanState::Incomplete {
                    reason: "legacy mapping fixture".to_owned(),
                },
                std::slice::from_ref(&video_record),
                std::slice::from_ref(&video_record),
                &[],
                1_800_000_000_001,
            )
            .expect("seed old MP4 source mapping without reconciling audio");
        assert_eq!(
            database.count_tracks(None).expect("legacy mapping visible"),
            2
        );
        let video_track_id = database
            .resolve_track_id_for_locator(&video_locator)
            .expect("resolve old MP4 mapping")
            .expect("old MP4 TrackId");
        let mut with_stale_video_link = saved;
        with_stale_video_link
            .entries
            .push(player_core::PlaylistEntry {
                track_id: Some(video_track_id),
                locator: video_locator.clone(),
                title: None,
                duration_ms: None,
            });
        database
            .save_playlist(&with_stale_video_link)
            .expect("seed stale MP4 playlist link");

        write_tagged_mp3(&song, "第二版・中繼資料已更新");
        let second = sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_001, &mut |_| {})
            .expect("unchanged playlist media check")
            .expect("playlist remains readable");
        assert_eq!(second.metadata_reads, 1, "changed audio tags are refreshed");
        assert_eq!(database.count_tracks(None).expect("count tracks"), 1);
        let page = database
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 10,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .expect("read library projection");
        assert_eq!(page.items[0].id, stable_track_id);
        assert_eq!(
            page.items[0].title.as_deref(),
            Some("第二版・中繼資料已更新")
        );
        let after_mp4_cleanup = database
            .get_playlist(playlist_id)
            .expect("read playlist after successful sync")
            .expect("playlist projection remains");
        assert_eq!(after_mp4_cleanup.entries[3].track_id, None);
        assert_eq!(after_mp4_cleanup.entries[3].locator, video_locator);
        assert_eq!(
            database.count_tracks(None).expect("only audio is visible"),
            1
        );
        let playlist_page = database
            .get_playlist_page(playlist_id, 0, 10)
            .expect("read playlist page after reconciliation")
            .expect("playlist page");
        assert_eq!(playlist_page.items[3].track_id, None);
        let after_page_reconciliation = database
            .get_playlist(playlist_id)
            .expect("read playlist after page reconciliation")
            .expect("playlist projection remains");
        assert_eq!(after_page_reconciliation.entries[3].track_id, None);
        assert_eq!(after_page_reconciliation.entries[3].locator, video_locator);

        fs::remove_file(&playlist_path).expect("take playlist source offline");
        assert!(
            sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_002, &mut |_| {})
                .expect("offline source is recoverable")
                .is_none()
        );
        assert_eq!(
            database.count_tracks(None).expect("offline cache remains"),
            1
        );

        fs::write(&playlist_path, "#EXTM3U\n../夜の音樂 東京/暫時缺席.mp3\n")
            .expect("write changed playlist without previous song");
        let changed = sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_003, &mut |_| {})
            .expect("reconcile changed playlist")
            .expect("changed playlist is readable");
        assert_eq!(changed.observed, 0);
        assert_eq!(
            database
                .count_tracks(None)
                .expect("removed mapping is reconciled"),
            0
        );
        let updated = database
            .get_playlist(playlist_id)
            .expect("read updated playlist")
            .expect("playlist exists");
        assert_eq!(updated.entries.len(), 1);
        assert!(matches!(
            updated.entries[0].locator,
            player_core::MediaLocator::FileSystem(_)
        ));

        let recovered = music.join("暫時缺席.mp3");
        write_tagged_mp3(&recovered, "恢復的曲目");
        let recovery = sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_004, &mut |_| {})
            .expect("rescan restored entry")
            .expect("playlist readable");
        assert_eq!(recovery.observed, 1);
        assert_eq!(database.count_tracks(None).expect("restored mapping"), 1);
        let page = database
            .get_playlist_page(playlist_id, 0, 10)
            .expect("reconcile restored playlist link")
            .expect("playlist page");
        assert!(page.items[0].track_id.is_some());

        let root = database
            .library_roots()
            .expect("source projection")
            .into_iter()
            .find(|root| root.id == source_id)
            .expect("playlist source root");
        assert_eq!(root.kind, MediaSourceKind::PlaylistFile);
    }

    #[test]
    fn importing_same_canonical_playlist_path_reuses_both_ids() {
        let directory = TestDirectory::new();
        let playlist_path = directory.0.join("音樂 清單.m3u8");
        fs::write(&playlist_path, "#EXTM3U\n").expect("create playlist file");
        let source_id = SourceId::new();
        let playlist_id = PlaylistId::new();
        let existing = playlist_source(&playlist_path, source_id, playlist_id);
        let alias = directory.0.join(".").join("音樂 清單.m3u8");

        let first = source_ids_for_path(std::slice::from_ref(&existing), &playlist_path)
            .expect("resolve first import");
        let second = source_ids_for_path(std::slice::from_ref(&existing), &alias)
            .expect("resolve repeated import through path alias");
        assert_eq!((first.0, first.1), (source_id, Some(playlist_id)));
        assert_eq!((second.0, second.1), (source_id, Some(playlist_id)));
    }

    #[test]
    fn playlist_digest_detects_same_size_and_modified_time_content_change() {
        let directory = TestDirectory::new();
        let playlist_path = directory.0.join("hash-check.m3u8");
        let original = b"#EXTM3U\nsong-a.mp3\n";
        fs::write(&playlist_path, original).expect("write initial M3U8");
        let original_modified = fs::metadata(&playlist_path)
            .expect("initial metadata")
            .modified()
            .expect("initial mtime");
        let source = playlist_source(&playlist_path, SourceId::new(), PlaylistId::new());
        let mut database = Database::open_in_memory().expect("database");
        sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_000, &mut |_| {})
            .expect("initial source sync");

        let changed = b"#EXTM3U\nsong-b.mp3\n";
        assert_eq!(original.len(), changed.len());
        fs::write(&playlist_path, changed).expect("replace same-length content");
        fs::File::options()
            .write(true)
            .open(&playlist_path)
            .expect("open playlist for timestamp restore")
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .expect("restore original mtime");

        let report = sync_playlist_file_source(&mut database, &source, &[], 1_800_000_000_001, &mut |_| {})
            .expect("digest check")
            .expect("valid changed M3U8");
        assert_eq!(report.observed, 0);
        let saved = database
            .get_playlist(match source.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                SourceEntryKind::Folder { .. } => unreachable!(),
            })
            .expect("read updated playlist")
            .expect("playlist projection");
        assert_eq!(saved.entries.len(), 1);
        assert!(matches!(
            &saved.entries[0].locator,
            player_core::MediaLocator::FileSystem(path) if path.file_name().is_some_and(|name| name == "song-b.mp3")
        ));
    }

    #[test]
    fn first_registration_reuses_only_full_locator_match_of_legacy_hanser_playlist() {
        let directory = TestDirectory::new();
        let playlist_path = directory.0.join("hanser.m3u8");
        fs::write(&playlist_path, "#EXTM3U\n#PLAYLIST:Hanser\nsong.mp3\n").expect("write playlist");
        let database = Database::open_in_memory().expect("database");
        let mut legacy = Playlist::new("Hanser");
        legacy.entries.push(player_core::PlaylistEntry {
            track_id: None,
            locator: player_core::MediaLocator::FileSystem(directory.0.join("song.mp3")),
            title: None,
            duration_ms: None,
        });
        let legacy_id = legacy.id;
        database
            .save_playlist(&legacy)
            .expect("save existing static playlist");
        let (source_id, registered_id, canonical_path) =
            source_ids_for_path(&[], &playlist_path).expect("new path has new source id");
        assert!(registered_id.is_none());

        let parsed = crate::playlist_exchange::read_playlist_file(&canonical_path)
            .expect("parse imported M3U8");
        let parsed_locators = parsed
            .entries
            .iter()
            .map(|entry| entry.locator.clone())
            .collect::<Vec<_>>();
        let playlist_id = database
            .unique_playlist_id_matching_locators(&parsed.name, &parsed_locators)
            .expect("compare complete legacy locators")
            .unwrap_or(parsed.id);
        assert_eq!(playlist_id, legacy_id);
        let imported = crate::playlist_exchange::import_playlist_file_with_id(
            &database,
            &canonical_path,
            playlist_id,
        )
        .expect("upgrade existing playlist projection");
        assert_eq!(imported.playlist.id, legacy_id);
        assert_eq!(database.list_playlists().expect("list playlists").len(), 1);
        assert_ne!(source_id, SourceId::new());

        let other_path = directory.0.join("other-hanser.m3u8");
        fs::write(&other_path, "#EXTM3U\n#PLAYLIST:Hanser\nother-song.mp3\n")
            .expect("write same-name different-content playlist");
        let other = crate::playlist_exchange::read_playlist_file(&other_path)
            .expect("parse unrelated list");
        let other_locators = other
            .entries
            .iter()
            .map(|entry| entry.locator.clone())
            .collect::<Vec<_>>();
        assert!(database
            .unique_playlist_id_matching_locators(&other.name, &other_locators)
            .expect("reject same-name unrelated legacy playlist")
            .is_none());
        let unrelated = crate::playlist_exchange::import_playlist_file_with_id(
            &database,
            &other_path,
            other.id,
        )
        .expect("save new same-name list without overwriting legacy list");
        assert_ne!(unrelated.playlist.id, legacy_id);
        assert_eq!(
            database.list_playlists().expect("both lists remain").len(),
            2
        );
        let preserved_locator = database
            .get_playlist(legacy_id)
            .expect("legacy playlist remains readable")
            .expect("legacy list preserved")
            .entries[0]
            .locator
            .clone();
        assert!(matches!(
            preserved_locator,
            player_core::MediaLocator::FileSystem(path)
                if player_core::windows_locator_key(&path)
                    == player_core::windows_locator_key(&directory.0.join("song.mp3"))
        ));
    }
}
