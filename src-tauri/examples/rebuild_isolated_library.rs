//! One-off fixed-input repair workflow for the isolated 2026-10-04 library rebuild.
//! This is not a general product tool: it reads the current settings for reference,
//! scans only the hard-coded D:\\Music sources, and refuses to overwrite its fixed
//! candidate directory under `target/`.

#[cfg(windows)]
mod windows_only {
    use std::{
        error::Error,
        fs::{self, OpenOptions},
        io::Write,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use player_core::{
        parse_m3u8, FileFingerprint, LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator,
        MediaSourceKind, MediaTrackRecord, PlaylistId, SourceId, SourceScan, SourceScanState,
        SyncEngine, TrackMetadata, TrackMetadataError,
    };
    use player_db::{Database, PlaylistFileSyncState};
    use player_platform_windows::WindowsMediaIndex;
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};

    const MUSIC_PATH: &str = r"D:\Music";
    const PLAYLIST_PATH: &str = r"D:\Music\Playlists\hanser.m3u8";
    const MUSIC_SOURCE_ID: &str = "bac66c48-6509-47be-8a46-c5f3b73cbd55";
    const PLAYLIST_SOURCE_ID: &str = "68d441f7-62af-48d4-b9ca-7d0a22625f23";
    const PLAYLIST_ID: &str = "17daacf1-a408-4e2b-b6bd-72a3fe8921c7";
    const EXPECTED_PLAYLIST_ENTRIES: usize = 177;
    const EXPECTED_FANCLUB_FILE: &str = "泠鳶yousa,hanser - 1 2 FanClub.m4a";
    const CANDIDATE_DIRECTORY: &str = "library-rebuild-candidate-20261004T103732Z";

