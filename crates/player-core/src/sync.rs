use std::{collections::HashSet, error::Error};

use crate::library::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaSourceError, MediaTrackRecord, SourceId,
    SourceScanState, TrackIdentity, TrackMetadataError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackSyncState {
    pub fingerprint: FileFingerprint,
    pub metadata_loaded: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SyncApplyStats {
    pub inserted_or_updated: u64,
    pub removed_source_mappings: u64,
}

/// Database-facing contract. Implementations must apply updates and complete-scan reconciliation
/// in one transaction, and must treat non-complete scan states as ineligible for reconciliation.
pub trait LibraryRepository {
    type Error: Error + Send + Sync + 'static;

    fn track_sync_state(
        &self,
        identity: &TrackIdentity,
    ) -> Result<Option<TrackSyncState>, Self::Error>;

    fn apply_source_scan(
        &mut self,
        root: &LibraryRoot,
        state: &SourceScanState,
        observed: &[MediaTrackRecord],
        changed: &[MediaTrackRecord],
        errors: &[MediaSourceError],
        synced_at_utc_ms: i64,
    ) -> Result<SyncApplyStats, Self::Error>;
}

#[derive(Debug)]
pub enum SyncError<E> {
    SourceMismatch {
        expected: SourceId,
        actual: SourceId,
    },
    Repository(E),
}

impl<E: std::fmt::Display> std::fmt::Display for SyncError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceMismatch { expected, actual } => write!(
                f,
                "source adapter returned source {actual} while scanning configured source {expected}"
            ),
            Self::Repository(error) => write!(f, "library database operation failed: {error}"),
        }
    }
}

impl<E: Error + 'static> Error for SyncError<E> {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SyncReport {
    pub observed: u64,
    pub metadata_reads: u64,
    pub unchanged: u64,
    pub metadata_errors: Vec<(String, TrackMetadataError)>,
    pub source_errors: Vec<MediaSourceError>,
    pub applied: SyncApplyStats,
    pub state: Option<SourceScanState>,
}

pub struct SyncEngine;

