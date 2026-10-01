use std::{collections::HashMap, path::PathBuf};

use player_core::{
    parse_m3u, parse_m3u8, FileFingerprint, LibraryRoot, MediaIndex, MediaLocator,
    MediaSourceError, MediaSourceKind, MediaTrackRecord, SourceScan, SourceScanState, SyncEngine,
    SyncReport, TrackIdentity, TrackMetadata, TrackMetadataError,
};
use player_db::{Database, PlaylistFileSyncState};
use sha2::{Digest, Sha256};

use crate::settings::{SourceEntry, SourceEntryKind, StoredPath};

/// Match the native SAF cache lease cap. Parsing and resolving happen before any projection write.
pub(crate) const MAX_ANDROID_PLAYLIST_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const MAX_ANDROID_PLAYLIST_ENTRIES: usize = 100_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedSafPlaylistEntry {
    pub content_uri: String,
    pub size_bytes: Option<u64>,
    pub modified_at_utc_ms: Option<i64>,
}

/// Synchronize one already-authorized SAF playlist. `resolve_relative` receives a path relative
/// to the playlist document's parent and must resolve it strictly inside the granted tree.
/// Returning `Ok(None)` means this one entry is currently unavailable; its locator is retained
/// and complete reconciliation is disabled for this scan. Returning `Err` means the source/tree
/// itself could not be read or trusted and leaves all existing projections untouched.
#[allow(clippy::too_many_arguments)] // Resolver, URI authorization, and metadata are injected platform boundaries.
pub(crate) fn sync_android_playlist_source<R, A, M>(
    database: &mut Database,
    source: &SourceEntry,
    playlist_bytes: &[u8],
    modified_at_utc_ms: Option<i64>,
    synced_at_utc_ms: i64,
    mut resolve_relative: R,
    mut authorize_content_uri: A,
    read_metadata: M,
) -> Result<Option<SyncReport>, String>
where
    R: FnMut(&str) -> Result<Option<ResolvedSafPlaylistEntry>, String>,
    A: FnMut(&str) -> Result<Option<ResolvedSafPlaylistEntry>, String>,
    M: FnMut(&MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError>,
{
    let SourceEntryKind::PlaylistFile {
        playlist_id,
        path: StoredPath::Uri(document_uri),
        tree_uri: Some(tree_uri),
    } = &source.kind
    else {
        return Ok(None);
    };
    if !source.enabled {
        return Ok(None);
    }
    if !document_uri.starts_with("content://")
        || !tree_uri.starts_with("content://")
        || !same_content_authority(document_uri, tree_uri)
    {
        return Err("SAF playlist source is missing its document or tree URI".to_owned());
    }
    if playlist_bytes.len() > MAX_ANDROID_PLAYLIST_BYTES {
        return Err(format!(
            "Android playlist exceeds the {} MiB limit",
            MAX_ANDROID_PLAYLIST_BYTES / (1024 * 1024)
        ));
    }
    reject_excessive_entries_before_parse(playlist_bytes)?;

    let extension = source.display_name.rsplit('.').next().unwrap_or_default();
    let synthetic_file =
        PathBuf::from("__android_saf_playlist_parent__").join(format!("playlist.{extension}"));
    let mut parsed = if extension.eq_ignore_ascii_case("m3u8") {
        parse_m3u8(playlist_bytes, &synthetic_file)
    } else if extension.eq_ignore_ascii_case("m3u") {
        parse_m3u(playlist_bytes, &synthetic_file)
    } else {
        return Err("Android playlist must have an .m3u or .m3u8 display name".to_owned());
    }
    .map_err(|error| format!("unable to parse Android playlist: {error}"))?;
    if parsed.entries.len() > MAX_ANDROID_PLAYLIST_ENTRIES {
        return Err(format!(
            "Android playlist exceeds the {MAX_ANDROID_PLAYLIST_ENTRIES} entry limit"
        ));
    }

    let fingerprint = FileFingerprint {
        size_bytes: playlist_bytes.len() as u64,
        modified_at_utc_ms,
    };
    let digest: [u8; 32] = Sha256::digest(playlist_bytes).into();
    let document_locator = MediaLocator::ContentUri(document_uri.clone());
    let prior_state = database
        .playlist_file_sync_state(source.id)
        .map_err(|error| error.to_string())?;
    let changed = prior_state.as_ref().is_none_or(|state| {
        state.playlist_id != *playlist_id
            || state.locator != document_locator
            || state.fingerprint != fingerprint
            || state.content_sha256 != digest
    });

    // Do all authorization-dependent resolution before writing the root, playlist, sync state,
    // or source mappings. The parser's synthetic parent makes relative references distinguishable
    // from absolute paths and arbitrary URI schemes without interpreting them as filesystem paths.
    let synthetic_parent = synthetic_file.parent().expect("synthetic playlist parent");
    let mut records = Vec::new();
    let mut seen_uris = HashMap::<String, usize>::new();
    let mut resolved_relative_cache = HashMap::<String, Option<ResolvedSafPlaylistEntry>>::new();
    let mut known_absolute_cache = HashMap::<String, Option<ResolvedSafPlaylistEntry>>::new();
    let mut resolved_by_entry = Vec::with_capacity(parsed.entries.len());
    let mut incomplete_reason = None;
    for entry in &parsed.entries {
        match &entry.locator {
            MediaLocator::FileSystem(path) => {
                if let Ok(relative) = path.strip_prefix(synthetic_parent) {
                    let relative = relative.to_string_lossy().replace('\\', "/");
                    let resolution = if let Some(cached) = resolved_relative_cache.get(&relative) {
                        cached.clone()
                    } else {
                        let resolved = resolve_relative(&relative)?;
                        resolved_relative_cache.insert(relative.clone(), resolved.clone());
                        resolved
                    };
                    match resolution {
                        Some(resolved) => {
                            validate_resolved(&resolved)?;
                            let record_index =
                                if let Some(index) = seen_uris.get(&resolved.content_uri) {
                                    *index
                                } else {
                                    let index = records.len();
                                    let uri = resolved.content_uri.clone();
                                    records.push(MediaTrackRecord {
                                        identity: TrackIdentity {
                                            source_id: source.id,
                                            source_item_id: uri.clone(),
                                            locator_key: Some(uri.clone()),
                                        },
                                        locator: MediaLocator::ContentUri(uri.clone()),
                                        fingerprint: FileFingerprint {
                                            size_bytes: resolved.size_bytes.unwrap_or(0),
                                            modified_at_utc_ms: resolved.modified_at_utc_ms,
                                        },
                                        metadata: None,
                                    });
                                    seen_uris.insert(uri, index);
                                    index
                                };
                            resolved_by_entry.push(Some(records[record_index].locator.clone()));
                        }
                        None => {
                            incomplete_reason.get_or_insert_with(|| "one or more relative SAF playlist entries could not be resolved".to_owned());
                            resolved_by_entry.push(None);
                        }
                    }
                } else {
                    // Absolute paths (including Windows paths) cannot be interpreted within a
                    // SAF grant. Keep their original locator, and prevent destructive cleanup.
                    incomplete_reason.get_or_insert_with(|| "playlist contains an absolute filesystem locator outside SAF resolution".to_owned());
                    resolved_by_entry.push(None);
                }
            }
            MediaLocator::ContentUri(uri) if uri.starts_with("content://") => {
                let resolved = if let Some(resolved) = known_absolute_cache.get(uri) {
                    resolved.clone()
                } else {
                    let track_exists = database
                        .resolve_track_id_for_locator(&MediaLocator::ContentUri(uri.clone()))
                        .map_err(|error| error.to_string())?
                        .is_some();
                    let resolved = if track_exists {
                        authorize_content_uri(uri)?
                    } else {
                        None
                    };
                    known_absolute_cache.insert(uri.clone(), resolved.clone());
                    resolved
                };
                if let Some(resolved) = resolved {
                    validate_resolved(&resolved)?;
                    if resolved.content_uri != *uri {
                        return Err(
                            "absolute URI authorization returned a different content URI"
                                .to_owned(),
                        );
                    }
                    let record_index = if let Some(index) = seen_uris.get(uri) {
                        *index
                    } else {
                        let index = records.len();
                        records.push(MediaTrackRecord {
                            identity: TrackIdentity {
                                source_id: source.id,
                                source_item_id: uri.clone(),
                                locator_key: Some(uri.clone()),
                            },
                            locator: MediaLocator::ContentUri(uri.clone()),
                            fingerprint: FileFingerprint {
                                // The core fingerprint is non-optional. Zero denotes an
                                // unavailable size; None remains explicit for timestamp.
                                size_bytes: resolved.size_bytes.unwrap_or(0),
                                modified_at_utc_ms: resolved.modified_at_utc_ms,
                            },
                            metadata: None,
                        });
                        seen_uris.insert(uri.clone(), index);
                        index
                    };
                    resolved_by_entry.push(Some(records[record_index].locator.clone()));
                } else {
                    incomplete_reason.get_or_insert_with(|| {
                        "absolute content URI is not mapped by an enabled Android source".to_owned()
                    });
                    resolved_by_entry.push(None);
                }
            }
            MediaLocator::ContentUri(_) => {
                incomplete_reason
                    .get_or_insert_with(|| "playlist contains a non-SAF URI locator".to_owned());
                resolved_by_entry.push(None);
            }
        }
    }

    // The parser needs a synthetic parent to identify relative entries. Remove that internal
    // prefix before persistence so unresolved items retain source-relative, exportable locators.
    for entry in &mut parsed.entries {
        if let MediaLocator::FileSystem(path) = &entry.locator {
            if let Ok(relative) = path.strip_prefix(synthetic_parent) {
                entry.locator = MediaLocator::FileSystem(relative.to_path_buf());
            }
        }
    }

    let root = LibraryRoot {
        id: source.id,
        kind: MediaSourceKind::PlaylistFile,
        display_name: source.display_name.clone(),
        locator: document_locator.clone(),
        enabled: source.enabled,
    };
    let scan = SourceScan {
        source_id: source.id,
        state: incomplete_reason.map_or(SourceScanState::Complete, |reason| {
            SourceScanState::Incomplete { reason }
        }),
        tracks: records,
        errors: Vec::<MediaSourceError>::new(),
    };
    database
        .save_library_root(&root)
        .map_err(|error| error.to_string())?;
    let mut index = SafPlaylistMediaIndex {
        scan: Some(scan),
        read_metadata,
    };
    let report = SyncEngine::sync(&root, &mut index, database, synced_at_utc_ms)
        .map_err(|error| error.to_string())?;

    let mut playlist = parsed;
    playlist.id = *playlist_id;
    let prior_playlist = database
        .get_playlist(*playlist_id)
        .map_err(|error| error.to_string())?;
    for (position, entry) in playlist.entries.iter_mut().enumerate() {
        if let Some(Some(MediaLocator::ContentUri(uri))) = resolved_by_entry.get(position) {
            entry.track_id = database
                .resolve_track_id_for_locator(&MediaLocator::ContentUri(uri.clone()))
                .map_err(|error| error.to_string())?;
        } else if let Some(old_entry) = prior_playlist
            .as_ref()
            .and_then(|old| old.entries.get(position))
        {
            if old_entry.locator == entry.locator {
                entry.track_id = old_entry.track_id;
            }
        }
    }
    let playlist_needs_save = changed || prior_playlist.as_ref() != Some(&playlist);
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
                    locator: document_locator,
                    fingerprint,
                    content_sha256: digest,
                },
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(report))
}

