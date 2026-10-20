use std::{
    collections::{HashMap, HashSet},
    error::Error,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use serde::Serialize;

use crate::library::{
    read_lyric_sidecar, FileFingerprint, LibraryRoot, LyricSidecar, LyricSidecarRead,
    LyricSidecarState, LyricSidecarWrite, MediaIndex, MediaScanProgress, MediaScanProgressUnit,
    MediaSourceError, MediaTrackRecord, MetadataReader, SourceId, SourceScanState,
    SyncCancellation, TrackIdentity, TrackMetadata, TrackMetadataError, TRACK_METADATA_VERSION,
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

    /// The fingerprints the database holds for the `.lrc` sidecars of a source, by the audio file's `source_item_id`.
    fn lyric_sidecar_states(
        &self,
        _source_id: SourceId,
    ) -> Result<HashMap<String, LyricSidecarState>, Self::Error> {
        Ok(HashMap::new())
    }

    /// Record the sidecars a scan found and mark the source as scanned for lyrics (so a track without a record has no
    /// sidecar). `complete` says the scan saw the whole source: only then are sidecars it did not see forgotten.
    fn save_lyric_sidecars(
        &mut self,
        _source_id: SourceId,
        _writes: &[LyricSidecarWrite],
        _complete: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

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
        let timing = std::time::Instant::now();
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
        eprintln!(
            "[sync] {:?}: listed {} audio files in {} ms",
            root.display_name,
            scan.tracks.len(),
            timing.elapsed().as_millis()
        );
        let lyric_sidecars = index.take_lyric_sidecars();
        let scans_lyric_sidecars = index.scans_lyric_sidecars();
        let lyric_threads = index.metadata_read_threads().max(1);
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

        // Pass 1: decide what every track needs. Only database lookups happen here, no file is parsed.
        let mut entries: Vec<Entry> = Vec::with_capacity(scan.tracks.len());
        let mut metadata_processed: u64 = 0;
        for track in scan.tracks {
            if cancellation.is_cancelled() {
                report.cancelled = true;
                effective_state = SourceScanState::Incomplete {
                    reason: "sync cancelled before metadata processing completed".to_owned(),
                };
                break;
            }
            if track.identity.source_id != root.id {
                report.source_errors.push(MediaSourceError {
                    source_item_id: Some(track.identity.source_item_id.clone()),
                    message: "track identity does not match configured library root".to_owned(),
                });
                effective_state = SourceScanState::Incomplete {
                    reason: "adapter returned an item belonging to a different source".to_owned(),
                };
                metadata_processed += 1;
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
                metadata_processed += 1;
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
                continue;
            }
            report.observed += 1;
            let existing = repository
                .track_sync_state(&track.identity)
                .map_err(SyncError::Repository)?;
            let unchanged = existing.is_some_and(|state| {
                state.fingerprint.size_bytes == track.fingerprint.size_bytes
                    && state.fingerprint.modified_at_utc_ms == track.fingerprint.modified_at_utc_ms
            });
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
                entries.push(Entry {
                    track,
                    kind: EntryKind::Unchanged,
                    needs_read: false,
                    read: None,
                });
                metadata_processed += 1;
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
                continue;
            }
            // Metadata the adapter already supplied needs no parsing.
            let needs_read = track.metadata.is_none();
            if !needs_read {
                metadata_processed += 1;
                emit_metadata_progress(progress, metadata_processed, metadata_total, &report);
            }
            entries.push(Entry {
                track,
                kind: if legacy_metadata {
                    EntryKind::Legacy
                } else {
                    EntryKind::Changed
                },
                needs_read,
                read: None,
            });
        }

        eprintln!(
            "[sync] {:?}: compared with the library at {} ms: {} unchanged, {} to read",
            root.display_name,
            timing.elapsed().as_millis(),
            report.unchanged,
            entries.iter().filter(|entry| entry.needs_read).count()
        );
        // Pass 2: parse the tracks that need it, on METADATA_READ_THREADS threads when the adapter allows it.
        // Cancellation stops the reading; what is already parsed is kept.
        let todo: Vec<usize> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.needs_read)
            .map(|(index, _)| index)
            .collect();
        let mut results: Vec<Option<Result<TrackMetadata, TrackMetadataError>>> =
            (0..todo.len()).map(|_| None).collect();
        if !todo.is_empty() && !cancellation.is_cancelled() {
            let tracks: Vec<&MediaTrackRecord> =
                todo.iter().map(|&index| &entries[index].track).collect();
            let reader = index
                .parallel_metadata_reader()
                .filter(|_| tracks.len() > 1);
            match reader {
                Some(reader) => {
                    let next = AtomicUsize::new(0);
                    let (sender, receiver) = std::sync::mpsc::channel();
                    std::thread::scope(|scope| {
                        for _ in 0..index.metadata_read_threads().max(1).min(tracks.len()) {
                            let sender = sender.clone();
                            let (next, tracks) = (&next, &tracks);
                            scope.spawn(move || loop {
                                if cancellation.is_cancelled() {
                                    break;
                                }
                                let n = next.fetch_add(1, Ordering::Relaxed);
                                let Some(track) = tracks.get(n) else {
                                    break;
                                };
                                if sender.send((n, read_guarded(reader, track))).is_err() {
                                    break;
                                }
                            });
                        }
                        drop(sender);
                        for (n, result) in receiver {
                            results[n] = Some(result);
                            report.metadata_reads += 1;
                            metadata_processed += 1;
                            emit_metadata_progress(
                                progress,
                                metadata_processed,
                                metadata_total,
                                &report,
                            );
                        }
                    });
                }
                None => {
                    for (n, track) in tracks.iter().enumerate() {
                        if cancellation.is_cancelled() {
                            break;
                        }
                        results[n] = Some(index.read_metadata(track));
                        report.metadata_reads += 1;
                        metadata_processed += 1;
                        emit_metadata_progress(
                            progress,
                            metadata_processed,
                            metadata_total,
                            &report,
                        );
                    }
                }
            }
        }
        for (slot, &entry_index) in todo.iter().enumerate() {
            entries[entry_index].read = results[slot].take();
        }

        eprintln!(
            "[sync] {:?}: tags read at {} ms ({} files)",
            root.display_name,
            timing.elapsed().as_millis(),
            report.metadata_reads
        );
        // Pass 3: in track order, turn the results into the sets the repository applies. A track whose parsing was
        // stopped by cancellation is left out; it is read again by the next sync.
        for mut entry in entries {
            if entry.needs_read {
                match entry.read.take() {
                    None => continue,
                    Some(Ok(metadata)) => entry.track.metadata = Some(metadata),
                    Some(Err(error)) => report
                        .metadata_errors
                        .push((entry.track.identity.source_item_id.clone(), error)),
                }
            }
            match entry.kind {
                EntryKind::Unchanged => seen.push(entry.track),
                EntryKind::Legacy => {
                    if entry.track.metadata.is_some() {
                        metadata_backfill_batch.push(entry.track.clone());
                        if metadata_backfill_batch.len() == METADATA_BACKFILL_BATCH_SIZE {
                            repository
                                .apply_metadata_backfill_batch(&metadata_backfill_batch)
                                .map_err(SyncError::Repository)?;
                            metadata_backfill_batch.clear();
                            std::thread::yield_now();
                        }
                    }
                    seen.push(entry.track);
                }
                EntryKind::Changed => {
                    seen.push(entry.track.clone());
                    changed.push(entry.track);
                }
            }
        }

        if !metadata_backfill_batch.is_empty() {
            repository
                .apply_metadata_backfill_batch(&metadata_backfill_batch)
                .map_err(SyncError::Repository)?;
            std::thread::yield_now();
        }

        if cancellation.is_cancelled() && !report.cancelled {
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
        // A cancelled run still saves what it read: the tracks handled so far are written with an incomplete scan
        // state (no reconciliation, nothing is removed), so the next sync continues from there instead of reading
        // everything again. With nothing handled there is nothing to save.
        let cancelled = report.cancelled;
        if cancelled && seen.is_empty() {
            return Ok(report);
        }
        let partial_save = SyncCancellation::default();
        let persist_cancellation = if cancelled {
            &partial_save
        } else {
            cancellation
        };

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
                persist_cancellation,
            )
            .map_err(SyncError::Repository)?;
        report.applied = apply.stats;
        eprintln!(
            "[sync] {:?}: stored at {} ms",
            root.display_name,
            timing.elapsed().as_millis()
        );
        report.cancelled = cancelled || apply.cancelled;
        if apply.cancelled {
            report.state = Some(SourceScanState::Incomplete {
                reason: "sync cancelled during database persistence".to_owned(),
            });
        }
        if !report.cancelled
            && scans_lyric_sidecars
            && matches!(
                effective_state,
                SourceScanState::Complete | SourceScanState::Incomplete { .. }
            )
        {
            // The lyrics beside the songs are part of the library: read when new or changed, never searched for at play
            // time. A failure here costs the lyrics, not the sync.
            let _ = sync_lyric_sidecars(
                root,
                repository,
                lyric_sidecars,
                lyric_threads,
                matches!(effective_state, SourceScanState::Complete),
                cancellation,
            );
        }
        Ok(report)
    }
}

