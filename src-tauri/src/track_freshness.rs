//! A song that starts playing is checked against its file in the background (ADR 0040): the file is the truth and the
//! library only an index of it. When the size or modified time differ from what the last scan recorded, everything that
//! does not touch playback is refreshed at once (tags, the album's covers, lyrics, the recorded size and time); the audio
//! that plays is never reopened, sought or waited for. A remote song is checked only when its server answers (a server
//! known to be down fails at once through the connection's backoff and the check is dropped without a word).

use player_core::{FileFingerprint, MediaLocator, RemoteContent, TrackId, TrackMetadata};
use player_db::Database;

/// What the check of one song found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FileCheck {
    /// The file is as the library recorded it.
    Unchanged,
    /// The file changed, and the library now holds what it is (tags, size and time).
    Refreshed,
    /// Nothing could be asked (no file of the library, a content URI, a server that does not answer, a read that
    /// failed): nothing was changed.
    Skipped,
}

/// The size and modified time of a file now: a local stat, or the server's answer for a remote file (`None` when it
/// cannot be had, whatever the reason).
fn stat_now(
    locator: &MediaLocator,
    remote: Option<&dyn RemoteContent>,
) -> Option<(u64, Option<i64>)> {
    match locator {
        MediaLocator::FileSystem(path) => {
            let metadata = std::fs::metadata(path)
                .ok()
                .filter(std::fs::Metadata::is_file)?;
            let modified = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .and_then(|duration| i64::try_from(duration.as_millis()).ok());
            Some((metadata.len(), modified))
        }
        MediaLocator::Remote { source_id, path } => remote?
            .stat(*source_id, path)
            .ok()
            .map(|stamp| (stamp.len, stamp.modified_ms)),
        MediaLocator::ContentUri(_) => None,
    }
}

/// Whether the file is not the one the library recorded: another size, or another modified time when both are known.
pub(crate) fn file_differs(recorded: &FileFingerprint, size: u64, modified: Option<i64>) -> bool {
    recorded.size_bytes != size
        || matches!((recorded.modified_at_utc_ms, modified), (Some(then), Some(now)) if then != now)
}

