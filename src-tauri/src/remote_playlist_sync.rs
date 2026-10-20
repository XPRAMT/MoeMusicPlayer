//! Playlist files on a remote server (ADR 0026): the file lists the members, the server holds both the list and the
//! songs. Everything is relative to the account's root, so one playlist source needs only its account and the path of
//! the file.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use player_core::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaTrackRecord, MetadataReader,
    Playlist, SourceId, SourceScan, SyncEngine, SyncReport, TrackMetadata, TrackMetadataError,
};
use player_db::{Database, PlaylistFileSyncState};
use player_remote::fs::RemoteFs;
use player_remote::index::{is_excluded, RemoteMediaIndex, RemoteResolver};
use sha2::{Digest, Sha256};

use crate::playlist_exchange::read_playlist_bytes;
use crate::settings::{SourceEntry, SourceEntryKind};

/// A playlist file larger than this is not a playlist.
const MAX_PLAYLIST_BYTES: u64 = 16 * 1024 * 1024;

/// Where relative entries of a remote playlist are resolved while parsing (the parser works on local paths).
const PSEUDO_ROOT: &str = "/__moeremote__";

/// A remote folder source of the same account: tracks under it already belong to that source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CoveringFolder {
    pub source_id: SourceId,
    /// Relative to the account's root, `/`-separated, no outer slashes.
    pub base: String,
}

/// `a/b/../c` → `a/c`; `None` when the path climbs out of the root.
fn normalize_relative(path: &Path) -> Option<String> {
    let mut pieces: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(piece) => pieces.push(piece.to_string_lossy().into_owned()),
            Component::ParentDir => {
                pieces.pop()?;
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
        }
    }
    Some(pieces.join("/"))
}

