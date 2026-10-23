//! The remote scanner and tag reader against a folder behind the `RemoteFs` trait, then through the real sync engine
//! into a library database (ADR 0026). The tracks are the WavPack fixtures of `player-wavpack`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use player_core::{
    LibraryRoot, ListTracksQuery, MediaIndex, MediaLocator, MediaSourceKind, MetadataReader,
    SourceId, SourceScanState, SyncCancellation, SyncEngine,
};
use player_db::Database;
use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::local::DirFs;
use player_track_metadata::{read_track_metadata, PathSource};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../player-wavpack/tests/fixtures")
}

/// A folder tree of copied fixtures: two albums, a hidden folder, a text file and a non-audio sidecar.
fn make_tree(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("moe-remote-index-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (relative, fixture) in [
        ("Artist/Album One/01 s16.wv", "s16.wv"),
        ("Artist/Album One/02 s24.wv", "s24.wv"),
        ("Artist/Album Two/03 lyrics.wv", "s16_lyrics.wv"),
        ("Other/f32.wv", "f32.wv"),
        (
            "Other/Smoke.mka",
            "../../../player-audio-windows/tests/fixtures/metadata-smoke.mka",
        ),
        (".hidden/skipped.wv", "s16.wv"),
        ("Artist/Album One/notes.txt", "s16.raw"),
    ] {
        let target = root.join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(fixtures().join(fixture), target).unwrap();
    }
    root
}

struct Fixed {
    fs: Arc<dyn RemoteFs>,
    base: String,
    excluded: Vec<String>,
}

impl RemoteResolver for Fixed {
    fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
        Ok(ResolvedSource {
            fs: Arc::clone(&self.fs),
            base_path: self.base.clone(),
            excluded: self.excluded.clone(),
        })
    }
}

fn root_for(base: &str) -> LibraryRoot {
    let id = SourceId::new();
    LibraryRoot {
        id,
        kind: MediaSourceKind::Remote,
        display_name: "remote".to_owned(),
        locator: MediaLocator::Remote {
            source_id: id,
            path: base.to_owned(),
        },
        enabled: true,
    }
}

fn index_over(fs: Arc<dyn RemoteFs>, base: &str) -> RemoteMediaIndex {
    index_excluding(fs, base, &[])
}

fn index_excluding(fs: Arc<dyn RemoteFs>, base: &str, excluded: &[&str]) -> RemoteMediaIndex {
    RemoteMediaIndex::new(Arc::new(Fixed {
        fs,
        base: base.to_owned(),
        excluded: excluded.iter().map(|folder| (*folder).to_owned()).collect(),
    }))
}

fn paths(scan: &player_core::SourceScan) -> Vec<String> {
    scan.tracks
        .iter()
        .map(|track| track.identity.source_item_id.clone())
        .collect()
}