impl SyncEngine {
    /// Reconcile one root. Metadata is read only for new/changed files or records whose metadata
    /// was not successfully loaded before. Per-file metadata errors do not abort the batch.
    pub fn sync<I, R>(
        root: &LibraryRoot,
        index: &mut I,
        repository: &mut R,
        synced_at_utc_ms: i64,
    ) -> Result<SyncReport, SyncError<R::Error>>
    where
        I: MediaIndex + ?Sized,
        R: LibraryRepository,
    {
        let mut scan = index.scan(root);
        if scan.source_id != root.id {
            return Err(SyncError::SourceMismatch {
                expected: root.id,
                actual: scan.source_id,
            });
        }

        let mut effective_state = scan.state.clone();
        let mut report = SyncReport {
            state: Some(effective_state.clone()),
            source_errors: std::mem::take(&mut scan.errors),
            ..SyncReport::default()
        };
        let mut seen = Vec::with_capacity(scan.tracks.len());
        let mut changed = Vec::new();
        let mut seen_item_ids = HashSet::new();

        for mut track in scan.tracks {
            if track.identity.source_id != root.id {
                report.source_errors.push(MediaSourceError {
                    source_item_id: Some(track.identity.source_item_id.clone()),
                    message: "track identity does not match configured library root".to_owned(),
                });
                effective_state = SourceScanState::Incomplete {
                    reason: "adapter returned an item belonging to a different source".to_owned(),
                };
                continue;
            }
            if !seen_item_ids.insert(track.identity.source_item_id.clone()) {
                report.source_errors.push(MediaSourceError {
                    source_item_id: Some(track.identity.source_item_id.clone()),
                    message: "adapter returned the same source item more than once".to_owned(),
                });
                effective_state = SourceScanState::Incomplete {
                    reason: "adapter returned duplicate source items".to_owned(),
                };
                continue;
            }
            report.observed += 1;
            let existing = repository
                .track_sync_state(&track.identity)
                .map_err(SyncError::Repository)?;
            let unchanged = existing.is_some_and(|state| state.fingerprint == track.fingerprint);
            if unchanged && existing.is_some_and(|state| state.metadata_loaded) {
                report.unchanged += 1;
                seen.push(track);
                continue;
            }

            if track.metadata.is_none() {
                report.metadata_reads += 1;
                match index.read_metadata(&track) {
                    Ok(metadata) => track.metadata = Some(metadata),
                    Err(error) => report
                        .metadata_errors
                        .push((track.identity.source_item_id.clone(), error)),
                }
            }
            seen.push(track.clone());
            changed.push(track);
        }

        let mut apply_errors = report.source_errors.clone();
        apply_errors.extend(
            report
                .metadata_errors
                .iter()
                .map(|(item, error)| MediaSourceError {
                    source_item_id: Some(item.clone()),
                    message: error.message.clone(),
                }),
        );
        report.state = Some(effective_state.clone());
        report.applied = repository
            .apply_source_scan(
                root,
                &effective_state,
                &seen,
                &changed,
                &apply_errors,
                synced_at_utc_ms,
            )
            .map_err(SyncError::Repository)?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::{LibraryRepository, SyncApplyStats, SyncEngine, SyncReport, TrackSyncState};
    use crate::library::{
        FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaSourceError, MediaSourceKind,
        MediaTrackRecord, SourceId, SourceScan, SourceScanState, TrackIdentity, TrackMetadata,
        TrackMetadataError,
    };

    #[derive(Default)]
    struct FakeRepository {
        entries: HashMap<String, TrackSyncState>,
        applied_state: Option<SourceScanState>,
        observed: Vec<String>,
        changed: Vec<String>,
        errors: Vec<MediaSourceError>,
    }

    impl LibraryRepository for FakeRepository {
        type Error = std::io::Error;

        fn track_sync_state(
            &self,
            identity: &TrackIdentity,
        ) -> Result<Option<TrackSyncState>, Self::Error> {
            Ok(self.entries.get(&identity.source_item_id).copied())
        }

        fn apply_source_scan(
            &mut self,
            _root: &LibraryRoot,
            state: &SourceScanState,
            observed: &[MediaTrackRecord],
            changed: &[MediaTrackRecord],
            errors: &[MediaSourceError],
            _synced_at_utc_ms: i64,
        ) -> Result<SyncApplyStats, Self::Error> {
            self.applied_state = Some(state.clone());
            self.observed = observed
                .iter()
                .map(|track| track.identity.source_item_id.clone())
                .collect();
            self.changed = changed
                .iter()
                .map(|track| track.identity.source_item_id.clone())
                .collect();
            self.errors = errors.to_vec();
            Ok(SyncApplyStats {
                inserted_or_updated: changed.len() as u64,
                removed_source_mappings: 0,
            })
        }
    }

    struct FakeIndex {
        scan: Option<SourceScan>,
        reads: Vec<String>,
        failures: Vec<String>,
    }

    impl MediaIndex for FakeIndex {
        fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
            self.scan.take().expect("one scan result")
        }

        fn read_metadata(
            &mut self,
            track: &MediaTrackRecord,
        ) -> Result<TrackMetadata, TrackMetadataError> {
            self.reads.push(track.identity.source_item_id.clone());
            if self.failures.contains(&track.identity.source_item_id) {
                return Err(TrackMetadataError {
                    message: "bad tag block".to_owned(),
                });
            }
            Ok(TrackMetadata {
                title: Some(format!("title:{}", track.identity.source_item_id)),
                ..TrackMetadata::default()
            })
        }
    }

