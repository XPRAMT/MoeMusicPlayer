use std::{
    collections::HashSet,
    error::Error,
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::library::{
    FileFingerprint, LibraryRoot, MediaIndex, MediaScanProgress, MediaScanProgressUnit,
    MediaSourceError, MediaTrackRecord, SourceId, SourceScanState, SyncCancellation, TrackIdentity,
    TrackMetadataError, TRACK_METADATA_VERSION,
};

const METADATA_BACKFILL_BATCH_SIZE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TrackSyncState {
    pub fingerprint: FileFingerprint,
    pub metadata_loaded: bool,
    pub metadata_version: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SyncApplyStats {
    pub inserted_or_updated: u64,
    pub removed_source_mappings: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncProgressStage {
    Enumerating,
    Metadata,
    Persisting,
    Finished,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncProgressUnit {
    FilesystemEntries,
    Tracks,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncProgressOutcome {
    Complete,
    Incomplete,
    Unavailable,
    PermissionRevoked,
    Cancelled,
    Failed,
}

/// Per-source progress snapshot. `total` is stage-local and remains `None` while a filesystem
/// traversal has no reliable total; consumers should show its processed count without a percent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub source_id: SourceId,
    pub stage: SyncProgressStage,
    pub processed: u64,
    pub total: Option<u64>,
    pub unit: SyncProgressUnit,
    pub observed: u64,
    pub metadata_reads: u64,
    pub unchanged: u64,
    pub error_count: u64,
    pub outcome: Option<SyncProgressOutcome>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SyncApplyOutcome {
    pub stats: SyncApplyStats,
    pub cancelled: bool,
}

pub struct SyncApplyRequest<'a> {
    pub root: &'a LibraryRoot,
    pub state: &'a SourceScanState,
    pub observed: &'a [MediaTrackRecord],
    pub changed: &'a [MediaTrackRecord],
    pub errors: &'a [MediaSourceError],
    pub synced_at_utc_ms: i64,
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

    /// Commit successfully parsed legacy metadata in small durable batches. This is separate
    /// from complete-scan reconciliation so cancellation/restart keeps completed batches.
    fn apply_metadata_backfill_batch(
        &mut self,
        tracks: &[MediaTrackRecord],
    ) -> Result<(), Self::Error>;

    /// Apply one source result atomically where possible. A repository may report progress while
    /// it persists records. If it cannot cancel inside its transaction, it should finish that
    /// transaction and report `cancelled: false` once committed.
    fn apply_source_scan_with_progress(
        &mut self,
        request: SyncApplyRequest<'_>,
        progress: &mut dyn FnMut(u64),
        cancellation: &SyncCancellation,
    ) -> Result<SyncApplyOutcome, Self::Error> {
        if cancellation.is_cancelled() {
            return Ok(SyncApplyOutcome {
                cancelled: true,
                ..SyncApplyOutcome::default()
            });
        }

        let stats = self.apply_source_scan(
            request.root,
            request.state,
            request.observed,
            request.changed,
            request.errors,
            request.synced_at_utc_ms,
        )?;
        progress(request.observed.len() as u64);
        Ok(SyncApplyOutcome {
            stats,
            cancelled: false,
        })
    }
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
    pub cancelled: bool,
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
        Self::sync_with_progress(root, index, repository, synced_at_utc_ms, |_| {})
    }

    /// Reconcile one root and emit throttled, stage-specific progress snapshots.
    pub fn sync_with_progress<I, R, F>(
        root: &LibraryRoot,
        index: &mut I,
        repository: &mut R,
        synced_at_utc_ms: i64,
        progress: F,
    ) -> Result<SyncReport, SyncError<R::Error>>
    where
        I: MediaIndex + ?Sized,
        R: LibraryRepository,
        F: FnMut(SyncProgress),
    {
        Self::sync_cancellable_with_progress(
            root,
            index,
            repository,
            synced_at_utc_ms,
            &SyncCancellation::default(),
            progress,
        )
    }

    /// Cancellable form used by hosts that expose a cancel action. Cancellation is cooperative:
    /// scanners should check between entries, metadata parsing is checked between tracks, and the
    /// repository may roll back a cancellable persistence transaction. No incomplete seen-set is
    /// ever passed to complete-scan reconciliation.
    pub fn sync_cancellable_with_progress<I, R, F>(
        root: &LibraryRoot,
        index: &mut I,
        repository: &mut R,
        synced_at_utc_ms: i64,
        cancellation: &SyncCancellation,
        mut progress: F,
    ) -> Result<SyncReport, SyncError<R::Error>>
    where
        I: MediaIndex + ?Sized,
        R: LibraryRepository,
        F: FnMut(SyncProgress),
    {
        let mut reporter = ProgressReporter::new(root.id, &mut progress);
        reporter.emit(
            SyncProgressStage::Enumerating,
            0,
            None,
            SyncProgressUnit::FilesystemEntries,
            0,
            0,
            0,
            0,
            None,
            true,
        );

        let result = Self::sync_inner(
            root,
            index,
            repository,
            synced_at_utc_ms,
            cancellation,
            &mut reporter,
        );

        match &result {
            Ok(report) => reporter.emit(
                SyncProgressStage::Finished,
                report.observed,
                Some(report.observed),
                SyncProgressUnit::Tracks,
                report.observed,
                report.metadata_reads,
                report.unchanged,
                sync_error_count(report),
                Some(progress_outcome(report)),
                true,
            ),
            Err(_) => reporter.emit_failure(),
        }
        result
    }

    fn sync_inner<I, R>(
        root: &LibraryRoot,
        index: &mut I,
        repository: &mut R,
        synced_at_utc_ms: i64,
        cancellation: &SyncCancellation,
        progress: &mut ProgressReporter<'_>,
    ) -> Result<SyncReport, SyncError<R::Error>>
    where
        I: MediaIndex + ?Sized,
        R: LibraryRepository,
    {
        let mut scan = index.scan_with_progress(
            root,
            &mut |update: MediaScanProgress| {
                progress.emit(
                    SyncProgressStage::Enumerating,
                    update.processed,
                    update.total,
                    match update.unit {
                        MediaScanProgressUnit::FilesystemEntries => {
                            SyncProgressUnit::FilesystemEntries
                        }
                        MediaScanProgressUnit::Tracks => SyncProgressUnit::Tracks,
                    },
                    0,
                    0,
                    0,
                    0,
                    None,
                    false,
                );
            },
            cancellation,
        );
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
        let mut metadata_backfill_batch = Vec::with_capacity(METADATA_BACKFILL_BATCH_SIZE);
        let mut seen_item_ids = HashSet::new();
        let metadata_total = scan.tracks.len() as u64;
        progress.emit(
            SyncProgressStage::Metadata,
            0,
            Some(metadata_total),
            SyncProgressUnit::Tracks,
            report.observed,
            report.metadata_reads,
            report.unchanged,
            sync_error_count(&report),
            None,
            true,
        );

        let mut metadata_processed = 0;
        for mut track in scan.tracks {
            if cancellation.is_cancelled() {
                report.cancelled = true;
                effective_state = SourceScanState::Incomplete {
                    reason: "sync cancelled before metadata processing completed".to_owned(),
                };
                break;
            }
            metadata_processed += 1;
            if track.identity.source_id != root.id {
                report.source_errors.push(MediaSourceError {
                    source_item_id: Some(track.identity.source_item_id.clone()),
                    message: "track identity does not match configured library root".to_owned(),
                });
                effective_state = SourceScanState::Incomplete {
                    reason: "adapter returned an item belonging to a different source".to_owned(),
                };
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
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
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
                continue;
            }
            report.observed += 1;
            let existing = repository
                .track_sync_state(&track.identity)
                .map_err(SyncError::Repository)?;
            let unchanged = existing.is_some_and(|state| state.fingerprint == track.fingerprint);
            let legacy_metadata = unchanged
                && existing.is_some_and(|state| {
                    state.metadata_loaded && state.metadata_version < TRACK_METADATA_VERSION
                });
            if unchanged
                && existing.is_some_and(|state| {
                    state.metadata_loaded && state.metadata_version >= TRACK_METADATA_VERSION
                })
            {
                report.unchanged += 1;
                seen.push(track);
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
                continue;
            }

            if legacy_metadata {
                let metadata_result = match track.metadata.take() {
                    Some(metadata) => Ok(metadata),
                    None => {
                        report.metadata_reads += 1;
                        index.read_metadata(&track)
                    }
                };
                match metadata_result {
                    Ok(metadata) => {
                        track.metadata = Some(metadata);
                        metadata_backfill_batch.push(track.clone());
                        if metadata_backfill_batch.len() == METADATA_BACKFILL_BATCH_SIZE {
                            repository
                                .apply_metadata_backfill_batch(&metadata_backfill_batch)
                                .map_err(SyncError::Repository)?;
                            metadata_backfill_batch.clear();
                            std::thread::yield_now();
                        }
                    }
                    Err(error) => report
                        .metadata_errors
                        .push((track.identity.source_item_id.clone(), error)),
                }
                seen.push(track);
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
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
            emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
        }

        if !metadata_backfill_batch.is_empty() {
            repository
                .apply_metadata_backfill_batch(&metadata_backfill_batch)
                .map_err(SyncError::Repository)?;
            std::thread::yield_now();
        }

        if cancellation.is_cancelled() {
            report.cancelled = true;
            effective_state = SourceScanState::Incomplete {
                reason: "sync cancelled before persistence".to_owned(),
            };
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
        if report.cancelled {
            return Ok(report);
        }

        progress.emit(
            SyncProgressStage::Persisting,
            0,
            Some(seen.len() as u64),
            SyncProgressUnit::Tracks,
            report.observed,
            report.metadata_reads,
            report.unchanged,
            sync_error_count(&report),
            None,
            true,
        );
        let apply = repository
            .apply_source_scan_with_progress(
                SyncApplyRequest {
                    root,
                    state: &effective_state,
                    observed: &seen,
                    changed: &changed,
                    errors: &apply_errors,
                    synced_at_utc_ms,
                },
                &mut |processed| {
                    progress.emit(
                        SyncProgressStage::Persisting,
                        processed,
                        Some(seen.len() as u64),
                        SyncProgressUnit::Tracks,
                        report.observed,
                        report.metadata_reads,
                        report.unchanged,
                        sync_error_count(&report),
                        None,
                        false,
                    );
                },
                cancellation,
            )
            .map_err(SyncError::Repository)?;
        report.applied = apply.stats;
        report.cancelled = apply.cancelled;
        if report.cancelled {
            report.state = Some(SourceScanState::Incomplete {
                reason: "sync cancelled during database persistence".to_owned(),
            });
        }
        Ok(report)
    }
}

struct ProgressReporter<'a> {
    source_id: SourceId,
    callback: &'a mut dyn FnMut(SyncProgress),
    last_stage: Option<SyncProgressStage>,
    last_processed: u64,
    last_emitted_at: Option<Instant>,
    last_snapshot: Option<SyncProgress>,
}

impl<'a> ProgressReporter<'a> {
    fn new(source_id: SourceId, callback: &'a mut dyn FnMut(SyncProgress)) -> Self {
        Self {
            source_id,
            callback,
            last_stage: None,
            last_processed: 0,
            last_emitted_at: None,
            last_snapshot: None,
        }
    }

    fn emit_failure(&mut self) {
        let previous = self.last_snapshot.clone();
        let (processed, total, unit, observed, metadata_reads, unchanged, error_count) = previous
            .map(|snapshot| {
                (
                    snapshot.processed,
                    snapshot.total,
                    snapshot.unit,
                    snapshot.observed,
                    snapshot.metadata_reads,
                    snapshot.unchanged,
                    snapshot.error_count.saturating_add(1),
                )
            })
            .unwrap_or((0, None, SyncProgressUnit::Tracks, 0, 0, 0, 1));
        self.emit(
            SyncProgressStage::Finished,
            processed,
            total,
            unit,
            observed,
            metadata_reads,
            unchanged,
            error_count,
            Some(SyncProgressOutcome::Failed),
            true,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &mut self,
        stage: SyncProgressStage,
        processed: u64,
        total: Option<u64>,
        unit: SyncProgressUnit,
        observed: u64,
        metadata_reads: u64,
        unchanged: u64,
        error_count: u64,
        outcome: Option<SyncProgressOutcome>,
        force: bool,
    ) {
        let now = Instant::now();
        let stage_changed = self.last_stage != Some(stage);
        let elapsed = self
            .last_emitted_at
            .map(|last| now.duration_since(last))
            .unwrap_or(Duration::MAX);
        let advanced = processed > self.last_processed;
        let first_progress = stage == SyncProgressStage::Enumerating
            && self.last_stage == Some(stage)
            && self.last_processed == 0
            && processed >= 64;
        let time_throttled = elapsed >= Duration::from_millis(200)
            || (processed.saturating_sub(self.last_processed) >= 2_048
                && elapsed >= Duration::from_millis(100));
        if !(force || stage_changed || first_progress || (advanced && time_throttled)) {
            return;
        }

        let snapshot = SyncProgress {
            source_id: self.source_id,
            stage,
            processed,
            total,
            unit,
            observed,
            metadata_reads,
            unchanged,
            error_count,
            outcome,
        };
        (self.callback)(snapshot.clone());
        self.last_snapshot = Some(snapshot);
        self.last_stage = Some(stage);
        self.last_processed = processed;
        self.last_emitted_at = Some(now);
    }
}

fn emit_metadata_progress(
    reporter: &mut ProgressReporter<'_>,
    processed: u64,
    total: u64,
    report: &SyncReport,
) {
    reporter.emit(
        SyncProgressStage::Metadata,
        processed,
        Some(total),
        SyncProgressUnit::Tracks,
        report.observed,
        report.metadata_reads,
        report.unchanged,
        sync_error_count(report),
        None,
        false,
    );
}

fn sync_error_count(report: &SyncReport) -> u64 {
    (report.source_errors.len() + report.metadata_errors.len()) as u64
}

fn progress_outcome(report: &SyncReport) -> SyncProgressOutcome {
    if report.cancelled {
        return SyncProgressOutcome::Cancelled;
    }
    match report.state.as_ref() {
        Some(SourceScanState::Complete) => SyncProgressOutcome::Complete,
        Some(SourceScanState::Incomplete { .. }) => SyncProgressOutcome::Incomplete,
        Some(SourceScanState::Unavailable { .. }) => SyncProgressOutcome::Unavailable,
        Some(SourceScanState::PermissionRevoked { .. }) => SyncProgressOutcome::PermissionRevoked,
        None => SyncProgressOutcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };

    use super::{
        LibraryRepository, SyncApplyStats, SyncEngine, SyncProgressOutcome, SyncProgressStage,
        SyncReport, TrackSyncState,
    };
    use crate::library::{
        FileFingerprint, LibraryRoot, MediaIndex, MediaLocator, MediaScanProgress,
        MediaScanProgressUnit, MediaSourceError, MediaSourceKind, MediaTrackRecord, SourceId,
        SourceScan, SourceScanState, SyncCancellation, TrackIdentity, TrackMetadata,
        TrackMetadataError, TRACK_METADATA_VERSION,
    };

    #[derive(Default)]
    struct FakeRepository {
        entries: HashMap<String, TrackSyncState>,
        applied_state: Option<SourceScanState>,
        observed: Vec<String>,
        changed: Vec<String>,
        errors: Vec<MediaSourceError>,
        backfill_batches: Vec<usize>,
        fail_apply: bool,
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
            if self.fail_apply {
                return Err(std::io::Error::other("test database failure"));
            }
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

        fn apply_metadata_backfill_batch(
            &mut self,
            tracks: &[MediaTrackRecord],
        ) -> Result<(), Self::Error> {
            if self.fail_apply {
                return Err(std::io::Error::other("test backfill failure"));
            }
            self.backfill_batches.push(tracks.len());
            for track in tracks {
                let state = self
                    .entries
                    .get_mut(&track.identity.source_item_id)
                    .expect("legacy row exists");
                state.metadata_loaded = true;
                state.metadata_version = TRACK_METADATA_VERSION;
            }
            Ok(())
        }
    }

    struct FakeIndex {
        scan: Option<SourceScan>,
        reads: Vec<String>,
        failures: Vec<String>,
        cancel_after_reads: Option<(usize, SyncCancellation)>,
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
            if self
                .cancel_after_reads
                .as_ref()
                .is_some_and(|(count, _)| self.reads.len() == *count)
            {
                self.cancel_after_reads
                    .as_ref()
                    .expect("cancellation configured")
                    .1
                    .cancel();
            }
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

    struct StreamingIndex {
        scan: Option<SourceScan>,
        core_callback_seen: Arc<AtomicBool>,
    }

    impl MediaIndex for StreamingIndex {
        fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
            self.scan.take().expect("one scan result")
        }

        fn scan_with_progress(
            &mut self,
            root: &LibraryRoot,
            progress: &mut dyn FnMut(MediaScanProgress),
            _cancellation: &SyncCancellation,
        ) -> SourceScan {
            progress(MediaScanProgress {
                processed: 64,
                total: None,
                unit: MediaScanProgressUnit::FilesystemEntries,
            });
            assert!(
                self.core_callback_seen.load(Ordering::SeqCst),
                "core callback must run before enumeration returns"
            );
            self.scan(root)
        }

        fn read_metadata(
            &mut self,
            _track: &MediaTrackRecord,
        ) -> Result<TrackMetadata, TrackMetadataError> {
            Ok(TrackMetadata::default())
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
            cancel_after_reads: None,
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
                metadata_version: TRACK_METADATA_VERSION,
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
    fn metadata_backfill_commits_bounded_batches_and_resumes_after_cancellation() {
        let root = root();
        let records = (0..300)
            .map(|index| record(root.id, &format!("legacy-{index:03}"), 10, None))
            .collect::<Vec<_>>();
        let mut repository = FakeRepository::default();
        for track in &records {
            repository.entries.insert(
                track.identity.source_item_id.clone(),
                TrackSyncState {
                    fingerprint: track.fingerprint,
                    metadata_loaded: true,
                    metadata_version: TRACK_METADATA_VERSION - 1,
                },
            );
        }

        let cancellation = SyncCancellation::default();
        let mut first_index = index(&root, records.clone(), SourceScanState::Complete);
        first_index.cancel_after_reads = Some((128, cancellation.clone()));
        let first = SyncEngine::sync_cancellable_with_progress(
            &root,
            &mut first_index,
            &mut repository,
            1_800_000_000_100,
            &cancellation,
            |_| {},
        )
        .expect("cancelled backfill is reported");
        assert!(first.cancelled);
        assert_eq!(first_index.reads.len(), 128);
        assert_eq!(repository.backfill_batches, [128]);
        assert_eq!(
            repository
                .entries
                .values()
                .filter(|state| state.metadata_version == TRACK_METADATA_VERSION)
                .count(),
            128
        );

        let mut second_index = index(&root, records, SourceScanState::Complete);
        let second = SyncEngine::sync(&root, &mut second_index, &mut repository, 1_800_000_000_200)
            .expect("resume stale metadata after restart");
        assert!(!second.cancelled);
        assert_eq!(second_index.reads.len(), 172);
        assert_eq!(repository.backfill_batches, [128, 128, 44]);
        assert!(repository
            .entries
            .values()
            .all(|state| state.metadata_version == TRACK_METADATA_VERSION));
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
    fn progress_reports_each_sync_stage_and_terminal_outcome() {
        let root = root();
        let mut index = index(
            &root,
            vec![
                record(root.id, "one", 10, None),
                record(root.id, "two", 20, None),
            ],
            SourceScanState::Complete,
        );
        let mut repository = FakeRepository::default();
        let mut progress = Vec::new();

        let report = SyncEngine::sync_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            |snapshot| progress.push(snapshot),
        )
        .expect("sync succeeds");

        assert_eq!(report.observed, 2);
        assert_eq!(
            progress.first().map(|snapshot| snapshot.stage),
            Some(SyncProgressStage::Enumerating)
        );
        assert!(progress.iter().any(|snapshot| {
            snapshot.stage == SyncProgressStage::Metadata && snapshot.total == Some(2)
        }));
        assert!(progress.iter().any(|snapshot| {
            snapshot.stage == SyncProgressStage::Persisting && snapshot.total == Some(2)
        }));
        let finished = progress.last().expect("terminal progress event");
        assert_eq!(finished.stage, SyncProgressStage::Finished);
        assert_eq!(finished.outcome, Some(SyncProgressOutcome::Complete));
        assert_eq!(finished.observed, 2);
        assert!(progress
            .iter()
            .all(|snapshot| snapshot.source_id == root.id));
    }

    #[test]
    fn progress_events_remain_scoped_to_their_source_across_sequential_syncs() {
        let first_root = root();
        let second_root = root();
        assert_ne!(first_root.id, second_root.id);
        let mut first_index = index(
            &first_root,
            vec![record(first_root.id, "first", 10, None)],
            SourceScanState::Complete,
        );
        let mut second_index = index(
            &second_root,
            vec![record(second_root.id, "second", 20, None)],
            SourceScanState::Complete,
        );
        let mut first_repository = FakeRepository::default();
        let mut second_repository = FakeRepository::default();
        let mut progress = Vec::new();

        SyncEngine::sync_with_progress(
            &first_root,
            &mut first_index,
            &mut first_repository,
            1_800_000_000_100,
            |snapshot| progress.push(snapshot),
        )
        .expect("first root sync succeeds");
        SyncEngine::sync_with_progress(
            &second_root,
            &mut second_index,
            &mut second_repository,
            1_800_000_000_200,
            |snapshot| progress.push(snapshot),
        )
        .expect("second root sync succeeds");

        let first_events = progress
            .iter()
            .filter(|snapshot| snapshot.source_id == first_root.id)
            .collect::<Vec<_>>();
        let second_events = progress
            .iter()
            .filter(|snapshot| snapshot.source_id == second_root.id)
            .collect::<Vec<_>>();
        assert!(!first_events.is_empty());
        assert!(!second_events.is_empty());
        assert_eq!(
            first_events.last().unwrap().stage,
            SyncProgressStage::Finished
        );
        assert_eq!(
            second_events.last().unwrap().stage,
            SyncProgressStage::Finished
        );
        assert!(progress.iter().all(|snapshot| {
            snapshot.source_id == first_root.id || snapshot.source_id == second_root.id
        }));
    }

    #[test]
    fn core_forwards_enumeration_progress_before_scanner_returns() {
        let root = root();
        let scan = SourceScan {
            source_id: root.id,
            state: SourceScanState::Complete,
            tracks: Vec::new(),
            errors: Vec::new(),
        };
        let callback_seen = Arc::new(AtomicBool::new(false));
        let mut index = StreamingIndex {
            scan: Some(scan),
            core_callback_seen: Arc::clone(&callback_seen),
        };
        let mut repository = FakeRepository::default();
        let mut progress = Vec::new();

        SyncEngine::sync_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            |snapshot| {
                if snapshot.stage == SyncProgressStage::Enumerating && snapshot.processed >= 64 {
                    callback_seen.store(true, Ordering::SeqCst);
                }
                progress.push(snapshot);
            },
        )
        .expect("sync with a streaming scanner succeeds");

        assert!(progress.iter().any(|snapshot| {
            snapshot.stage == SyncProgressStage::Enumerating
                && snapshot.processed == 64
                && snapshot.total.is_none()
        }));
    }

    #[test]
    fn pre_cancelled_sync_does_not_scan_or_apply_an_empty_seen_set() {
        let root = root();
        let mut index = index(&root, Vec::new(), SourceScanState::Complete);
        let mut repository = FakeRepository::default();
        let cancellation = SyncCancellation::default();
        cancellation.cancel();
        let mut progress = Vec::new();

        let report = SyncEngine::sync_cancellable_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            &cancellation,
            |snapshot| progress.push(snapshot),
        )
        .expect("cancelled sync is reported, not failed");

        assert!(report.cancelled);
        assert!(index.scan.is_some(), "scanner was not called");
        assert!(
            repository.applied_state.is_none(),
            "cancelled scan was not applied"
        );
        assert_eq!(
            progress.last().and_then(|snapshot| snapshot.outcome),
            Some(SyncProgressOutcome::Cancelled)
        );
    }

    #[test]
    fn cancellation_requested_during_metadata_stops_before_database_apply() {
        let root = root();
        let mut index = index(
            &root,
            vec![record(root.id, "one", 10, None)],
            SourceScanState::Complete,
        );
        let mut repository = FakeRepository::default();
        let cancellation = SyncCancellation::default();
        let cancellation_from_callback = cancellation.clone();
        let mut progress = Vec::new();

        let report = SyncEngine::sync_cancellable_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            &cancellation,
            |snapshot| {
                if snapshot.stage == SyncProgressStage::Metadata {
                    cancellation_from_callback.cancel();
                }
                progress.push(snapshot);
            },
        )
        .expect("cancellation is not a failed sync");

        assert!(report.cancelled);
        assert!(repository.applied_state.is_none());
        assert!(index.reads.is_empty());
        assert_eq!(
            progress.last().and_then(|snapshot| snapshot.outcome),
            Some(SyncProgressOutcome::Cancelled)
        );
    }

    #[test]
    fn unavailable_source_has_a_distinct_terminal_outcome() {
        let root = root();
        let mut index = index(
            &root,
            Vec::new(),
            SourceScanState::Unavailable {
                reason: "root is offline".to_owned(),
            },
        );
        let mut repository = FakeRepository::default();
        let mut progress = Vec::new();

        let report = SyncEngine::sync_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            |snapshot| progress.push(snapshot),
        )
        .expect("unavailable source is represented as a report");

        assert_eq!(
            report.state,
            Some(SourceScanState::Unavailable {
                reason: "root is offline".to_owned(),
            })
        );
        assert_eq!(
            progress.last().and_then(|snapshot| snapshot.outcome),
            Some(SyncProgressOutcome::Unavailable)
        );
        assert_eq!(repository.applied_state, report.state);
    }

    #[test]
    fn repository_failure_emits_a_failed_terminal_snapshot_with_work_counts() {
        let root = root();
        let mut index = index(
            &root,
            vec![record(root.id, "one", 10, None)],
            SourceScanState::Complete,
        );
        let mut repository = FakeRepository {
            fail_apply: true,
            ..FakeRepository::default()
        };
        let mut progress = Vec::new();

        let result = SyncEngine::sync_with_progress(
            &root,
            &mut index,
            &mut repository,
            1_800_000_000_100,
            |snapshot| progress.push(snapshot),
        );

        assert!(result.is_err());
        let finished = progress.last().expect("terminal progress event");
        assert_eq!(finished.stage, SyncProgressStage::Finished);
        assert_eq!(finished.outcome, Some(SyncProgressOutcome::Failed));
        assert_eq!(finished.metadata_reads, 1);
        assert_eq!(finished.error_count, 1);
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