#[test]
fn scans_the_whole_tree_and_skips_hidden_and_non_audio() {
    let tree = make_tree("whole");
    let mut index = index_over(Arc::new(DirFs::new(&tree)), "");
    let scan = index.scan(&root_for(""));
    assert_eq!(scan.state, SourceScanState::Complete);
    assert_eq!(
        paths(&scan),
        vec![
            "Artist/Album One/01 s16.wv",
            "Artist/Album One/02 s24.wv",
            "Artist/Album Two/03 lyrics.wv",
            "Other/Smoke.mka",
            "Other/f32.wv",
        ]
    );
    let first = &scan.tracks[0];
    assert_eq!(
        first.fingerprint.size_bytes,
        fs::metadata(tree.join("Artist/Album One/01 s16.wv"))
            .unwrap()
            .len()
    );
    assert!(first.fingerprint.modified_at_utc_ms.is_some());
    assert_eq!(
        first.identity.locator_key.as_deref(),
        Some(format!("remote-acct:{}:Artist/Album One/01 s16.wv", scan.source_id).as_str())
    );
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn looking_songs_up_by_path_reports_how_far_it_got_and_ends_complete() {
    let tree = make_tree("progress");
    let index = index_over(Arc::new(DirFs::new(&tree)), "");
    let wanted: Vec<String> = [
        "Artist/Album One/01 s16.wv",
        "Artist/Album One/02 s24.wv",
        "Artist/Album Two/03 lyrics.wv",
        "Other/f32.wv",
        "notes/not-audio.txt",
    ]
    .iter()
    .map(|path| (*path).to_owned())
    .collect();
    let mut reports = Vec::new();
    let scan = index.scan_paths_with_progress(SourceId::new(), &wanted, &mut |progress| {
        reports.push((progress.processed, progress.total.unwrap_or(0)));
    });
    assert_eq!(scan.tracks.len(), 4);
    // Only the audio files are looked up; the count never goes backwards and finishes at the total.
    assert_eq!(reports.first(), Some(&(0, 4)), "{reports:?}");
    assert_eq!(reports.last(), Some(&(4, 4)), "{reports:?}");
    assert!(reports.windows(2).all(|pair| pair[0].0 <= pair[1].0), "{reports:?}");
    assert!(reports.iter().all(|(_, total)| *total == 4), "{reports:?}");
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn a_song_gone_from_a_reachable_server_is_missing_but_an_unreachable_top_folder_is_offline() {
    use player_core::MediaSourceErrorKind;
    let tree = make_tree("missing-vs-offline");
    let index = index_over(Arc::new(DirFs::new(&tree)), "");
    let wanted: Vec<String> = ["Other/f32.wv", "Gone/away.wv"].iter().map(|path| (*path).to_owned()).collect();
    let scan = index.scan_paths(SourceId::new(), &wanted);
    assert_eq!(scan.tracks.len(), 1);
    assert_eq!(scan.errors.len(), 1);
    assert_eq!(scan.errors[0].kind, MediaSourceErrorKind::MissingFile);
    assert_eq!(scan.state, SourceScanState::Complete, "only missing files: the playlist scan is complete");

    // The whole share is gone (or unplugged): the same not-found answers say nothing about the songs.
    let _ = fs::remove_dir_all(&tree);
    let scan = index.scan_paths(SourceId::new(), &wanted);
    assert!(scan.tracks.is_empty());
    assert!(!scan.errors.is_empty());
    assert!(scan.errors.iter().all(|error| error.kind == MediaSourceErrorKind::Other), "{:?}", scan.errors);
    assert!(matches!(scan.state, SourceScanState::Incomplete { .. }));
}

#[test]
fn a_source_rooted_in_a_folder_lists_paths_relative_to_it() {
    let tree = make_tree("base");
    let mut index = index_over(Arc::new(DirFs::new(&tree)), "Artist");
    let scan = index.scan(&root_for("Artist"));
    assert_eq!(scan.state, SourceScanState::Complete);
    assert_eq!(
        paths(&scan),
        vec![
            "Album One/01 s16.wv",
            "Album One/02 s24.wv",
            "Album Two/03 lyrics.wv"
        ]
    );
    // The tags are read through the same base.
    let metadata = index.read(&scan.tracks[1]).expect("metadata");
    assert_eq!(metadata.sample_rate_hz, Some(96_000));
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn remote_tags_equal_the_tags_read_from_the_local_file() {
    let tree = make_tree("tags");
    let index = index_over(Arc::new(DirFs::new(&tree)), "");
    let mut scanner = index_over(Arc::new(DirFs::new(&tree)), "");
    let scan = scanner.scan(&root_for(""));
    assert_eq!(scan.tracks.len(), 5);
    for track in &scan.tracks {
        let MediaLocator::Remote { path, .. } = &track.locator else {
            panic!("a remote locator");
        };
        let remote = index.read(track).expect("remote read");
        let local = read_track_metadata(&PathSource(&tree.join(path))).expect("local read");
        assert_eq!(remote, local, "{path}");
    }
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn an_unreachable_or_refusing_server_never_looks_like_an_empty_library() {
    struct Failing(RemoteError);
    impl RemoteResolver for Failing {
        fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
            Err(self.0.clone())
        }
    }
    let mut down = RemoteMediaIndex::new(Arc::new(Failing(RemoteError::Network("timeout".into()))));
    let scan = down.scan(&root_for(""));
    assert!(matches!(scan.state, SourceScanState::Unavailable { .. }));
    assert!(scan.tracks.is_empty());

    let mut refused = RemoteMediaIndex::new(Arc::new(Failing(RemoteError::Auth)));
    assert!(matches!(
        refused.scan(&root_for("")).state,
        SourceScanState::PermissionRevoked { .. }
    ));

    // A folder that is gone is unavailable too, not an empty complete scan.
    let tree = make_tree("gone");
    let mut missing = index_over(Arc::new(DirFs::new(&tree)), "no/such/folder");
    assert!(matches!(
        missing.scan(&root_for("no/such/folder")).state,
        SourceScanState::Unavailable { .. }
    ));
    let _ = fs::remove_dir_all(tree);
}

/// A tree whose one folder cannot be listed: its siblings are still found, and the scan is not `Complete`, so the
/// library keeps what it already knew about the missing folder.
#[test]
fn a_folder_that_cannot_be_listed_makes_the_scan_incomplete() {
    struct Flaky(DirFs);
    impl RemoteFs for Flaky {
        fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
            if dir.ends_with("Album Two") {
                return Err(RemoteError::Network("link dropped".into()));
            }
            self.0.list(dir)
        }
        fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
            self.0.stat(path)
        }
        fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
            self.0.open(path)
        }
    }
    let tree = make_tree("flaky");
    let mut index = index_over(Arc::new(Flaky(DirFs::new(&tree))), "");
    let scan = index.scan(&root_for(""));
    assert!(matches!(scan.state, SourceScanState::Incomplete { .. }));
    assert_eq!(scan.tracks.len(), 4);
    assert_eq!(scan.errors.len(), 1);
    assert_eq!(
        scan.errors[0].source_item_id.as_deref(),
        Some("Artist/Album Two")
    );
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn a_cancelled_scan_is_incomplete() {
    let tree = make_tree("cancel");
    let mut index = index_over(Arc::new(DirFs::new(&tree)), "");
    let cancellation = SyncCancellation::default();
    cancellation.cancel();
    let scan = index.scan_with_progress(&root_for(""), &mut |_| {}, &cancellation);
    assert!(matches!(scan.state, SourceScanState::Incomplete { .. }));
    assert!(scan.tracks.is_empty());
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn the_sync_engine_stores_remote_tracks_and_leaves_them_when_the_server_is_down() {
    let tree = make_tree("engine");
    let mut db = Database::open_in_memory().expect("database");
    let root = root_for("");
    db.save_library_root(&root).expect("save the root");

    let mut index = index_over(Arc::new(DirFs::new(&tree)), "");
    let report = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000).expect("sync");
    assert_eq!(report.state, Some(SourceScanState::Complete));
    assert_eq!(report.observed, 5);
    assert!(
        report.metadata_errors.is_empty(),
        "{:?}",
        report.metadata_errors
    );

    let page = |db: &Database| {
        db.list_tracks_page(ListTracksQuery {
            offset: 0,
            limit: 20,
            query: None,
            field_filter: None,
            sort: Default::default(),
        })
        .expect("page")
    };
    let first = page(&db);
    assert_eq!(first.items.len(), 5);
    let locators = db.track_locators(first.items[0].id).expect("locators");
    assert!(matches!(
        locators.as_slice(),
        [MediaLocator::Remote { source_id, .. }] if *source_id == root.id
    ));

    // A second scan changes nothing: every track is unchanged by size and modified time.
    let again = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000).expect("again");
    assert_eq!(again.metadata_reads, 0);
    assert_eq!(again.unchanged, 5);

    // The server goes away: the scan is unavailable and not one track is removed.
    struct Down;
    impl RemoteResolver for Down {
        fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
            Err(RemoteError::Network("host is down".into()))
        }
    }
    let mut offline = RemoteMediaIndex::new(Arc::new(Down));
    let report = SyncEngine::sync(&root, &mut offline, &mut db, 1_800_000_200_000).expect("down");
    assert!(matches!(
        report.state,
        Some(SourceScanState::Unavailable { .. })
    ));
    assert_eq!(page(&db).items.len(), 5);

    let _ = fs::remove_dir_all(tree);
    let _: HashMap<(), ()> = HashMap::new();
}