/// Bring the stored `.lrc` sidecars of a source up to date with what the scan found: a sidecar whose name, size and
/// modified time are what the database holds is left alone, a new or changed one is read (on several threads: for a remote
/// source each read is a round trip) and stored with its fingerprint.
fn sync_lyric_sidecars<R: LibraryRepository>(
    root: &LibraryRoot,
    repository: &mut R,
    found: Vec<LyricSidecar>,
    threads: usize,
    complete: bool,
    cancellation: &SyncCancellation,
) -> Result<(), R::Error> {
    let stored = repository.lyric_sidecar_states(root.id)?;
    let mut writes: Vec<Option<LyricSidecarWrite>> = (0..found.len()).map(|_| None).collect();
    let mut todo = Vec::new();
    for (index, sidecar) in found.iter().enumerate() {
        let unchanged = stored.get(&sidecar.track_item_id).is_some_and(|state| {
            state.name == sidecar.name
                && state.size_bytes == sidecar.size_bytes
                && state.modified_at_utc_ms == sidecar.modified_at_utc_ms
                && state.has_text
        });
        if unchanged {
            writes[index] = Some(LyricSidecarWrite {
                sidecar: sidecar.clone(),
                text: None,
                oversized: false,
                unchanged: true,
            });
        } else {
            todo.push(index);
        }
    }
    let next = AtomicUsize::new(0);
    let reads: std::sync::Mutex<Vec<(usize, LyricSidecarRead)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads.min(todo.len().max(1)) {
            scope.spawn(|| loop {
                if cancellation.is_cancelled() {
                    break;
                }
                let n = next.fetch_add(1, Ordering::Relaxed);
                let Some(&index) = todo.get(n) else { break };
                let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    read_lyric_sidecar(&found[index])
                }))
                .unwrap_or(LyricSidecarRead::Unreadable);
                reads
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((index, read));
            });
        }
    });
    if cancellation.is_cancelled() {
        return Ok(());
    }
    for (index, read) in reads.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
        let (text, oversized) = match read {
            LyricSidecarRead::Text(text) => (Some(text), false),
            LyricSidecarRead::Oversized => (None, true),
            // Remembered without text and read again at the next sync.
            LyricSidecarRead::Unreadable => (None, false),
        };
        writes[index] = Some(LyricSidecarWrite {
            sidecar: found[index].clone(),
            text,
            oversized,
            unchanged: false,
        });
    }
    let writes: Vec<LyricSidecarWrite> = writes.into_iter().flatten().collect();
    repository.save_lyric_sidecars(root.id, &writes, complete)
}