    fn root() -> LibraryRoot {
        LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "music".to_owned(),
            locator: MediaLocator::FileSystem(PathBuf::from(r"C:\Music")),
            enabled: true,
        }
    }

    fn record(
        source_id: SourceId,
        item: &str,
        size: u64,
        metadata: Option<TrackMetadata>,
    ) -> MediaTrackRecord {
        MediaTrackRecord {
            identity: TrackIdentity {
                source_id,
                source_item_id: item.to_owned(),
                locator_key: Some(format!("win:{item}")),
            },
            locator: MediaLocator::FileSystem(PathBuf::from(format!(r"C:\Music\{item}.flac"))),
            fingerprint: FileFingerprint {
                size_bytes: size,
                modified_at_utc_ms: Some(1_800_000_000_000),
            },
            metadata,
        }
    }

    fn index(
        root: &LibraryRoot,
        tracks: Vec<MediaTrackRecord>,
        state: SourceScanState,
    ) -> FakeIndex {
        FakeIndex {
            scan: Some(SourceScan {
                source_id: root.id,
                state,
                tracks,
                errors: Vec::new(),
            }),
            reads: Vec::new(),
            failures: Vec::new(),
        }
    }

    #[test]
    fn unchanged_fingerprint_skips_metadata_read() {
        let root = root();
        let record = record(root.id, "same", 10, None);
        let mut index = index(&root, vec![record.clone()], SourceScanState::Complete);
        let mut repository = FakeRepository::default();
        repository.entries.insert(
            "same".to_owned(),
            TrackSyncState {
                fingerprint: record.fingerprint,
                metadata_loaded: true,
            },
        );

        let report = SyncEngine::sync(&root, &mut index, &mut repository, 1_800_000_000_100)
            .expect("sync succeeds");

        assert_eq!(report.unchanged, 1);
        assert_eq!(report.metadata_reads, 0);
        assert!(index.reads.is_empty());
        assert_eq!(repository.observed, ["same"]);
        assert!(repository.changed.is_empty());
    }

    #[test]
    fn sync_accepts_media_index_trait_object() {
        let root = root();
        let mut concrete_index = index(&root, Vec::new(), SourceScanState::Complete);
        let index: &mut dyn MediaIndex = &mut concrete_index;
        let mut repository = FakeRepository::default();

        let report = SyncEngine::sync(&root, index, &mut repository, 1_800_000_000_100)
            .expect("trait object adapter sync succeeds");

        assert_eq!(report.state, Some(SourceScanState::Complete));
        assert_eq!(repository.applied_state, Some(SourceScanState::Complete));
    }

    #[test]
    fn changed_files_are_parsed_and_a_bad_file_does_not_stop_the_batch() {
        let root = root();
        let mut index = index(
            &root,
            vec![
                record(root.id, "broken", 10, None),
                record(root.id, "good", 20, None),
            ],
            SourceScanState::Complete,
        );
        index.failures.push("broken".to_owned());
        let mut repository = FakeRepository::default();

        let report = SyncEngine::sync(&root, &mut index, &mut repository, 1_800_000_000_100)
            .expect("single file failure does not abort sync");

        assert_eq!(report.metadata_reads, 2);
        assert_eq!(report.metadata_errors.len(), 1);
        assert_eq!(index.reads, ["broken", "good"]);
        assert_eq!(repository.observed, ["broken", "good"]);
        assert_eq!(repository.changed, ["broken", "good"]);
        assert_eq!(repository.errors.len(), 1);
        assert_eq!(report.state, Some(SourceScanState::Complete));
    }

    #[test]
    fn interrupted_scan_is_forwarded_as_non_reconcilable() {
        let root = root();
        let mut index = index(
            &root,
            Vec::new(),
            SourceScanState::Unavailable {
                reason: "NAS unreachable".to_owned(),
            },
        );
        let mut repository = FakeRepository::default();

        let SyncReport { state, .. } =
            SyncEngine::sync(&root, &mut index, &mut repository, 1_800_000_000_100)
                .expect("unavailable source is a normal sync outcome");

        assert_eq!(state, repository.applied_state);
        assert!(!state.expect("scan state").allows_reconciliation());
    }

    #[test]
    fn mismatched_track_source_downgrades_complete_scan() {
        let root = root();
        let other = SourceId::new();
        let mut index = index(
            &root,
            vec![record(other, "foreign", 10, None)],
            SourceScanState::Complete,
        );
        let mut repository = FakeRepository::default();

        let report = SyncEngine::sync(&root, &mut index, &mut repository, 1_800_000_000_100)
            .expect("bad item is isolated");

        assert!(matches!(
            report.state,
            Some(SourceScanState::Incomplete { .. })
        ));
        assert!(!repository.applied_state.unwrap().allows_reconciliation());
        assert!(repository.observed.is_empty());
    }
}
