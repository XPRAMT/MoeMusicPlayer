//! Playlist files on a remote server (ADR 0026): the file lists the members, the server holds both the list and the
//! songs. Everything is relative to the account's root, so one playlist source needs only its account and the path of
//! the file.

use std::path::PathBuf;
use std::sync::Arc;

use player_core::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MetadataReader, Playlist, SourceId, SyncEngine,
    SyncProgress, SyncReport,
};
use player_db::{Database, PlaylistFileSyncState};
use player_remote::fs::RemoteFs;
use player_remote::index::{is_excluded, RemoteMediaIndex, RemoteResolver};
use player_remote::playlist_style::PlaylistStyle;
use sha2::{Digest, Sha256};

use crate::playlist_exchange::read_playlist_bytes;
use crate::settings::{SourceEntry, SourceEntryKind};

/// Where relative entries of a remote playlist are resolved while parsing (the parser works on local paths).
const PSEUDO_ROOT: &str = "/__moeremote__";

/// Read the playlist file `file_rel` (relative to the account's root): the structure (name, `#EXTINF`) comes from the
/// ordinary M3U parser, and every entry that names a place on the server is mapped by the style of the file itself
/// ([`PlaylistStyle`], the same conversion the favorites writer uses in the other direction). `exists` says whether a
/// path relative to the account's root is a file on the server (used once, to learn how absolute entries map). Entries
/// that are not on the server (a web address, a path that climbs out of the root, an absolute path nothing matches) stay
/// what the parser made them and stay unresolved.
pub(crate) fn read_remote_playlist(
    bytes: &[u8],
    file_rel: &str,
    playlist_source: SourceId,
    exists: &dyn Fn(&str) -> bool,
) -> Result<Playlist, String> {
    let pseudo = PathBuf::from(PSEUDO_ROOT).join(file_rel);
    let mut playlist = read_playlist_bytes(bytes, &pseudo)?;
    let allow_utf16 = file_rel.to_ascii_lowercase().ends_with(".m3u");
    let Ok(text) = player_remote::playlist_edit::decode_text(bytes, allow_utf16) else {
        return Ok(playlist);
    };
    let raws = player_remote::playlist_edit::entry_texts(&text);
    if raws.len() != playlist.entries.len() {
        return Ok(playlist);
    }
    let folder = player_remote::playlist_edit::folder_of(file_rel);
    let style = PlaylistStyle::detect(&raws, folder, exists);
    for (entry, raw) in playlist.entries.iter_mut().zip(raws) {
        if let Some(rel) = style.to_server_path(raw, folder) {
            entry.locator = MediaLocator::Remote {
                source_id: playlist_source,
                path: rel,
            };
        }
    }
    Ok(playlist)
}

/// Read a whole (small) remote file.
fn read_all(fs: &dyn RemoteFs, path: &str) -> Result<Vec<u8>, String> {
    player_remote::playlist_edit::read_all(fs, path).map_err(|error| error.to_string())
}