/// How many tracks are parsed at once when the adapter offers a parallel reader.
pub const METADATA_READ_THREADS: usize = 4;

enum EntryKind {
    Unchanged,
    /// Unchanged file whose stored metadata predates the current metadata version.
    Legacy,
    Changed,
}

struct Entry {
    track: MediaTrackRecord,
    kind: EntryKind,
    needs_read: bool,
    read: Option<Result<TrackMetadata, TrackMetadataError>>,
}

/// One parse; a parser that panics on a damaged file counts as a failed read, not a failed sync.
fn read_guarded(
    reader: &dyn MetadataReader,
    track: &MediaTrackRecord,
) -> Result<TrackMetadata, TrackMetadataError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reader.read(track))).unwrap_or_else(
        |_| {
            Err(TrackMetadataError {
                message: "the metadata parser failed on this file".to_owned(),
            })
        },
    )
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
                created_at_utc_ms: None,
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

    mod parallel_reading {
        use std::{
            collections::HashSet,
            sync::{atomic::AtomicUsize, Mutex},
            thread::ThreadId,
            time::Duration,
        };

        use super::*;
        use crate::library::MetadataReader;
        use crate::sync::METADATA_READ_THREADS;

        struct SharedReader {
            threads: Mutex<HashSet<ThreadId>>,
            reads: AtomicUsize,
            cancel_after: Option<(usize, SyncCancellation)>,
            failing: Vec<String>,
        }

        impl MetadataReader for SharedReader {
            fn read(&self, track: &MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError> {
                std::thread::sleep(Duration::from_millis(3));
                self.threads
                    .lock()
                    .unwrap()
                    .insert(std::thread::current().id());
                let count = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
                if let Some((after, cancellation)) = &self.cancel_after {
                    if count == *after {
                        cancellation.cancel();
                    }
                }
                if self.failing.contains(&track.identity.source_item_id) {
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

        struct ParallelIndex {
            scan: Option<SourceScan>,
            reader: SharedReader,
        }

        impl MediaIndex for ParallelIndex {
            fn scan(&mut self, _root: &LibraryRoot) -> SourceScan {
                self.scan.take().expect("one scan result")
            }

            fn read_metadata(
                &mut self,
                _track: &MediaTrackRecord,
            ) -> Result<TrackMetadata, TrackMetadataError> {
                panic!("the shared reader reads every track when there is one");
            }

            fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
                Some(&self.reader)
            }
        }

        fn parallel_index(
            root: &LibraryRoot,
            count: usize,
            failing: Vec<String>,
            cancel_after: Option<(usize, SyncCancellation)>,
        ) -> ParallelIndex {
            ParallelIndex {
                scan: Some(SourceScan {
                    source_id: root.id,
                    state: SourceScanState::Complete,
                    tracks: (0..count)
                        .map(|number| record(root.id, &format!("track-{number:03}"), 100, None))
                        .collect(),
                    errors: Vec::new(),
                }),
                reader: SharedReader {
                    threads: Mutex::new(HashSet::new()),
                    reads: AtomicUsize::new(0),
                    cancel_after,
                    failing,
                },
            }
        }

        #[test]
        fn tracks_are_read_on_up_to_four_threads_and_applied_in_track_order() {
            let root = root();
            let mut index = parallel_index(&root, 40, vec!["track-007".to_owned()], None);
            let mut repository = FakeRepository::default();
            let report = SyncEngine::sync_cancellable_with_progress(
                &root,
                &mut index,
                &mut repository,
                1,
                &SyncCancellation::default(),
                |_| {},
            )
            .expect("sync");
            assert_eq!(report.observed, 40);
            assert_eq!(report.metadata_reads, 40);
            assert!(!report.cancelled);
            assert!(matches!(report.state, Some(SourceScanState::Complete)));
            let expected: Vec<String> =
                (0..40).map(|number| format!("track-{number:03}")).collect();
            assert_eq!(
                repository.observed, expected,
                "applied in track order, whichever thread finished first"
            );
            assert_eq!(repository.changed, expected);
            assert_eq!(
                report
                    .metadata_errors
                    .iter()
                    .map(|(item, _)| item.as_str())
                    .collect::<Vec<_>>(),
                ["track-007"]
            );
            let threads = index.reader.threads.lock().unwrap().len();
            assert!(
                threads > 1 && threads <= METADATA_READ_THREADS,
                "read on {threads} threads, expected 2 to {METADATA_READ_THREADS}"
            );
        }

        #[test]
        fn cancelling_stops_reading_and_saves_what_was_read_without_removing_anything() {
            let root = root();
            let cancellation = SyncCancellation::default();
            let mut index = parallel_index(&root, 60, Vec::new(), Some((10, cancellation.clone())));
            let mut repository = FakeRepository::default();
            let report = SyncEngine::sync_cancellable_with_progress(
                &root,
                &mut index,
                &mut repository,
                1,
                &cancellation,
                |_| {},
            )
            .expect("sync");
            assert!(report.cancelled);
            let saved = repository.observed.len();
            assert!(
                (10..60).contains(&saved),
                "the tracks read before the cancellation are saved, not all 60 and not none: {saved}"
            );
            assert_eq!(
                repository.changed.len(),
                saved,
                "every saved track carries its metadata"
            );
            assert_eq!(report.applied.inserted_or_updated, saved as u64);
            let state = repository.applied_state.expect("progress was written");
            assert!(
                !state.allows_reconciliation(),
                "an interrupted scan never removes tracks"
            );
            assert!(
                index.reader.reads.load(Ordering::SeqCst) < 60,
                "reading stopped early"
            );
        }

        #[test]
        fn a_cancellation_before_any_read_saves_nothing() {
            let root = root();
            let cancellation = SyncCancellation::default();
            cancellation.cancel();
            let mut index = parallel_index(&root, 20, Vec::new(), None);
            let mut repository = FakeRepository::default();
            let report = SyncEngine::sync_cancellable_with_progress(
                &root,
                &mut index,
                &mut repository,
                1,
                &cancellation,
                |_| {},
            )
            .expect("sync");
            assert!(report.cancelled);
            assert!(repository.applied_state.is_none());
            assert_eq!(index.reader.reads.load(Ordering::SeqCst), 0);
        }
    }
}