/// Check the file of `track_id` and, when it changed, read its tags with `read_metadata` (the reader the sync uses) and
/// store them with the file's size and time now. `remote` answers for remote files (the app's `RemoteContent`).
/// Blocking: run it off the playback path.
pub(crate) fn check_track_file(
    database: &Database,
    track_id: TrackId,
    remote: Option<&dyn RemoteContent>,
    read_metadata: impl FnOnce(&MediaLocator) -> Result<TrackMetadata, String>,
) -> FileCheck {
    let Ok(Some((source_id, source_item_id, locator, recorded))) =
        database.track_file_mapping(track_id)
    else {
        return FileCheck::Skipped;
    };
    let Some((size, modified)) = stat_now(&locator, remote) else {
        return FileCheck::Skipped;
    };
    if !file_differs(&recorded, size, modified) {
        return FileCheck::Unchanged;
    }
    if let MediaLocator::Remote { source_id, path } = &locator {
        // Covers, tags and lyrics of the new version are read from the server, not from the cached old one; what
        // plays keeps its own bytes.
        if let Some(content) = remote {
            content.file_changed(
                *source_id,
                path,
                player_core::RemoteStamp {
                    len: size,
                    modified_ms: modified,
                    created_utc_100ns: None,
                },
            );
        }
        player_core::forget_remote_tag_lyrics(*source_id, path);
    }
    let Ok(metadata) = read_metadata(&locator) else {
        return FileCheck::Skipped;
    };
    if database
        .refresh_track_metadata(track_id, &metadata, &locator)
        .and_then(|()| {
            database.update_mapping_fingerprint(source_id, &source_item_id, size, modified)
        })
        .is_err()
    {
        return FileCheck::Skipped;
    }
    FileCheck::Refreshed
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_core::{
        LibraryRepository, LibraryRoot, MediaSourceKind, MediaTrackRecord, RemoteContent,
        RemoteFileHandle, RemoteStamp, SourceId, SourceScanState, TrackIdentity,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn library_with(
        locator: MediaLocator,
        kind: MediaSourceKind,
        size: u64,
        modified: Option<i64>,
    ) -> (Database, TrackId) {
        let source_id = match &locator {
            MediaLocator::Remote { source_id, .. } => *source_id,
            _ => SourceId::new(),
        };
        let root = LibraryRoot {
            id: source_id,
            kind,
            display_name: "check".to_owned(),
            locator: match &locator {
                MediaLocator::FileSystem(path) => {
                    MediaLocator::FileSystem(path.parent().unwrap().to_path_buf())
                }
                MediaLocator::Remote { source_id, .. } => MediaLocator::Remote {
                    source_id: *source_id,
                    path: String::new(),
                },
                other => other.clone(),
            },
            enabled: true,
        };
        let mut database = Database::open_in_memory().expect("database");
        database.save_library_root(&root).expect("root");
        let record = MediaTrackRecord {
            identity: TrackIdentity {
                source_id,
                source_item_id: "song".to_owned(),
                locator_key: None,
            },
            locator: locator.clone(),
            fingerprint: FileFingerprint {
                size_bytes: size,
                modified_at_utc_ms: modified,
                created_at_utc_ms: None,
            },
            metadata: Some(TrackMetadata {
                title: Some("舊曲名".to_owned()),
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
            .expect("scan");
        let id = database
            .resolve_track_id_for_locator(&locator)
            .expect("resolve")
            .expect("track");
        (database, id)
    }

    fn title(database: &Database, id: TrackId) -> Option<String> {
        database.get_track_summary(id).unwrap().unwrap().title
    }

    fn new_tags(_: &MediaLocator) -> Result<TrackMetadata, String> {
        Ok(TrackMetadata {
            title: Some("新曲名".to_owned()),
            ..TrackMetadata::default()
        })
    }

    #[test]
    fn a_local_file_as_recorded_is_left_alone_and_a_changed_one_is_read_again() {
        let dir = std::env::temp_dir().join(format!("moe-freshness-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.flac");
        std::fs::write(&path, b"first").unwrap();
        let (size, modified) = stat_now(&MediaLocator::FileSystem(path.clone()), None).unwrap();
        let (database, id) = library_with(
            MediaLocator::FileSystem(path.clone()),
            MediaSourceKind::WindowsFilesystem,
            size,
            modified,
        );

        let mut read = 0;
        let outcome = check_track_file(&database, id, None, |_| {
            read += 1;
            new_tags(&MediaLocator::FileSystem(path.clone()))
        });
        assert_eq!(outcome, FileCheck::Unchanged);
        assert_eq!(read, 0, "an unchanged file is not read");
        assert_eq!(title(&database, id).as_deref(), Some("舊曲名"));

        std::fs::write(&path, b"edited elsewhere").unwrap();
        assert_eq!(
            check_track_file(&database, id, None, new_tags),
            FileCheck::Refreshed
        );
        assert_eq!(title(&database, id).as_deref(), Some("新曲名"));
        // The new size and time are recorded: the next start (and the next sync) sees the file as it is.
        let (_, _, _, recorded) = database.track_file_mapping(id).unwrap().unwrap();
        assert_eq!(recorded.size_bytes, "edited elsewhere".len() as u64);
        assert_eq!(
            check_track_file(&database, id, None, |_| panic!("not read again")),
            FileCheck::Unchanged
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_gone_or_unreadable_changes_nothing() {
        let dir = std::env::temp_dir().join(format!("moe-freshness-gone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.flac");
        let (database, id) = library_with(
            MediaLocator::FileSystem(path.clone()),
            MediaSourceKind::WindowsFilesystem,
            5,
            Some(1),
        );
        assert_eq!(
            check_track_file(&database, id, None, new_tags),
            FileCheck::Skipped
        );
        std::fs::write(&path, b"other").unwrap();
        assert_eq!(
            check_track_file(&database, id, None, |_| Err("unreadable".to_owned())),
            FileCheck::Skipped
        );
        assert_eq!(title(&database, id).as_deref(), Some("舊曲名"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A server: its answer to `stat`, or a network error when it is down; counts what it is asked and what the
    /// player was told changed.
    struct Server {
        up: bool,
        len: u64,
        stats: AtomicUsize,
        changed: std::sync::Mutex<Vec<RemoteStamp>>,
    }

    impl RemoteContent for Server {
        fn open(&self, _: SourceId, _: &str) -> std::io::Result<RemoteFileHandle> {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        }

        fn stat(&self, _: SourceId, _: &str) -> std::io::Result<RemoteStamp> {
            self.stats.fetch_add(1, Ordering::SeqCst);
            if self.up {
                Ok(RemoteStamp {
                    len: self.len,
                    modified_ms: Some(2_000),
                    created_utc_100ns: None,
                })
            } else {
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionAborted))
            }
        }

        fn file_changed(&self, _: SourceId, _: &str, now: RemoteStamp) {
            self.changed.lock().unwrap().push(now);
        }
    }

    #[test]
    fn a_remote_song_is_checked_only_when_its_server_answers() {
        let source = SourceId::new();
        let locator = MediaLocator::Remote {
            source_id: source,
            path: "a/song.flac".to_owned(),
        };

        let down = Arc::new(Server {
            up: false,
            len: 10,
            stats: AtomicUsize::new(0),
            changed: Default::default(),
        });
        let (database, id) =
            library_with(locator.clone(), MediaSourceKind::Remote, 10, Some(1_000));
        let started = std::time::Instant::now();
        assert_eq!(
            check_track_file(&database, id, Some(down.as_ref()), |_| panic!("not read")),
            FileCheck::Skipped
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        assert_eq!(down.stats.load(Ordering::SeqCst), 1);
        assert_eq!(title(&database, id).as_deref(), Some("舊曲名"));

        let up_server = Arc::new(Server {
            up: true,
            len: 12,
            stats: AtomicUsize::new(0),
            changed: Default::default(),
        });
        let up: &dyn RemoteContent = up_server.as_ref();
        assert_eq!(
            check_track_file(&database, id, Some(up), new_tags),
            FileCheck::Refreshed
        );
        assert_eq!(title(&database, id).as_deref(), Some("新曲名"));
        assert_eq!(
            up_server.changed.lock().unwrap().len(),
            1,
            "the disk cache stops serving the old version"
        );
        assert_eq!(
            check_track_file(&database, id, Some(up), |_| panic!("not read again")),
            FileCheck::Unchanged
        );
    }

    #[test]
    fn only_a_known_time_that_differs_or_another_size_is_a_change() {
        let recorded = FileFingerprint {
            size_bytes: 5,
            modified_at_utc_ms: Some(1),
            created_at_utc_ms: None,
        };
        assert!(!file_differs(&recorded, 5, Some(1)));
        assert!(!file_differs(&recorded, 5, None));
        assert!(file_differs(&recorded, 5, Some(2)));
        assert!(file_differs(&recorded, 6, Some(1)));
        let unknown = FileFingerprint {
            size_bytes: 5,
            modified_at_utc_ms: None,
            created_at_utc_ms: None,
        };
        assert!(!file_differs(&unknown, 5, Some(9)));
    }
}