#[test]
fn excluded_folders_and_everything_under_them_are_skipped() {
    let tree = make_tree("excluded");
    // Under the whole tree: one album and one other folder.
    let mut index = index_excluding(
        Arc::new(DirFs::new(&tree)),
        "",
        &["Artist/Album One", "Other"],
    );
    let scan = index.scan(&root_for(""));
    assert_eq!(scan.state, SourceScanState::Complete);
    assert_eq!(paths(&scan), vec!["Artist/Album Two/03 lyrics.wv"]);

    // A name that only starts like an excluded folder is not excluded ("Album" vs "Album One").
    let mut index = index_excluding(Arc::new(DirFs::new(&tree)), "", &["Artist/Album"]);
    assert_eq!(index.scan(&root_for("")).tracks.len(), 5);

    // The exclusion is relative to the account's root, also when the source is rooted in a folder.
    let mut index = index_excluding(Arc::new(DirFs::new(&tree)), "Artist", &["Artist/Album Two"]);
    let scan = index.scan(&root_for("Artist"));
    assert_eq!(
        paths(&scan),
        vec!["Album One/01 s16.wv", "Album One/02 s24.wv"]
    );

    // A source inside an excluded folder scans nothing and is complete (what was in it is removed).
    let mut index = index_excluding(Arc::new(DirFs::new(&tree)), "Artist/Album One", &["Artist"]);
    let scan = index.scan(&root_for("Artist/Album One"));
    assert_eq!(scan.state, SourceScanState::Complete);
    assert!(scan.tracks.is_empty());
    let _ = fs::remove_dir_all(tree);
}