/// Synchronize a registered playlist-file source that lives on a remote server. `None` means the file was unreachable
/// or invalid: nothing is reconciled and a later sync tries again.
pub(crate) fn sync_remote_playlist_source(
    database: &mut Database,
    source: &SourceEntry,
    _configured_sources: &[SourceEntry],
    resolver: Arc<dyn RemoteResolver>,
    synced_at_utc_ms: i64,
    progress: &mut dyn FnMut(SyncProgress),
) -> Result<Option<SyncReport>, String> {
    let SourceEntryKind::PlaylistFile {
        playlist_id,
        path,
        remote_account: Some(_),
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
    let mut changed = prior_state.as_ref().is_none_or(|state| {
        state.playlist_id != *playlist_id
            || state.locator != file_locator
            || state.fingerprint != fingerprint
            || state.content_sha256 != digest
    });

    if !changed {
        // Entries that are still plain paths were not found on the server last time; the file is read again so they get
        // another chance (the folders of the library may have changed).
        let saved = database
            .get_playlist(*playlist_id)
            .map_err(|error| error.to_string())?;
        if saved.as_ref().is_some_and(|saved| {
            saved
                .entries
                .iter()
                .any(|entry| entry.track_id.is_none() && matches!(entry.locator, MediaLocator::FileSystem(_)))
        }) {
            changed = true;
        }
        // An earlier version pointed the entries of songs a folder source held at that source. The list maps every song
        // itself now (see below), so a list saved that way is read again.
        if saved.as_ref().is_some_and(|saved| {
            saved.entries.iter().any(|entry| {
                matches!(&entry.locator, MediaLocator::Remote { source_id, .. } if *source_id != source.id)
            })
        }) {
            changed = true;
        }
    }
    let mut playlist = if changed {
        let fs = Arc::clone(&resolved.fs);
        let Ok(mut playlist) =
            read_remote_playlist(&bytes, file_rel, source.id, &|path| {
                fs.stat(path).is_ok_and(|entry| !entry.is_dir)
            })
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

    // Every song of the list is mapped by this source too, whatever other source of the account reaches the file: the
    // locator key names the file by account and path, so the song is still one track (the scan finds the existing one),
    // and the track keeps a mapping when the other source is disabled or removed.
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
    // The lookup of the songs on the server runs inside the engine's scan: a playlist reports its progress the way a
    // folder does, and the tags are read afterwards by the ordinary remote reader.
    let reader = RemoteMediaIndex::new(Arc::clone(&resolver));
    let source_id = source.id;
    let mut index = crate::playlist_scan_index::PlaylistScanIndex::new(
        Box::new(|report| reader.scan_paths_with_progress(source_id, &own_paths, report)),
        Box::new(|tracks| reader.lyric_sidecars_beside(source_id, tracks)),
        Box::new(|track| MetadataReader::read(&reader, track)),
        Some((&reader, reader.metadata_read_threads())),
    );
    let report =
        SyncEngine::sync_with_progress(&root, &mut index, database, synced_at_utc_ms, |update| {
            progress(update)
        })
        .map_err(|error| error.to_string())?;
    drop(index);

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
            read_remote_playlist(text.as_bytes(), "Music/Lists/best.m3u8", own, &|_| false).unwrap();
        assert_eq!(playlist.name, "best");
        assert_eq!(remote(&playlist, 0), (own, "Music/Lists/song a.flac".to_owned()));
        assert_eq!(remote(&playlist, 1), (own, "Music/Other/b.flac".to_owned()));
        assert_eq!(remote(&playlist, 2), (own, "Music/Lists/sub/c.flac".to_owned()));
        assert_eq!(playlist.entries[0].title.as_deref(), Some("A"));
    }

    #[test]
    fn absolute_windows_paths_are_found_on_the_server() {
        let own = SourceId::new();
        let text = "#EXTM3U\nD:\\Music\\Loss\\[a]\\x (1).flac\nD:\\Music\\Loss\\[a]\\y.flac\nD:\\Elsewhere\\z.flac\nrtsp://cam/stream\n";
        // The server shows the drive's `Music` folder as `Music` at its top.
        let files = ["Music/Loss/[a]/x (1).flac", "Music/Loss/[a]/y.flac"];
        let playlist = read_remote_playlist(
            text.as_bytes(),
            "Music/Playlists/p.m3u8",
            own,
            &|path| files.contains(&path),
        )
        .unwrap();
        assert_eq!(remote(&playlist, 0), (own, "Music/Loss/[a]/x (1).flac".to_owned()));
        assert_eq!(remote(&playlist, 1), (own, "Music/Loss/[a]/y.flac".to_owned()));
        // The same rule is applied to the rest (the scan reports the ones that are missing); a web address is left alone.
        assert_eq!(remote(&playlist, 2), (own, "Elsewhere/z.flac".to_owned()));
        assert!(matches!(playlist.entries[3].locator, MediaLocator::ContentUri(_)));
    }

    #[test]
    fn a_drive_that_is_a_folder_above_the_playlist_is_found() {
        let own = SourceId::new();
        let playlist = read_remote_playlist(
            b"E:\\Songs\\a.flac\n",
            "Share/Lists/p.m3u8",
            own,
            &|path| path == "Share/Songs/a.flac",
        )
        .unwrap();
        assert_eq!(remote(&playlist, 0), (own, "Share/Songs/a.flac".to_owned()));
    }

    #[test]
    fn entries_that_leave_the_server_stay_unresolved() {
        let own = SourceId::new();
        let text = "../../../up.flac\nhttp://example.com/x.mp3\n";
        let playlist =
            read_remote_playlist(text.as_bytes(), "a/list.m3u8", own, &|_| false).unwrap();
        assert!(matches!(playlist.entries[0].locator, MediaLocator::FileSystem(_)));
        assert!(matches!(playlist.entries[1].locator, MediaLocator::ContentUri(_)));
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

            fn account_id(&self, _: SourceId) -> String {
                "acc".to_owned()
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

            let report = sync_remote_playlist_source(&mut database, &source, &[], Arc::clone(&resolver), 1_000, &mut |_| {})
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
            sync_remote_playlist_source(&mut database, &source, &[], Arc::clone(&resolver), 2_000, &mut |_| {})
                .unwrap()
                .expect("second sync");
            assert_eq!(track_count(&database), 2);

            // A shorter list drops the song that is no longer named.
            fs::write(root.join("Lists/mix.m3u8"), "#EXTM3U\n../Other/f32.wv\n").unwrap();
            sync_remote_playlist_source(&mut database, &source, &[], resolver, 3_000, &mut |_| {})
                .unwrap()
                .expect("third sync");
            let saved = database.get_playlist(*playlist_id).unwrap().expect("playlist");
            assert_eq!(saved.entries.len(), 1);
            let _ = fs::remove_dir_all(root);
        }

        fn folder_root(id: SourceId, base: &str) -> LibraryRoot {
            LibraryRoot {
                id,
                kind: MediaSourceKind::Remote,
                display_name: format!("folder {base}"),
                locator: MediaLocator::Remote {
                    source_id: id,
                    path: base.to_owned(),
                },
                enabled: true,
            }
        }

        fn sync_folder(
            database: &mut Database,
            resolver: &Arc<dyn RemoteResolver>,
            root: &LibraryRoot,
            at: i64,
        ) {
            database.save_library_root(root).unwrap();
            let mut index = RemoteMediaIndex::new(Arc::clone(resolver));
            SyncEngine::sync_with_progress(root, &mut index, database, at, |_| {}).unwrap();
        }

        fn sync_list(
            database: &mut Database,
            resolver: &Arc<dyn RemoteResolver>,
            source: &SourceEntry,
            at: i64,
        ) {
            sync_remote_playlist_source(database, source, &[], Arc::clone(resolver), at, &mut |_| {})
                .unwrap()
                .expect("the playlist is reachable");
        }

        fn all_ids(database: &Database) -> Vec<player_core::TrackId> {
            let mut ids: Vec<_> = database
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
                .into_iter()
                .map(|item| item.id)
                .collect();
            ids.sort_by_key(|id| id.to_string());
            ids
        }

        /// Whatever the sources are and in whatever order they are synchronized (any number of times), a file of the
        /// server is one track, and a track that exists keeps its id when another source starts to hold it too.
        #[test]
        fn a_file_is_one_track_however_many_sources_reach_it() {
            // The list names its songs three ways: with a drive letter (the drive is the account's root), climbing out of
            // `Lists`, and with `..` pieces.
            let lists = [
                "D:\\Artist\\Album\\01 s16.wv\nD:\\Artist\\Album\\02 s24.wv\nD:\\Other\\f32.wv\n",
                "../Artist/Album/01 s16.wv\n../Artist/Album/02 s24.wv\n../Other/f32.wv\n",
                "../Lists/../Artist/Album/01 s16.wv\n../Artist/Album/02 s24.wv\n../Other/f32.wv\n",
            ];
            // Which sources there are: folder bases, and how many playlist sources.
            let folder_sets: [&[&str]; 4] = [&["Artist"], &["Artist", "Artist/Album"], &["", "Artist"], &["Artist", "Other"]];
            for text in lists {
                for folders in folder_sets {
                    for playlists_first in [false, true] {
                        for two_lists in [false, true] {
                            let root = tree("matrix");
                            fs::write(root.join("Lists/mix.m3u8"), format!("#EXTM3U\n{text}")).unwrap();
                            fs::write(root.join("Lists/other.m3u8"), format!("#EXTM3U\n{text}")).unwrap();
                            let folder_ids: Vec<SourceId> = folders.iter().map(|_| SourceId::new()).collect();
                            let list_a = SourceId::new();
                            let list_b = SourceId::new();
                            let mut bases: HashMap<SourceId, String> = folder_ids
                                .iter()
                                .zip(folders)
                                .map(|(id, base)| (*id, (*base).to_owned()))
                                .collect();
                            bases.insert(list_a, String::new());
                            bases.insert(list_b, String::new());
                            let resolver: Arc<dyn RemoteResolver> = Arc::new(Resolver {
                                fs: Arc::new(DirFs::new(&root)),
                                bases,
                            });
                            let mut database = Database::open_in_memory().unwrap();
                            let roots: Vec<_> = folder_ids
                                .iter()
                                .zip(folders)
                                .map(|(id, base)| folder_root(*id, base))
                                .collect();
                            let mut sources = vec![playlist_source(list_a, "Lists/mix.m3u8")];
                            if two_lists {
                                sources.push(playlist_source(list_b, "Lists/other.m3u8"));
                            }
                            let label = format!("{text:?} {folders:?} playlists_first={playlists_first} two={two_lists}");

                            let sync_all = |database: &mut Database, at: i64| {
                                if playlists_first {
                                    for source in &sources {
                                        sync_list(database, resolver_ref(&resolver), source, at);
                                    }
                                    for folder in &roots {
                                        sync_folder(database, resolver_ref(&resolver), folder, at + 1);
                                    }
                                } else {
                                    for folder in &roots {
                                        sync_folder(database, resolver_ref(&resolver), folder, at);
                                    }
                                    for source in &sources {
                                        sync_list(database, resolver_ref(&resolver), source, at + 1);
                                    }
                                }
                            };
                            sync_all(&mut database, 1_000);
                            let ids = all_ids(&database);
                            assert_eq!(ids.len(), 3, "{label}");
                            // Again and again: nothing changes.
                            sync_all(&mut database, 2_000);
                            sync_all(&mut database, 3_000);
                            assert_eq!(all_ids(&database), ids, "{label}");
                            // Every list entry resolves, and to one of those tracks.
                            for source in &sources {
                                let SourceEntryKind::PlaylistFile { playlist_id, .. } = &source.kind else {
                                    unreachable!()
                                };
                                let saved = database.get_playlist(*playlist_id).unwrap().expect("playlist");
                                assert_eq!(saved.entries.len(), 3, "{label}");
                                assert!(
                                    saved.entries.iter().all(|entry| entry.track_id.is_some_and(|id| ids.contains(&id))),
                                    "{label}"
                                );
                            }
                            let _ = fs::remove_dir_all(root);
                        }
                    }
                }
            }
        }

        fn resolver_ref(resolver: &Arc<dyn RemoteResolver>) -> &Arc<dyn RemoteResolver> {
            resolver
        }

        /// The user's case: the playlist source was synchronized first (and its file never changed), the folder source
        /// came later. Track ids that already exist are kept, and removing a source never loses the song.
        #[test]
        fn a_folder_added_after_a_playlist_gets_the_playlists_tracks_and_either_can_leave() {
            let root = tree("later");
            fs::write(
                root.join("Lists/mix.m3u8"),
                "#EXTM3U\n../Artist/Album/01 s16.wv\n../Other/f32.wv\n",
            )
            .unwrap();
            let folder = SourceId::new();
            let list = SourceId::new();
            let resolver: Arc<dyn RemoteResolver> = Arc::new(Resolver {
                fs: Arc::new(DirFs::new(&root)),
                bases: HashMap::from([(folder, "Artist".to_owned()), (list, String::new())]),
            });
            let mut database = Database::open_in_memory().unwrap();
            let source = playlist_source(list, "Lists/mix.m3u8");
            sync_list(&mut database, &resolver, &source, 1_000);
            let first = all_ids(&database);
            assert_eq!(first.len(), 2);

            let folder_root = folder_root(folder, "Artist");
            sync_folder(&mut database, &resolver, &folder_root, 2_000);
            sync_list(&mut database, &resolver, &source, 3_000);
            // Two files in the folder (one of them in the list): three files, three tracks, the old ones kept.
            let both = all_ids(&database);
            assert_eq!(both.len(), 3);
            assert!(first.iter().all(|id| both.contains(id)));

            // The folder source is removed: its mappings go, the list keeps the songs it names.
            database
                .remove_playlist_file_source(folder)
                .unwrap();
            sync_list(&mut database, &resolver, &source, 4_000);
            let after = all_ids(&database);
            assert!(first.iter().all(|id| after.contains(id)));
            let _ = fs::remove_dir_all(root);
        }
    }

    /// Against the real server (skipped without `MOE_SMB_HOST`, `MOE_SMB_SHARE`, `MOE_SMB_FILE`, `MOE_SMB_PLAYLIST`): the
    /// playlist is read, mapped, and its songs are found.
    #[test]
    fn a_real_playlist_with_drive_paths_is_mapped_to_files_on_the_server() {
        let (Ok(host), Ok(share), Ok(file), Ok(playlist_path)) = (
            std::env::var("MOE_SMB_HOST"),
            std::env::var("MOE_SMB_SHARE"),
            std::env::var("MOE_SMB_FILE"),
            std::env::var("MOE_SMB_PLAYLIST"),
        ) else {
            eprintln!("skipped: MOE_SMB_* is not set");
            return;
        };
        let text = std::fs::read_to_string(file).unwrap();
        let mut lines = text.lines();
        let fs = player_remote::smb::SmbFs::connect(player_remote::smb::SmbSettings {
            host,
            port: std::env::var("MOE_SMB_PORT").ok().and_then(|port| port.parse().ok()),
            share,
            base_path: String::new(),
            user: lines.next().unwrap().trim().to_owned(),
            password: zeroize::Zeroizing::new(lines.next().unwrap().trim().to_owned()),
        })
        .unwrap();
        let bytes = read_all(&fs, &playlist_path).unwrap();
        let own = SourceId::new();
        let total = bytes.iter().filter(|byte| **byte == b'\n').count();
        let playlist = read_remote_playlist(&bytes, &playlist_path, own, &|path| {
            fs.stat(path).is_ok_and(|entry| !entry.is_dir)
        })
        .unwrap();
        // Writing is the inverse of reading: every entry of the real file comes back as the same text.
        let text = player_remote::playlist_edit::decode_text(&bytes, false).unwrap();
        let raws = player_remote::playlist_edit::entry_texts(&text);
        let folder = player_remote::playlist_edit::folder_of(&playlist_path);
        let style = PlaylistStyle::detect(&raws, folder, &|path| fs.stat(path).is_ok_and(|entry| !entry.is_dir));
        println!("style: {style:?}");
        let mut round_tripped = 0;
        for raw in &raws {
            if let Some(server) = style.to_server_path(raw, folder) {
                assert_eq!(&style.to_entry_text(&server, folder), raw);
                round_tripped += 1;
            }
        }
        println!("{round_tripped} of {} entries round-trip", raws.len());
        let mapped: Vec<_> = playlist
            .entries
            .iter()
            .filter_map(|entry| match &entry.locator {
                MediaLocator::Remote { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();
        println!("{} of {total} entries mapped; first: {:?}", mapped.len(), mapped.first());
        let found = mapped.iter().take(10).filter(|path| fs.stat(path).is_ok()).count();
        println!("{found} of the first 10 exist");
        assert!(!mapped.is_empty());
        assert_eq!(found, mapped.len().min(10));
    }
}