/// The locator an entry of a remote playlist is stored under: the folder source of the account that already holds the
/// song when there is one (so a song is one track however many places list it), otherwise the playlist's own source.
fn locator_for(rel: &str, playlist_source: SourceId, covering: &[CoveringFolder]) -> MediaLocator {
    let best = covering
        .iter()
        .filter(|folder| {
            folder.base.is_empty()
                || rel
                    .strip_prefix(folder.base.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
        .max_by_key(|folder| folder.base.len());
    match best {
        Some(folder) => MediaLocator::Remote {
            source_id: folder.source_id,
            path: rel
                .strip_prefix(folder.base.as_str())
                .unwrap_or(rel)
                .trim_start_matches('/')
                .to_owned(),
        },
        None => MediaLocator::Remote {
            source_id: playlist_source,
            path: rel.to_owned(),
        },
    }
}

/// Parse the bytes of the playlist file `file_rel` (relative to the account's root). Relative entries become remote
/// locators; entries that are not on the server (a drive path, a web address) stay what they are and stay unresolved.
pub(crate) fn parse_remote_playlist(
    bytes: &[u8],
    file_rel: &str,
    playlist_source: SourceId,
    covering: &[CoveringFolder],
) -> Result<Playlist, String> {
    let pseudo = PathBuf::from(PSEUDO_ROOT).join(file_rel);
    let mut playlist = read_playlist_bytes(bytes, &pseudo)?;
    let root = Path::new(PSEUDO_ROOT);
    for entry in &mut playlist.entries {
        let MediaLocator::FileSystem(path) = &entry.locator else {
            continue;
        };
        let Ok(inside) = path.strip_prefix(root) else {
            continue;
        };
        let Some(rel) = normalize_relative(inside) else {
            continue;
        };
        entry.locator = locator_for(&rel, playlist_source, covering);
    }
    Ok(playlist)
}

/// Read a whole (small) remote file.
fn read_all(fs: &dyn RemoteFs, path: &str) -> Result<Vec<u8>, String> {
    let mut file = fs.open(path).map_err(|error| error.to_string())?;
    if file.len() > MAX_PLAYLIST_BYTES {
        return Err("播放清單檔案過大。".to_owned());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(file.len()).unwrap_or(0));
    let mut offset = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    while offset < file.len() {
        let read = file
            .read_at(offset, &mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        offset += read as u64;
    }
    Ok(bytes)
}

/// The remote folder sources of `account_id` (enabled ones), for [`locator_for`].
pub(crate) fn covering_folders(sources: &[SourceEntry], account_id: &str) -> Vec<CoveringFolder> {
    sources
        .iter()
        .filter(|source| source.enabled)
        .filter_map(|source| {
            let SourceEntryKind::Folder {
                path,
                remote_account: Some(account),
                ..
            } = &source.kind
            else {
                return None;
            };
            if account != account_id {
                return None;
            }
            match path.to_media_locator() {
                Ok(MediaLocator::Remote { path, .. }) => Some(CoveringFolder {
                    source_id: source.id,
                    base: path.trim_matches('/').to_owned(),
                }),
                _ => None,
            }
        })
        .collect()
}

/// The index of one playlist sync: the scan is made up front, the tags are read by the ordinary remote reader.
struct PlaylistIndex {
    scan: Option<SourceScan>,
    reader: RemoteMediaIndex,
}

impl MediaIndex for PlaylistIndex {
    fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
        Some(&self.reader)
    }

    fn metadata_read_threads(&self) -> usize {
        self.reader.metadata_read_threads()
    }

    fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
        self.scan.take().unwrap_or_else(|| SourceScan {
            source_id: SourceId::new(),
            state: player_core::SourceScanState::Incomplete {
                reason: "playlist scan was already consumed".to_owned(),
            },
            tracks: Vec::new(),
            errors: Vec::new(),
        })
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        MetadataReader::read(&self.reader, track)
    }
}

/// Synchronize a registered playlist-file source that lives on a remote server. `None` means the file was unreachable
/// or invalid: nothing is reconciled and a later sync tries again.
pub(crate) fn sync_remote_playlist_source(
    database: &mut Database,
    source: &SourceEntry,
    configured_sources: &[SourceEntry],
    resolver: Arc<dyn RemoteResolver>,
    synced_at_utc_ms: i64,
) -> Result<Option<SyncReport>, String> {
    let SourceEntryKind::PlaylistFile {
        playlist_id,
        path,
        remote_account: Some(account_id),
        ..
    } = &source.kind
    else {
        return Ok(None);
    };
    let file_locator = path.to_media_locator().map_err(|error| error.to_string())?;
    let MediaLocator::Remote {
        path: file_rel, ..
    } = &file_locator
    else {
        return Ok(None);
    };
    let Ok(resolved) = resolver.resolve(source.id) else {
        return Ok(None);
    };
    let Ok(entry) = resolved.fs.stat(file_rel) else {
        return Ok(None);
    };
    if entry.is_dir {
        return Ok(None);
    }
    let Ok(bytes) = read_all(resolved.fs.as_ref(), file_rel) else {
        return Ok(None);
    };
    let fingerprint = FileFingerprint {
        size_bytes: entry.size,
        modified_at_utc_ms: entry.modified_ms,
        created_at_utc_ms: None,
    };
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let prior_state = database
        .playlist_file_sync_state(source.id)
        .map_err(|error| error.to_string())?;
    let changed = prior_state.as_ref().is_none_or(|state| {
        state.playlist_id != *playlist_id
            || state.locator != file_locator
            || state.fingerprint != fingerprint
            || state.content_sha256 != digest
    });

    let mut playlist = if changed {
        let covering = covering_folders(configured_sources, account_id);
        let Ok(mut playlist) = parse_remote_playlist(&bytes, file_rel, source.id, &covering)
        else {
            return Ok(None);
        };
        playlist.id = *playlist_id;
        playlist
    } else {
        database
            .get_playlist(*playlist_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| {
                "registered playlist source has no saved playlist projection".to_owned()
            })?
    };

    let root = LibraryRoot {
        id: source.id,
        kind: player_core::MediaSourceKind::PlaylistFile,
        display_name: source.display_name.clone(),
        locator: file_locator.clone(),
        enabled: source.enabled,
    };
    database
        .save_library_root(&root)
        .map_err(|error| error.to_string())?;

    // Only the songs no folder source of the account holds are scanned into this source.
    let own_paths: Vec<String> = playlist
        .entries
        .iter()
        .filter_map(|entry| match &entry.locator {
            MediaLocator::Remote { source_id, path } if *source_id == source.id => {
                Some(path.clone())
            }
            _ => None,
        })
        .collect();
    let reader = RemoteMediaIndex::new(Arc::clone(&resolver));
    let scan = reader.scan_paths(source.id, &own_paths);
    let mut index = PlaylistIndex {
        scan: Some(scan),
        reader,
    };
    let report =
        SyncEngine::sync_with_progress(&root, &mut index, database, synced_at_utc_ms, |_| {})
            .map_err(|error| error.to_string())?;

    // Attach the shared TrackId of every entry that now resolves; an entry that does not stays in the list and is tried
    // again at the next sync (the file may be back, or the folder source may not have been scanned yet).
    let mut playlist_needs_save = changed;
    for entry in &mut playlist.entries {
        if entry.track_id.is_some() {
            continue;
        }
        let MediaLocator::Remote { source_id, path } = &entry.locator else {
            continue;
        };
        if *source_id == source.id
            && is_excluded(path, &resolved.excluded)
        {
            continue;
        }
        let track_id = database
            .resolve_track_id_for_locator(&entry.locator)
            .map_err(|error| error.to_string())?;
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
                    locator: file_locator,
                    fingerprint,
                    content_sha256: digest,
                },
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn covering(id: SourceId, base: &str) -> CoveringFolder {
        CoveringFolder {
            source_id: id,
            base: base.to_owned(),
        }
    }

    fn remote(playlist: &Playlist, index: usize) -> (SourceId, String) {
        match &playlist.entries[index].locator {
            MediaLocator::Remote { source_id, path } => (*source_id, path.clone()),
            other => panic!("not a remote locator: {other:?}"),
        }
    }

    #[test]
    fn relative_entries_become_paths_under_the_account_root() {
        let own = SourceId::new();
        let text = "#EXTM3U\n#EXTINF:10,A\nsong a.flac\n../Other/b.flac\nsub/./c.flac\nsub\\d.flac\n";
        let playlist =
            parse_remote_playlist(text.as_bytes(), "Music/Lists/best.m3u8", own, &[]).unwrap();
        assert_eq!(playlist.name, "best");
        assert_eq!(remote(&playlist, 0), (own, "Music/Lists/song a.flac".to_owned()));
        assert_eq!(remote(&playlist, 1), (own, "Music/Other/b.flac".to_owned()));
        assert_eq!(remote(&playlist, 2), (own, "Music/Lists/sub/c.flac".to_owned()));
        assert_eq!(playlist.entries[0].title.as_deref(), Some("A"));
    }

    #[test]
    fn entries_that_leave_the_server_stay_unresolved() {
        let own = SourceId::new();
        let text = "../../../up.flac\nhttp://example.com/x.mp3\n";
        let playlist = parse_remote_playlist(text.as_bytes(), "a/list.m3u8", own, &[]).unwrap();
        assert!(matches!(playlist.entries[0].locator, MediaLocator::FileSystem(_)));
        assert!(matches!(playlist.entries[1].locator, MediaLocator::ContentUri(_)));
    }

    #[test]
    fn songs_under_a_folder_source_of_the_account_use_that_source() {
        let own = SourceId::new();
        let music = SourceId::new();
        let deep = SourceId::new();
        let folders = [covering(music, "Music"), covering(deep, "Music/Hi-Res")];
        let text = "../Hi-Res/x.flac\n../Loss/y.flac\n../../Other/z.flac\n";
        let playlist =
            parse_remote_playlist(text.as_bytes(), "Music/Lists/p.m3u8", own, &folders).unwrap();
        // The deepest folder source wins; the path is relative to that source's folder.
        assert_eq!(remote(&playlist, 0), (deep, "x.flac".to_owned()));
        assert_eq!(remote(&playlist, 1), (music, "Loss/y.flac".to_owned()));
        assert_eq!(remote(&playlist, 2), (own, "Other/z.flac".to_owned()));
    }

    mod sync {
        use std::collections::HashMap;
        use std::fs;
        use std::path::{Path, PathBuf};

        use player_core::{
            ListTracksQuery, MediaSourceKind, SyncEngine, TrackListSort, TrackSortDirection,
            TrackSortField,
        };
        use player_remote::fs::RemoteError;
        use player_remote::index::ResolvedSource;
        use player_remote::local::DirFs;

        use crate::settings::StoredPath;

        use super::*;

        struct Resolver {
            fs: Arc<dyn RemoteFs>,
            /// Source → base folder.
            bases: HashMap<SourceId, String>,
        }

        impl RemoteResolver for Resolver {
            fn resolve(&self, source_id: SourceId) -> Result<ResolvedSource, RemoteError> {
                let base = self.bases.get(&source_id).ok_or(RemoteError::NotFound)?;
                Ok(ResolvedSource {
                    fs: Arc::clone(&self.fs),
                    base_path: base.clone(),
                    excluded: Vec::new(),
                })
            }
        }

        fn fixtures() -> PathBuf {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/player-wavpack/tests/fixtures")
        }

        fn tree(name: &str) -> PathBuf {
            let root = std::env::temp_dir()
                .join(format!("moe-remote-playlist-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            for (relative, fixture) in [
                ("Artist/Album/01 s16.wv", "s16.wv"),
                ("Artist/Album/02 s24.wv", "s24.wv"),
                ("Other/f32.wv", "f32.wv"),
            ] {
                let target = root.join(relative);
                fs::create_dir_all(target.parent().unwrap()).unwrap();
                fs::copy(fixtures().join(fixture), target).unwrap();
            }
            fs::create_dir_all(root.join("Lists")).unwrap();
            root
        }

        fn playlist_source(id: SourceId, file: &str) -> SourceEntry {
            let locator = MediaLocator::Remote {
                source_id: id,
                path: file.to_owned(),
            };
            SourceEntry {
                id,
                display_name: "mix".to_owned(),
                enabled: true,
                kind: SourceEntryKind::PlaylistFile {
                    playlist_id: player_core::PlaylistId::new(),
                    path: StoredPath::from_locator(&locator).unwrap(),
                    tree_uri: None,
                    remote_account: Some("acc".to_owned()),
                },
            }
        }

        fn track_count(database: &Database) -> usize {
            database
                .list_tracks_page(ListTracksQuery {
                    offset: 0,
                    limit: 100,
                    query: None,
                    field_filter: None,
                    sort: TrackListSort {
                        field: TrackSortField::Title,
                        direction: TrackSortDirection::Asc,
                    },
                })
                .unwrap()
                .items
                .len()
        }

        #[test]
        fn a_remote_playlist_scans_its_songs_and_follows_the_file() {
            let root = tree("own");
            fs::write(
                root.join("Lists/mix.m3u8"),
                "#EXTM3U\n#PLAYLIST:Mix\n../Artist/Album/01 s16.wv\n../Other/f32.wv\n../Gone/x.wv\n",
            )
            .unwrap();
            let id = SourceId::new();
            let resolver: Arc<dyn RemoteResolver> = Arc::new(Resolver {
                fs: Arc::new(DirFs::new(&root)),
                bases: HashMap::from([(id, String::new())]),
            });
            let source = playlist_source(id, "Lists/mix.m3u8");
            let mut database = Database::open_in_memory().unwrap();

            let report = sync_remote_playlist_source(&mut database, &source, &[], Arc::clone(&resolver), 1_000)
                .unwrap()
                .expect("the playlist is reachable");
            assert_eq!(report.observed, 2);
            assert_eq!(track_count(&database), 2);
            let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind else {
                unreachable!()
            };
            let saved = database.get_playlist(*playlist_id).unwrap().expect("playlist");
            assert_eq!(saved.name, "Mix");
            assert_eq!(saved.entries.len(), 3);
            assert_eq!(saved.entries.iter().filter(|entry| entry.track_id.is_some()).count(), 2);

            // The same file again changes nothing.
            sync_remote_playlist_source(&mut database, &source, &[], Arc::clone(&resolver), 2_000)
                .unwrap()
                .expect("second sync");
            assert_eq!(track_count(&database), 2);

            // A shorter list drops the song that is no longer named.
            fs::write(root.join("Lists/mix.m3u8"), "#EXTM3U\n../Other/f32.wv\n").unwrap();
            sync_remote_playlist_source(&mut database, &source, &[], resolver, 3_000)
                .unwrap()
                .expect("third sync");
            let saved = database.get_playlist(*playlist_id).unwrap().expect("playlist");
            assert_eq!(saved.entries.len(), 1);
            let _ = fs::remove_dir_all(root);
        }

        #[test]
        fn a_song_the_account_already_holds_is_not_a_second_track() {
            let root = tree("covered");
            fs::write(
                root.join("Lists/mix.m3u8"),
                "../Artist/Album/01 s16.wv\n../Other/f32.wv\n",
            )
            .unwrap();
            let folder = SourceId::new();
            let list = SourceId::new();
            let resolver: Arc<dyn RemoteResolver> = Arc::new(Resolver {
                fs: Arc::new(DirFs::new(&root)),
                bases: HashMap::from([(folder, "Artist".to_owned()), (list, String::new())]),
            });
            let mut database = Database::open_in_memory().unwrap();
            // The folder source `Artist` was scanned before.
            let folder_root = LibraryRoot {
                id: folder,
                kind: MediaSourceKind::Remote,
                display_name: "Artist".to_owned(),
                locator: MediaLocator::Remote {
                    source_id: folder,
                    path: "Artist".to_owned(),
                },
                enabled: true,
            };
            database.save_library_root(&folder_root).unwrap();
            let mut index = RemoteMediaIndex::new(Arc::clone(&resolver));
            SyncEngine::sync_with_progress(&folder_root, &mut index, &mut database, 500, |_| {})
                .unwrap();
            assert_eq!(track_count(&database), 2);

            let folder_entry = SourceEntry {
                id: folder,
                display_name: "Artist".to_owned(),
                enabled: true,
                kind: SourceEntryKind::Folder {
                    media_kind: MediaSourceKind::Remote,
                    path: StoredPath::from_locator(&folder_root.locator).unwrap(),
                    remote_account: Some("acc".to_owned()),
                },
            };
            let source = playlist_source(list, "Lists/mix.m3u8");
            sync_remote_playlist_source(
                &mut database,
                &source,
                &[folder_entry, source.clone()],
                resolver,
                1_000,
            )
            .unwrap()
            .expect("synced");
            // Only the song outside the folder source is new.
            assert_eq!(track_count(&database), 3);
            let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind else {
                unreachable!()
            };
            let saved = database.get_playlist(*playlist_id).unwrap().expect("playlist");
            assert!(saved.entries.iter().all(|entry| entry.track_id.is_some()));
            let _ = fs::remove_dir_all(root);
        }
    }

    #[test]
    fn a_source_of_the_whole_account_holds_everything() {
        let own = SourceId::new();
        let all = SourceId::new();
        let playlist = parse_remote_playlist(
            b"x/y.flac\n",
            "p.m3u8",
            own,
            &[covering(all, "")],
        )
        .unwrap();
        assert_eq!(remote(&playlist, 0), (all, "x/y.flac".to_owned()));
    }
}