    pub(super) fn run() -> Result<(), Box<dyn Error>> {
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .ok_or("src-tauri must be inside the project root")?
            .to_path_buf();
        let target_root = workspace.join("target");
        let candidate_dir = target_root.join(CANDIDATE_DIRECTORY);
        if candidate_dir.exists() {
            return Err(format!(
                "refusing to overwrite existing candidate directory: {}",
                candidate_dir.display()
            )
            .into());
        }

        let music_path = PathBuf::from(MUSIC_PATH);
        let playlist_path = PathBuf::from(PLAYLIST_PATH);
        let canonical_music = fs::canonicalize(&music_path)?;
        let canonical_playlist = fs::canonicalize(&playlist_path)?;
        if !canonical_playlist.starts_with(&canonical_music) {
            return Err("playlist must remain inside D:\\Music".into());
        }
        let playlist_file_metadata = fs::metadata(&canonical_playlist)?;
        if !playlist_file_metadata.is_file() {
            return Err("hanser.m3u8 is not a regular file".into());
        }
        let playlist_bytes = fs::read(&canonical_playlist)?;
        let mut playlist = parse_m3u8(&playlist_bytes, &canonical_playlist)?;
        if playlist.entries.len() != EXPECTED_PLAYLIST_ENTRIES {
            return Err(format!(
                "expected {EXPECTED_PLAYLIST_ENTRIES} playlist slots, found {}",
                playlist.entries.len()
            )
            .into());
        }
        playlist.id = PlaylistId::parse(PLAYLIST_ID)?;

        let fanclub_path = playlist
            .entries
            .iter()
            .find_map(|entry| match &entry.locator {
                MediaLocator::FileSystem(path)
                    if path
                        .file_name()
                        .is_some_and(|name| name == EXPECTED_FANCLUB_FILE) =>
                {
                    Some(path.clone())
                }
                _ => None,
            })
            .ok_or("FanClub file name is absent from playlist entries")?;
        if !fanclub_path.is_file() {
            return Err(format!(
                "expected FanClub source file is missing: {}",
                fanclub_path.display()
            )
            .into());
        }

        let app_data = PathBuf::from(std::env::var_os("APPDATA").ok_or("APPDATA is unavailable")?);
        let settings_input = app_data
            .join("com.moemusicplayer.app")
            .join("settings.json");
        let current_settings: Value = serde_json::from_slice(&fs::read(&settings_input)?)?;
        let settings_version = current_settings
            .get("schemaVersion")
            .and_then(Value::as_u64)
            .ok_or("current settings must contain schemaVersion")?;
        if settings_version > 5 {
            return Err(
                format!("settings schema {settings_version} is newer than this tool").into(),
            );
        }

        fs::create_dir(&candidate_dir)?;
        let database_path = candidate_dir.join("moemusicplayer.sqlite3");
        let mut database = Database::open(&database_path)?;
        let music_id = SourceId::parse(MUSIC_SOURCE_ID)?;
        let playlist_source_id = SourceId::parse(PLAYLIST_SOURCE_ID)?;
        let playlist_id = PlaylistId::parse(PLAYLIST_ID)?;
        let music_root = LibraryRoot {
            id: music_id,
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "Music".to_owned(),
            locator: MediaLocator::FileSystem(music_path.clone()),
            enabled: true,
        };
        database.save_library_root(&music_root)?;

        let synced_at_utc_ms = utc_now_ms()?;
        let mut windows_index = WindowsMediaIndex::new();
        let music_report = SyncEngine::sync_with_progress(
            &music_root,
            &mut windows_index,
            &mut database,
            synced_at_utc_ms,
            |progress| {
                println!(
                "music stage={:?} processed={} total={:?} observed={} metadata_reads={} unchanged={} errors={}",
                progress.stage,
                progress.processed,
                progress.total,
                progress.observed,
                progress.metadata_reads,
                progress.unchanged,
                progress.error_count,
            );
            },
        )?;
        if music_report.state != Some(SourceScanState::Complete) {
            return Err(format!("Music scan was not complete: {:?}", music_report.state).into());
        }

        let playlist_root = LibraryRoot {
            id: playlist_source_id,
            kind: MediaSourceKind::PlaylistFile,
            display_name: "hanser.m3u8".to_owned(),
            locator: MediaLocator::FileSystem(canonical_playlist.clone()),
            enabled: true,
        };
        database.save_library_root(&playlist_root)?;
        let playlist_paths = playlist
            .entries
            .iter()
            .map(|entry| match &entry.locator {
                MediaLocator::FileSystem(path) => Ok(path.clone()),
                MediaLocator::ContentUri(_) => {
                    Err("Windows playlist unexpectedly contains a content URI")
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut playlist_index = WindowsMediaIndex::new();
        let playlist_scan = playlist_index.scan_playlist_paths(playlist_source_id, &playlist_paths);
        if playlist_scan.state != SourceScanState::Complete {
            return Err(
                format!("playlist scan was not complete: {:?}", playlist_scan.state).into(),
            );
        }
        let mut playlist_source_index = PlaylistSourceIndex {
            scan: Some(playlist_scan),
            windows: playlist_index,
        };
        let playlist_report = SyncEngine::sync_with_progress(
            &playlist_root,
            &mut playlist_source_index,
            &mut database,
            utc_now_ms()?,
            |progress| {
                println!(
                "playlist stage={:?} processed={} total={:?} observed={} metadata_reads={} unchanged={} errors={}",
                progress.stage,
                progress.processed,
                progress.total,
                progress.observed,
                progress.metadata_reads,
                progress.unchanged,
                progress.error_count,
            );
            },
        )?;
        if playlist_report.state != Some(SourceScanState::Complete) {
            return Err(format!(
                "playlist sync was not complete: {:?}",
                playlist_report.state
            )
            .into());
        }

        for entry in &mut playlist.entries {
            entry.track_id = match &entry.locator {
                MediaLocator::FileSystem(path) => database
                    .resolve_track_id_for_locator(&MediaLocator::FileSystem(path.clone()))?,
                MediaLocator::ContentUri(_) => None,
            };
        }
        let unmatched = playlist
            .entries
            .iter()
            .filter(|entry| entry.track_id.is_none())
            .count();
        if unmatched != 0 {
            return Err(format!("playlist still has {unmatched} unmatched slots").into());
        }
        database.save_playlist(&playlist)?;

        let playlist_modified_at_utc_ms = playlist_file_metadata
            .modified()
            .ok()
            .and_then(system_time_to_utc_ms);
        let fingerprint = FileFingerprint {
            size_bytes: playlist_file_metadata.len(),
            modified_at_utc_ms: playlist_modified_at_utc_ms,
        };
        let digest: [u8; 32] = Sha256::digest(&playlist_bytes).into();
        database.record_playlist_file_sync_state(
            playlist_source_id,
            &PlaylistFileSyncState {
                playlist_id,
                locator: MediaLocator::FileSystem(canonical_playlist.clone()),
                fingerprint,
                content_sha256: digest,
            },
        )?;

        let library_count = database
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 1,
                query: None,
                field_filter: None,
            })?
            .total_count;
        if library_count != music_report.observed {
            return Err(format!(
                "database contains {library_count} tracks, Music scan observed {}",
                music_report.observed
            )
            .into());
        }
        let stored_playlist = database
            .get_playlist(playlist_id)?
            .ok_or("saved playlist could not be reopened")?;
        if stored_playlist.entries.len() != EXPECTED_PLAYLIST_ENTRIES
            || stored_playlist
                .entries
                .iter()
                .any(|entry| entry.track_id.is_none())
        {
            return Err("persisted playlist did not retain all 177 matched slots".into());
        }

        let output_settings = prepare_candidate_settings(
            current_settings,
            music_id.to_string().as_str(),
            playlist_source_id.to_string().as_str(),
            playlist_id.to_string().as_str(),
            &music_path,
            &canonical_playlist,
        )?;
        write_new_file(&candidate_dir.join("settings.json"), &output_settings)?;

        println!(
        "candidate_ready path={} database_tracks={} music_observed={} music_metadata_reads={} playlist_slots={} playlist_unique_tracks={} playlist_metadata_reads={} playlist_source_errors={} music_metadata_errors={}",
        candidate_dir.display(),
        library_count,
        music_report.observed,
        music_report.metadata_reads,
        stored_playlist.entries.len(),
        stored_playlist
            .entries
            .iter()
            .filter_map(|entry| entry.track_id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        playlist_report.metadata_reads,
        playlist_report.source_errors.len(),
        music_report.metadata_errors.len(),
    );

        drop(database);
        let wal_path = append_suffix(&database_path, "-wal");
        let shm_path = append_suffix(&database_path, "-shm");
        println!(
            "after_database_close wal_exists={} shm_exists={}",
            wal_path.exists(),
            shm_path.exists()
        );
        Ok(())
    }

    fn prepare_candidate_settings(
        mut settings: Value,
        music_source_id: &str,
        playlist_source_id: &str,
        playlist_id: &str,
        music_path: &Path,
        playlist_path: &Path,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let path_value = |path: &Path| -> Result<Value, Box<dyn Error>> {
            use std::os::windows::ffi::OsStrExt;
            Ok(json!({
                "encoding": "windowsUtf16",
                "value": path.as_os_str().encode_wide().collect::<Vec<_>>(),
            }))
        };
        settings["sourceRegistryAuthoritative"] = Value::Bool(true);
        settings["sources"] = json!([
            {
                "id": music_source_id,
                "displayName": "Music",
                "enabled": true,
                "kind": {
                    "type": "folder",
                    "mediaKind": "windows_filesystem",
                    "path": path_value(music_path)?,
                }
            },
            {
                "id": playlist_source_id,
                "displayName": "hanser.m3u8",
                "enabled": true,
                "kind": {
                    "type": "playlistFile",
                    "playlistId": playlist_id,
                    "path": path_value(playlist_path)?,
                }
            }
        ]);
        let mut bytes = serde_json::to_vec_pretty(&settings)?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    }

    fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        PathBuf::from(value)
    }

    fn utc_now_ms() -> Result<i64, Box<dyn Error>> {
        let duration = SystemTime::now().duration_since(UNIX_EPOCH)?;
        Ok(i64::try_from(duration.as_millis())?)
    }

    fn system_time_to_utc_ms(time: SystemTime) -> Option<i64> {
        time.duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
    }

    struct PlaylistSourceIndex {
        scan: Option<SourceScan>,
        windows: WindowsMediaIndex,
    }

    impl MediaIndex for PlaylistSourceIndex {
        fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
            self.scan.take().unwrap_or_else(|| SourceScan {
                source_id: root.id,
                state: SourceScanState::Incomplete {
                    reason: "playlist source scan was already consumed".to_owned(),
                },
                tracks: Vec::new(),
                errors: Vec::new(),
            })
        }

        fn read_metadata(
            &mut self,
            record: &MediaTrackRecord,
        ) -> Result<TrackMetadata, TrackMetadataError> {
            self.windows.read_metadata(record)
        }
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    windows_only::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("unsupported: this one-off library rebuild example requires Windows");
}