/// The remote content provider over a folder, so the sync can read the `.lrc` files it found.
struct FolderContent(Arc<dyn RemoteFs>);

impl player_core::RemoteContent for FolderContent {
    fn open(&self, _: SourceId, path: &str) -> std::io::Result<player_core::RemoteFileHandle> {
        let reader = player_remote::fs::open_reader(self.0.as_ref(), path)?;
        let len = reader.len();
        Ok(player_core::RemoteFileHandle {
            reader: Box::new(reader),
            len,
        })
    }
}

#[test]
fn the_sync_stores_the_lrc_beside_a_song_with_its_fingerprint_and_follows_changes() {
    use player_db::LyricSidecarLookup;

    let tree = make_tree("lrc");
    fs::write(tree.join("Artist/Album One/01 s16.lrc"), "\u{feff}[00:01.00]hello\n").unwrap();
    // The name matches whatever the case of the extension and the name.
    fs::write(tree.join("Other/F32.LRC"), "[00:02.00]loud\n").unwrap();
    let fs_tree: Arc<dyn RemoteFs> = Arc::new(DirFs::new(&tree));
    player_core::set_remote_content(Arc::new(FolderContent(Arc::clone(&fs_tree))));
    let mut db = Database::open_in_memory().expect("database");
    let root = root_for("");
    db.save_library_root(&root).expect("save the root");
    let mut index = index_over(Arc::clone(&fs_tree), "");
    SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000).expect("sync");

    let lookup = |db: &Database, path: &str| {
        let id = db
            .resolve_track_id_for_locator(&MediaLocator::Remote {
                source_id: root.id,
                path: path.to_owned(),
            })
            .expect("resolve")
            .expect("the track is in the library");
        db.track_lyric_sidecar(id).expect("lookup")
    };
    assert_eq!(
        lookup(&db, "Artist/Album One/01 s16.wv"),
        LyricSidecarLookup::Present { text: Some("[00:01.00]hello\n".to_owned()), oversized: false },
        "the BOM is dropped"
    );
    assert!(matches!(lookup(&db, "Other/f32.wv"), LyricSidecarLookup::Present { text: Some(_), .. }));
    // A song of a source that was scanned and has no `.lrc` is known to have none.
    assert_eq!(lookup(&db, "Artist/Album One/02 s24.wv"), LyricSidecarLookup::Absent);

    // An edited `.lrc` is read again (its size changed); an unchanged one is not touched.
    fs::write(tree.join("Artist/Album One/01 s16.lrc"), "[00:01.00]hello again\n").unwrap();
    SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000).expect("sync again");
    assert_eq!(
        lookup(&db, "Artist/Album One/01 s16.wv"),
        LyricSidecarLookup::Present { text: Some("[00:01.00]hello again\n".to_owned()), oversized: false }
    );
    // The lyrics in a song's own tags are kept from the sync that read the tags (or the fact that there are none).
    let embedded = |db: &Database, path: &str| {
        let id = db
            .resolve_track_id_for_locator(&MediaLocator::Remote {
                source_id: root.id,
                path: path.to_owned(),
            })
            .expect("resolve")
            .expect("the track is in the library");
        db.track_embedded_lyrics(id).expect("embedded lookup")
    };
    assert!(
        matches!(embedded(&db, "Artist/Album Two/03 lyrics.wv"), Some(Some(text)) if !text.trim().is_empty()),
        "the APEv2 lyrics of the WavPack file"
    );
    assert_eq!(embedded(&db, "Artist/Album One/01 s16.wv"), Some(None), "read, and holds no lyrics");

    // A removed `.lrc` is forgotten at the next complete scan.
    fs::remove_file(tree.join("Artist/Album One/01 s16.lrc")).unwrap();
    SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_200_000).expect("sync a third time");
    assert_eq!(lookup(&db, "Artist/Album One/01 s16.wv"), LyricSidecarLookup::Absent);
    let _ = fs::remove_dir_all(tree);
}
/// A server that reports creation times (SMB does, in its directory listing and in `stat`), or holds them back like
/// an older build of the scanner that did not keep them. It counts the requests, so a test can see that the times
/// come with the listing and cost nothing extra.
struct WithCreationTimes {
    inner: DirFs,
    report: std::sync::atomic::AtomicBool,
    lists: std::sync::atomic::AtomicU64,
    stats: std::sync::atomic::AtomicU64,
}