fn validate_resolved(resolved: &ResolvedSafPlaylistEntry) -> Result<(), String> {
    if !resolved.content_uri.starts_with("content://") || resolved.content_uri.trim().is_empty() {
        return Err("SAF resolver returned an invalid content URI".to_owned());
    }
    if resolved.modified_at_utc_ms.is_some_and(|value| value < 0) {
        return Err("SAF resolver returned a pre-epoch modification timestamp".to_owned());
    }
    if resolved
        .size_bytes
        .is_some_and(|value| value > i64::MAX as u64)
    {
        return Err("SAF resolver returned a media size outside SQLite's integer range".to_owned());
    }
    Ok(())
}

fn reject_excessive_entries_before_parse(bytes: &[u8]) -> Result<(), String> {
    let decoded = if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        if !bytes.len().is_multiple_of(2) {
            return Ok(()); // Let the canonical parser report the encoding error.
        }
        let little_endian = bytes[0] == 0xFF;
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect::<Vec<_>>();
        let Ok(text) = String::from_utf16(&units) else {
            return Ok(()); // The canonical parser rejects malformed UTF-16 before playlist allocation.
        };
        Some(text)
    } else {
        std::str::from_utf8(bytes)
            .ok()
            .map(|text| text.strip_prefix('\u{feff}').unwrap_or(text).to_owned())
    };
    let Some(decoded) = decoded else {
        return Ok(()); // Invalid UTF-8 is rejected by the canonical parser.
    };
    let mut entries = 0usize;
    for line in decoded.lines() {
        if !line.is_empty() && !line.starts_with('#') {
            entries += 1;
            if entries > MAX_ANDROID_PLAYLIST_ENTRIES {
                return Err(format!(
                    "Android playlist exceeds the {MAX_ANDROID_PLAYLIST_ENTRIES} entry limit"
                ));
            }
        }
    }
    Ok(())
}

