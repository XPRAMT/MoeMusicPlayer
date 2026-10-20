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
        Some(format!("remote:{}:Artist/Album One/01 s16.wv", scan.source_id).as_str())
    );
    let _ = fs::remove_dir_all(tree);
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
    assert_eq!(paths(&scan), vec!["Album One/01 s16.wv", "Album One/02 s24.wv"]);

    // A source inside an excluded folder scans nothing and is complete (what was in it is removed).
    let mut index = index_excluding(Arc::new(DirFs::new(&tree)), "Artist/Album One", &["Artist"]);
    let scan = index.scan(&root_for("Artist/Album One"));
    assert_eq!(scan.state, SourceScanState::Complete);
    assert!(scan.tracks.is_empty());
    let _ = fs::remove_dir_all(tree);
}