/// The creation time the fake server gives a file, in 100 ns ticks since the Unix epoch: whole days apart by path, and
/// 1.2345 ms past the millisecond so the rounding is visible.
fn fake_creation_ticks(path: &str) -> i64 {
    let days: i64 = path.bytes().map(i64::from).sum();
    (1_700_000_000_000 + days * 86_400_000) * 10_000 + 12_345
}

impl WithCreationTimes {
    fn new(root: &Path) -> Self {
        Self {
            inner: DirFs::new(root),
            report: std::sync::atomic::AtomicBool::new(true),
            lists: Default::default(),
            stats: Default::default(),
        }
    }

    fn with_time(&self, path: &str, mut entry: RemoteEntry) -> RemoteEntry {
        if !entry.is_dir && self.report.load(std::sync::atomic::Ordering::SeqCst) {
            entry.created_utc_100ns = Some(fake_creation_ticks(path.trim_matches('/')));
        }
        entry
    }
}

impl RemoteFs for WithCreationTimes {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.lists.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self
            .inner
            .list(dir)?
            .into_iter()
            .map(|entry| {
                let path = if dir.trim_matches('/').is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", dir.trim_matches('/'), entry.name)
                };
                self.with_time(&path, entry)
            })
            .collect())
    }
    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.stats.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let entry = self.inner.stat(path)?;
        Ok(self.with_time(path, entry))
    }
    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        self.inner.open(path)
    }
}

fn created_by_path(db: &Database, root: &LibraryRoot) -> Vec<(String, Option<i64>, Option<i64>)> {
    let page = db
        .list_tracks_page(ListTracksQuery {
            offset: 0,
            limit: 20,
            query: None,
            field_filter: None,
            sort: player_core::TrackListSort {
                field: player_core::TrackSortField::CreatedAt,
                direction: player_core::TrackSortDirection::Asc,
            },
        })
        .expect("page");
    page.items
        .iter()
        .map(|item| {
            let path = db
                .track_locators(item.id)
                .expect("locators")
                .into_iter()
                .find_map(|locator| match locator {
                    MediaLocator::Remote { source_id, path } if source_id == root.id => Some(path),
                    _ => None,
                })
                .expect("a remote locator");
            (path, item.created_at_utc_ms, item.modified_at_utc_ms)
        })
        .collect()
}