fn content_authority(uri: &str) -> Option<&str> {
    uri.strip_prefix("content://")?
        .split('/')
        .next()
        .filter(|value| !value.is_empty())
}

fn same_content_authority(left: &str, right: &str) -> bool {
    content_authority(left)
        .zip(content_authority(right))
        .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
}

struct SafPlaylistMediaIndex<M> {
    scan: Option<SourceScan>,
    read_metadata: M,
}

impl<M> MediaIndex for SafPlaylistMediaIndex<M>
where
    M: FnMut(&MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError>,
{
    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.scan.take().unwrap_or_else(|| SourceScan {
            source_id: root.id,
            state: SourceScanState::Incomplete {
                reason: "SAF playlist scan was already consumed".to_owned(),
            },
            tracks: Vec::new(),
            errors: Vec::new(),
        })
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        (self.read_metadata)(track)
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::Path};

    use player_core::{
        write_m3u8, LibraryRoot, M3uExportOptions, MediaLocator, MediaSourceKind, PlaylistId,
        SourceId, TrackMetadataError,
    };
    use player_db::Database;
    use sha2::Digest;

    use crate::settings::{SourceEntry, SourceEntryKind, StoredPath};

    use super::{sync_android_playlist_source, ResolvedSafPlaylistEntry};

    fn source() -> SourceEntry {
        SourceEntry {
            id: SourceId::new(),
            display_name: "清單.m3u8".to_owned(),
            enabled: true,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id: PlaylistId::new(),
                path: StoredPath::Uri("content://provider/document/playlist".to_owned()),
                tree_uri: Some("content://provider/tree/root".to_owned()),
            },
        }
    }

    fn resolved(uri: &str) -> ResolvedSafPlaylistEntry {
        ResolvedSafPlaylistEntry {
            content_uri: uri.to_owned(),
            size_bytes: Some(17),
            modified_at_utc_ms: Some(1_800_000_000_000),
        }
    }

    fn sync(
        database: &mut Database,
        source: &SourceEntry,
        bytes: &[u8],
        modified: i64,
        uri: &str,
    ) -> player_core::SyncReport {
        sync_android_playlist_source(
            database,
            source,
            bytes,
            Some(modified),
            modified,
            |_| Ok(Some(resolved(uri))),
            |_| Ok(None),
            |_| Ok(Default::default()),
        )
        .expect("sync should succeed")
        .expect("registered SAF playlist")
    }

    #[test]
    fn legacy_source_json_without_tree_uri_deserializes_as_none() {
        let old = serde_json::json!({
            "id": SourceId::new(), "displayName": "old.m3u8", "enabled": true,
            "kind": { "type": "playlistFile", "playlistId": PlaylistId::new(),
                "path": { "encoding": "uri", "value": "content://provider/doc/old" } }
        });
        let parsed: SourceEntry = serde_json::from_value(old).expect("legacy settings source");
        assert!(matches!(
            parsed.kind,
            SourceEntryKind::PlaylistFile { tree_uri: None, .. }
        ));
    }

    #[test]
    fn revoked_tree_and_parse_failure_leave_prior_projection_and_fingerprint_unchanged() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let bytes = b"#EXTM3U\nfirst.mp3\n";
        sync(
            &mut db,
            &source,
            bytes,
            10,
            "content://provider/audio/first",
        );
        let before = db
            .get_playlist(match source.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                _ => unreachable!(),
            })
            .expect("playlist")
            .expect("saved");
        let state = db.playlist_file_sync_state(source.id).expect("state");
        let denied = sync_android_playlist_source(
            &mut db,
            &source,
            b"#EXTM3U\nother.mp3\n",
            Some(11),
            11,
            |_| Err("tree grant revoked".to_owned()),
            |_| Ok(None),
            |_| Ok(Default::default()),
        );
        assert!(denied.is_err());
        let malformed = sync_android_playlist_source(
            &mut db,
            &source,
            b"#EXTM3U\n#EXTINF:not-a-duration,title\na.mp3\n",
            Some(12),
            12,
            |_| panic!("parse must fail before resolver"),
            |_| Ok(None),
            |_| Ok(Default::default()),
        );
        assert!(malformed.is_err());
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        assert_eq!(
            db.get_playlist(playlist_id)
                .expect("playlist")
                .expect("still saved"),
            before
        );
        assert_eq!(
            db.playlist_file_sync_state(source.id).expect("state"),
            state
        );
    }

    #[test]
    fn identical_size_and_mtime_but_changed_bytes_are_resynchronized() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let first = b"#EXTM3U\naaaa.mp3\n";
        let second = b"#EXTM3U\nbbbb.mp3\n";
        assert_eq!(first.len(), second.len());
        let first_report = sync(&mut db, &source, first, 10, "content://provider/audio/one");
        let second_report = sync(&mut db, &source, second, 10, "content://provider/audio/two");
        assert_eq!(first_report.observed, 1);
        assert_eq!(second_report.observed, 1);
        let state = db
            .playlist_file_sync_state(source.id)
            .expect("state")
            .expect("recorded");
        assert_ne!(
            state.content_sha256,
            <[u8; 32]>::from(sha2::Sha256::digest(first))
        );
    }

    #[test]
    fn duplicate_slots_keep_order_and_share_track_id_and_metadata_is_not_reread() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let bytes = b"#EXTM3U\n#EXTINF:1,first\na.mp3\n#EXTINF:2,second\na.mp3\n";
        let reads = Cell::new(0);
        sync_android_playlist_source(
            &mut db,
            &source,
            bytes,
            Some(10),
            10,
            |_| Ok(Some(resolved("content://provider/audio/a"))),
            |_| Ok(None),
            |_| {
                reads.set(reads.get() + 1);
                Ok(Default::default())
            },
        )
        .expect("first sync");
        assert_eq!(reads.get(), 1);
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        let saved = db
            .get_playlist(playlist_id)
            .expect("playlist")
            .expect("saved");
        assert_eq!(saved.entries.len(), 2);
        assert_eq!(saved.entries[0].title.as_deref(), Some("first"));
        assert_eq!(saved.entries[1].title.as_deref(), Some("second"));
        assert_eq!(saved.entries[0].track_id, saved.entries[1].track_id);
        sync_android_playlist_source(
            &mut db,
            &source,
            bytes,
            Some(10),
            11,
            |_| Ok(Some(resolved("content://provider/audio/a"))),
            |_| Ok(None),
            |_| {
                reads.set(reads.get() + 1);
                Ok(Default::default())
            },
        )
        .expect("second sync");
        assert_eq!(
            reads.get(),
            1,
            "unchanged source mapping avoids metadata read"
        );
    }

    #[test]
    fn missing_relative_entry_retains_projection_as_incomplete() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let bytes = b"#EXTM3U\na.mp3\n";
        sync(&mut db, &source, bytes, 10, "content://provider/audio/a");
        let report = sync_android_playlist_source(
            &mut db,
            &source,
            "#EXTM3U\n未匹配 曲目.flac\n".as_bytes(),
            Some(11),
            11,
            |_| Ok(None),
            |_| Ok(None),
            |_| Ok(Default::default()),
        )
        .expect("partial sync")
        .expect("source");
        assert!(!report.state.expect("state").allows_reconciliation());
        assert_eq!(db.count_tracks(None).expect("old mapping retained"), 1);
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        let saved = db
            .get_playlist(playlist_id)
            .expect("playlist")
            .expect("saved");
        assert_eq!(
            saved.entries[0].locator,
            MediaLocator::FileSystem(Path::new("未匹配 曲目.flac").to_path_buf())
        );
        let exported = write_m3u8(
            &saved,
            Path::new("export.m3u8"),
            &M3uExportOptions::default(),
        )
        .expect("export preserved unresolved locator");
        assert!(String::from_utf8(exported)
            .expect("UTF-8 export")
            .contains("未匹配 曲目.flac"));
    }

    #[test]
    fn metadata_failure_keeps_the_seen_locator_and_mapping() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let bytes = b"#EXTM3U\na.mp3\n";
        sync(&mut db, &source, bytes, 10, "content://provider/audio/a");
        let report = sync_android_playlist_source(
            &mut db,
            &source,
            bytes,
            Some(11),
            11,
            |_| {
                Ok(Some(ResolvedSafPlaylistEntry {
                    content_uri: "content://provider/audio/a".to_owned(),
                    size_bytes: Some(18),
                    modified_at_utc_ms: Some(11),
                }))
            },
            |_| Ok(None),
            |_| {
                Err(TrackMetadataError {
                    message: "temporarily unreadable metadata".to_owned(),
                })
            },
        )
        .expect("metadata failure is reported, not fatal")
        .expect("source");
        assert_eq!(report.metadata_errors.len(), 1);
        assert_eq!(db.count_tracks(None).expect("mapping remains"), 1);
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        let saved = db
            .get_playlist(playlist_id)
            .expect("playlist")
            .expect("saved");
        assert_eq!(
            saved.entries[0].locator,
            MediaLocator::FileSystem(Path::new("a.mp3").to_path_buf())
        );
        assert!(saved.entries[0].track_id.is_some());
    }

    #[test]
    fn separate_playlist_sources_resolving_same_content_uri_share_track_id() {
        let mut db = Database::open_in_memory().expect("database");
        let first = source();
        sync(
            &mut db,
            &first,
            b"#EXTM3U\na.mp3\n",
            10,
            "content://provider/audio/shared",
        );
        let mut second = source();
        second.id = SourceId::new();
        second.display_name = "another.m3u8".to_owned();
        if let SourceEntryKind::PlaylistFile {
            playlist_id,
            path,
            tree_uri,
        } = &mut second.kind
        {
            *playlist_id = PlaylistId::new();
            *path = StoredPath::Uri("content://provider/document/another".to_owned());
            *tree_uri = Some("content://provider/tree/root".to_owned());
        }
        sync(
            &mut db,
            &second,
            b"#EXTM3U\nother.mp3\n",
            10,
            "content://provider/audio/shared",
        );
        let first_id = db
            .get_playlist(match first.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                _ => unreachable!(),
            })
            .expect("first")
            .expect("saved")
            .entries[0]
            .track_id;
        let second_id = db
            .get_playlist(match second.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                _ => unreachable!(),
            })
            .expect("second")
            .expect("saved")
            .entries[0]
            .track_id;
        assert_eq!(first_id, second_id);
    }

    #[test]
    fn absolute_exported_content_uri_imports_only_existing_authorized_mapping() {
        let mut db = Database::open_in_memory().expect("database");
        let first = source();
        let uri = "content://media/external/audio/media/42";
        sync(&mut db, &first, b"#EXTM3U\na.mp3\n", 10, uri);
        db.save_library_root(&LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::AndroidSaf,
            display_name: "授權媒體來源".to_owned(),
            locator: MediaLocator::ContentUri("content://media/external/audio".to_owned()),
            enabled: true,
        })
        .expect("active source");
        let second = source();
        let metadata_reads = Cell::new(0);
        let report = sync_android_playlist_source(
            &mut db,
            &second,
            format!("#EXTM3U\n{uri}\n{uri}\n").as_bytes(),
            Some(11),
            11,
            |_| panic!("absolute URI must not use relative resolver"),
            |_| Ok(Some(resolved(uri))),
            |_| {
                metadata_reads.set(metadata_reads.get() + 1);
                Ok(Default::default())
            },
        )
        .expect("absolute URI sync")
        .expect("source");
        assert_eq!(
            report.observed, 1,
            "duplicate URI slots share one source record"
        );
        let first_id = db
            .get_playlist(match first.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                _ => unreachable!(),
            })
            .expect("first")
            .expect("saved")
            .entries[0]
            .track_id;
        let second_id = db
            .get_playlist(match second.kind {
                SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
                _ => unreachable!(),
            })
            .expect("second")
            .expect("saved")
            .entries;
        assert_eq!(second_id.len(), 2);
        assert_eq!(
            second_id[0].locator,
            MediaLocator::ContentUri(uri.to_owned())
        );
        assert_eq!(second_id[0].track_id, first_id);
        assert_eq!(second_id[0].track_id, second_id[1].track_id);
        assert_eq!(
            metadata_reads.get(),
            0,
            "existing indexed metadata is reused"
        );
        sync_android_playlist_source(
            &mut db,
            &second,
            format!("#EXTM3U\n{uri}\n{uri}\n").as_bytes(),
            Some(11),
            12,
            |_| panic!("absolute URI must not use relative resolver"),
            |_| Ok(Some(resolved(uri))),
            |_| {
                metadata_reads.set(metadata_reads.get() + 1);
                Ok(Default::default())
            },
        )
        .expect("repeat sync");
        assert_eq!(
            metadata_reads.get(),
            0,
            "repeated URI import does not force metadata rereads"
        );
    }

    #[test]
    fn arbitrary_absolute_uri_is_preserved_without_resolution_or_reconciliation() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        sync(
            &mut db,
            &source,
            b"#EXTM3U\na.mp3\n",
            10,
            "content://provider/audio/a",
        );
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        let result = sync_android_playlist_source(
            &mut db,
            &source,
            b"#EXTM3U\ncontent://other.provider/audio/untrusted\n",
            Some(11),
            11,
            |_| panic!("absolute URI must not use relative resolver"),
            |_| Ok(None),
            |_| panic!("untrusted URI must not reach metadata reader"),
        )
        .expect("untrusted URI is a partial scan")
        .expect("source");
        assert!(!result.state.expect("state").allows_reconciliation());
        assert_eq!(db.count_tracks(None).expect("preserved mapping"), 1);
        let saved = db
            .get_playlist(playlist_id)
            .expect("playlist")
            .expect("saved");
        assert_eq!(saved.entries.len(), 1);
        assert_eq!(
            saved.entries[0].locator,
            MediaLocator::ContentUri("content://other.provider/audio/untrusted".to_owned())
        );
    }

    #[test]
    fn excessive_entry_count_is_rejected_before_any_projection_write() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let mut bytes = String::from("#EXTM3U\n");
        for _ in 0..100_001 {
            bytes.push_str("a.mp3\n");
        }
        let result = sync_android_playlist_source(
            &mut db,
            &source,
            bytes.as_bytes(),
            Some(1),
            1,
            |_| panic!("limit must be checked before resolving"),
            |_| Ok(None),
            |_| Ok(Default::default()),
        );
        assert!(result.is_err());
        assert!(db.library_roots().expect("roots").is_empty());
        assert!(db
            .playlist_file_sync_state(source.id)
            .expect("sync state")
            .is_none());
    }

    #[test]
    fn exactly_one_hundred_thousand_entries_are_retained() {
        let mut db = Database::open_in_memory().expect("database");
        let source = source();
        let mut bytes = String::from("#EXTM3U\n");
        for _ in 0..100_000 {
            bytes.push_str("a.mp3\n");
        }
        let report = sync_android_playlist_source(
            &mut db,
            &source,
            bytes.as_bytes(),
            Some(1),
            1,
            |_| Ok(Some(resolved("content://provider/audio/a"))),
            |_| Ok(None),
            |_| Ok(Default::default()),
        )
        .expect("sync")
        .expect("source");
        assert_eq!(report.observed, 1);
        let playlist_id = match source.kind {
            SourceEntryKind::PlaylistFile { playlist_id, .. } => playlist_id,
            _ => unreachable!(),
        };
        assert_eq!(
            db.get_playlist(playlist_id)
                .expect("read")
                .expect("saved")
                .entries
                .len(),
            100_000
        );
    }
}