#[test]
fn a_scan_keeps_the_creation_time_the_listing_gives_and_fills_it_in_for_unchanged_tracks() {
    let tree = make_tree("created");
    let server = Arc::new(WithCreationTimes::new(&tree));
    // A library made before the scan kept creation times: the server gives none.
    server
        .report
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut db = Database::open_in_memory().expect("database");
    let root = root_for("");
    db.save_library_root(&root).expect("save the root");
    let mut index = index_over(Arc::clone(&server) as Arc<dyn RemoteFs>, "");
    SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000).expect("sync");
    let before = created_by_path(&db, &root);
    assert_eq!(before.len(), 5);
    assert!(before
        .iter()
        .all(|(_, created, modified)| created.is_none() && modified.is_some()));
    let lists_without = server.lists.load(std::sync::atomic::Ordering::SeqCst);

    // The next sync sees the times. Nothing else changed, so no tag is read again, and the listing is all it asks.
    server
        .report
        .store(true, std::sync::atomic::Ordering::SeqCst);
    server.lists.store(0, std::sync::atomic::Ordering::SeqCst);
    let again = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000).expect("again");
    assert_eq!(again.metadata_reads, 0);
    assert_eq!(again.unchanged, 5);
    assert_eq!(
        server.lists.load(std::sync::atomic::Ordering::SeqCst),
        lists_without
    );
    assert_eq!(
        server.stats.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no stat per file"
    );
    let after = created_by_path(&db, &root);
    for (path, created, modified) in &after {
        // 100 ns ticks to milliseconds, rounded down like the local scanners.
        assert_eq!(
            *created,
            Some(fake_creation_ticks(path).div_euclid(10_000)),
            "{path}"
        );
        let on_disk = fs::metadata(tree.join(path)).unwrap().modified().unwrap();
        let on_disk = on_disk
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert_eq!(*modified, Some(on_disk), "{path}");
    }
    // The list sorted by 新增時間 follows the server's creation times.
    let mut expected: Vec<i64> = after
        .iter()
        .map(|(_, created, _)| created.unwrap())
        .collect();
    expected.sort_unstable();
    assert_eq!(
        after
            .iter()
            .map(|(_, created, _)| created.unwrap())
            .collect::<Vec<_>>(),
        expected
    );

    // The queue and every other list read the same summary.
    let first = db
        .resolve_track_id_for_locator(&MediaLocator::Remote {
            source_id: root.id,
            path: after[0].0.clone(),
        })
        .expect("resolve")
        .expect("in the library");
    let summary = db.get_track_summaries(&[first]).expect("summaries")[0]
        .clone()
        .expect("summary");
    assert_eq!(summary.created_at_utc_ms, after[0].1);
    assert_eq!(summary.modified_at_utc_ms, after[0].2);

    // A third sync finds every time already stored and changes nothing.
    let third = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_200_000).expect("third");
    assert_eq!(third.metadata_reads, 0);
    assert_eq!(created_by_path(&db, &root), after);
    let _ = fs::remove_dir_all(tree);
}

#[test]
fn songs_looked_up_by_path_carry_the_creation_time_of_their_stat() {
    let tree = make_tree("created-paths");
    let server = Arc::new(WithCreationTimes::new(&tree));
    let index = index_over(Arc::clone(&server) as Arc<dyn RemoteFs>, "");
    let wanted = vec![
        "Artist/Album One/01 s16.wv".to_owned(),
        "Other/f32.wv".to_owned(),
    ];
    let scan = index.scan_paths(SourceId::new(), &wanted);
    assert_eq!(scan.tracks.len(), 2);
    for track in &scan.tracks {
        assert_eq!(
            track.fingerprint.created_at_utc_ms,
            Some(fake_creation_ticks(&track.identity.source_item_id).div_euclid(10_000))
        );
    }
    // One stat per song, as before: the time comes with it.
    assert_eq!(server.stats.load(std::sync::atomic::Ordering::SeqCst), 2);
    let _ = fs::remove_dir_all(tree);
}
