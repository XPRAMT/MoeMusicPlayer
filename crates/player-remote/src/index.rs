//! Scanning and tag reading for remote sources (ADR 0026): [`RemoteMediaIndex`] is the `MediaIndex` of every remote
//! root, whatever the protocol, because it only talks to the [`RemoteFs`] trait.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use player_core::{
    FileFingerprint, LibraryRoot, LyricSidecar, MediaIndex, MediaLocator, MediaScanProgress,
    MediaScanProgressUnit, MediaSourceError, MediaTrackRecord, MetadataReader, SourceId,
    SourceScan, SourceScanState, SyncCancellation, TrackIdentity, TrackMetadata,
    TrackMetadataError,
};
use player_track_metadata::{ByteSource, ReadSeek};

use crate::fs::{
    RangeReader, RemoteError, RemoteFile, RemoteFs, SharedRangeReader, METADATA_BLOCK_SIZE,
};

/// Directories listed at the same time. A listing costs a round trip, so a few at once make a scan several times
/// faster on a distant server without loading it.
const LIST_WORKERS: usize = 4;

/// Tracks whose tags are read at the same time (16 on every platform, Windows and Android alike). Each read is a few
/// round trips, so many at once keep a long link busy.
pub const METADATA_READ_THREADS: usize = 16;

/// A remote source's file tree and the folder inside it that the source is rooted at.
#[derive(Clone)]
pub struct ResolvedSource {
    pub fs: Arc<dyn RemoteFs>,
    /// `/`-separated, relative to the account's root; `""` is the account's whole tree.
    pub base_path: String,
    /// Folders (relative to the account's root) the user excluded: neither they nor anything under them is scanned.
    pub excluded: Vec<String>,
}

/// Whether `path` is `excluded` or lies under it (both relative to the same root, `/`-separated).
pub fn is_excluded(path: &str, excluded: &[String]) -> bool {
    let path = path.trim_matches('/');
    excluded.iter().any(|folder| {
        let folder = folder.trim_matches('/');
        !folder.is_empty()
            && (path == folder
                || path
                    .strip_prefix(folder)
                    .is_some_and(|rest| rest.starts_with('/')))
    })
}

/// Finds the file tree of a source. The app implements it over the accounts it holds; tests over a folder.
pub trait RemoteResolver: Send + Sync {
    fn resolve(&self, source_id: SourceId) -> Result<ResolvedSource, RemoteError>;

    /// The account (server) the source belongs to. Every source of one account that reaches the same file identifies it
    /// with the same locator key ([`remote_locator_key`]), so the file is one track however many sources hold it. The
    /// default (each source is its own account) suits resolvers that know a single source.
    fn account_id(&self, source_id: SourceId) -> String {
        source_id.to_string()
    }
}

/// The identity key of a remote file, shared by every source of the account: the account and the file's path from the
/// account's root (not the path inside one source, and not the source's id).
pub fn remote_locator_key(account_id: &str, account_path: &str) -> String {
    format!("remote-acct:{account_id}:{}", join_path("", account_path))
}

/// `a` + `b` as one `/`-separated path without empty pieces.
pub fn join_path(base: &str, relative: &str) -> String {
    let mut parts = Vec::new();
    for piece in base.split('/').chain(relative.split('/')) {
        if !piece.is_empty() {
            parts.push(piece);
        }
    }
    parts.join("/")
}

/// Whether the library accepts a file with this name on this platform.
pub fn is_remote_library_audio(name: &str) -> bool {
    let path = Path::new(name);
    if cfg!(target_os = "android") {
        player_formats::is_android_library_audio(path)
    } else {
        player_formats::is_windows_library_audio(path)
    }
}

pub struct RemoteMediaIndex {
    resolver: Arc<dyn RemoteResolver>,
    /// The `.lrc` files the last scan saw beside audio files (see `MediaIndex::take_lyric_sidecars`).
    sidecars: Vec<LyricSidecar>,
}

impl RemoteMediaIndex {
    pub fn new(resolver: Arc<dyn RemoteResolver>) -> Self {
        Self {
            resolver,
            sidecars: Vec::new(),
        }
    }
}

impl RemoteMediaIndex {
    /// Scan the files a playlist names instead of a folder tree. `paths` are relative to the source's base folder; the
    /// result keeps the order of `paths`. A file that cannot be reached is an error of the scan, not of the playlist.
    pub fn scan_paths(&self, source_id: SourceId, paths: &[String]) -> SourceScan {
        self.scan_paths_with_progress(source_id, paths, &mut |_| {})
    }

    /// As [`scan_paths`](Self::scan_paths), reporting how many of the songs are looked up on the server about every
    /// 100 ms (on the calling thread, so the callback needs no `Sync`). The sync engine turns the reports into the
    /// same banner progress a folder scan gives.
    pub fn scan_paths_with_progress(
        &self,
        source_id: SourceId,
        paths: &[String],
        on_progress: &mut dyn FnMut(MediaScanProgress),
    ) -> SourceScan {
        let mut scan = SourceScan {
            source_id,
            state: SourceScanState::Unavailable {
                reason: "scan did not start".to_owned(),
            },
            tracks: Vec::new(),
            errors: Vec::new(),
        };
        let resolved = match self.resolver.resolve(source_id) {
            Ok(resolved) => resolved,
            Err(error) => {
                scan.state = unavailable(&error);
                return scan;
            }
        };
        let wanted: Vec<&String> = paths
            .iter()
            .filter(|path| {
                let name = path.rsplit('/').next().unwrap_or("");
                !name.starts_with('.')
                    && is_remote_library_audio(name)
                    && !is_excluded(&join_path(&resolved.base_path, path), &resolved.excluded)
            })
            .collect();
        let results = Mutex::new(vec![None; wanted.len()]);
        let next = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let total = wanted.len() as u64;
        let mut report = |done: u64| {
            on_progress(MediaScanProgress {
                processed: done.min(total),
                total: Some(total),
                unit: MediaScanProgressUnit::Tracks,
            });
        };
        report(0);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..LIST_WORKERS.min(wanted.len()))
                .map(|_| {
                    scope.spawn(|| loop {
                        let index = next.fetch_add(1, Ordering::SeqCst);
                        let Some(path) = wanted.get(index) else {
                            return;
                        };
                        let outcome = resolved
                            .fs
                            .stat(&join_path(&resolved.base_path, path));
                        results.lock().unwrap_or_else(PoisonError::into_inner)[index] =
                            Some(outcome);
                        finished.fetch_add(1, Ordering::SeqCst);
                    })
                })
                .collect();
            // Report while the workers run: the server is slow, so a long list would otherwise sit at zero until done.
            let mut reported = 0;
            while workers.iter().any(|worker| !worker.is_finished()) {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let done = finished.load(Ordering::SeqCst) as u64;
                if done != reported {
                    reported = done;
                    report(done);
                }
            }
        });
        report(total);
        let results = results.into_inner().unwrap_or_else(PoisonError::into_inner);
        let mut seen = std::collections::HashSet::new();
        for (path, outcome) in wanted.into_iter().zip(results) {
            if !seen.insert(path.clone()) {
                continue;
            }
            match outcome {
                Some(Ok(entry)) if !entry.is_dir => scan.tracks.push(MediaTrackRecord {
                    identity: TrackIdentity {
                        source_id,
                        source_item_id: path.clone(),
                        locator_key: Some(remote_locator_key(
                            &self.resolver.account_id(source_id),
                            &join_path(&resolved.base_path, path),
                        )),
                    },
                    locator: MediaLocator::Remote {
                        source_id,
                        path: path.clone(),
                    },
                    fingerprint: FileFingerprint {
                        size_bytes: entry.size,
                        modified_at_utc_ms: entry.modified_ms,
                        created_at_utc_ms: None,
                    },
                    metadata: None,
                }),
                Some(Ok(_)) => {}
                Some(Err(error)) => scan.errors.push(MediaSourceError {
                    source_item_id: Some(path.clone()),
                    message: error.to_string(),
                }),
                None => {}
            }
        }
        // A file that could not be reached may only be a bad moment on the link: nothing is removed for it.
        scan.state = if scan.errors.is_empty() {
            SourceScanState::Complete
        } else {
            SourceScanState::Incomplete {
                reason: "some files of the playlist could not be reached".to_owned(),
            }
        };
        scan
    }
}

impl RemoteMediaIndex {
    /// The `.lrc` files beside these songs (each folder is listed once): for a source that names its songs instead of
    /// walking a tree, such as a playlist file.
    pub fn lyric_sidecars_beside(
        &self,
        source_id: SourceId,
        tracks: &[MediaTrackRecord],
    ) -> Vec<LyricSidecar> {
        let Ok(resolved) = self.resolver.resolve(source_id) else {
            return Vec::new();
        };
        let mut by_folder: std::collections::HashMap<String, Vec<&MediaTrackRecord>> =
            std::collections::HashMap::new();
        for track in tracks {
            if let MediaLocator::Remote { path, .. } = &track.locator {
                let folder = path.rsplit_once('/').map_or("", |(folder, _)| folder);
                by_folder.entry(folder.to_owned()).or_default().push(track);
            }
        }
        let mut found = Vec::new();
        for (folder, songs) in by_folder {
            let Ok(entries) = resolved.fs.list(&join_path(&resolved.base_path, &folder)) else {
                continue;
            };
            let lyrics: std::collections::HashMap<String, &crate::fs::RemoteEntry> = entries
                .iter()
                .filter(|entry| !entry.is_dir && entry.name.to_ascii_lowercase().ends_with(".lrc"))
                .map(|entry| (entry.name[..entry.name.len() - 4].to_lowercase(), entry))
                .collect();
            for track in songs {
                let MediaLocator::Remote { path, .. } = &track.locator else {
                    continue;
                };
                let name = path.rsplit('/').next().unwrap_or(path);
                let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem).to_lowercase();
                let Some(entry) = lyrics.get(&stem) else {
                    continue;
                };
                found.push(LyricSidecar {
                    track_item_id: track.identity.source_item_id.clone(),
                    locator: MediaLocator::Remote {
                        source_id,
                        path: if folder.is_empty() {
                            entry.name.clone()
                        } else {
                            format!("{folder}/{}", entry.name)
                        },
                    },
                    name: entry.name.clone(),
                    size_bytes: entry.size,
                    modified_at_utc_ms: entry.modified_ms,
                });
            }
        }
        found
    }
}

/// One remote file as the tag reader's byte source.
struct RemoteByteSource {
    fs: Arc<dyn RemoteFs>,
    /// Relative to the account's root.
    path: String,
    /// The song's open file, made at the first `open` and shared by the next ones.
    shared: std::sync::Mutex<Option<SharedRangeReader>>,
}

/// The most blocks (1 MiB) one request of a tag read may grow to.
const METADATA_GROWTH_LIMIT_BLOCKS: u64 = 64;

/// A reader tuned for tags: small blocks, a short read-ahead. A tag with a megabyte-sized picture in it is read
/// through by the parser (MP3 and DSF `APIC`, MP4 `ilst`, the Ogg/Opus comment packet): the read-ahead then grows with
/// the run, so that is a few big requests instead of one per 32 KiB. Ordinary files never reach the growth.
fn metadata_reader(file: Box<dyn RemoteFile>) -> RangeReader {
    RangeReader::with_geometry(file, METADATA_BLOCK_SIZE, 64)
        .with_read_ahead(2)
        .with_growing_read_ahead(METADATA_GROWTH_LIMIT_BLOCKS)
}

impl ByteSource for RemoteByteSource {
    fn file_name(&self) -> String {
        self.path.rsplit('/').next().unwrap_or("").to_owned()
    }

    fn open(&self) -> std::io::Result<Box<dyn ReadSeek>> {
        // The tag readers open the song several times (the type probe, the parser, one frame again): they share one
        // open file and one cache, and the pieces are small, because the tags are at the start (or the end) of a file.
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if shared.is_none() {
            let file = self.fs.open(&self.path)?;
            *shared = Some(SharedRangeReader::new(metadata_reader(file)));
        }
        Ok(Box::new(shared.as_ref().map(SharedRangeReader::handle).expect("opened above")))
    }

    fn open_correction(&self) -> Option<Box<dyn ReadSeek>> {
        let correction = format!("{}c", self.path);
        let file = self.fs.open(&correction).ok()?;
        Some(Box::new(metadata_reader(file)))
    }
}

impl MetadataReader for RemoteMediaIndex {
    fn read(&self, track: &MediaTrackRecord) -> Result<TrackMetadata, TrackMetadataError> {
        let MediaLocator::Remote { source_id, path } = &track.locator else {
            return Err(TrackMetadataError {
                message: "the remote reader needs a remote locator".to_owned(),
            });
        };
        let resolved = self
            .resolver
            .resolve(*source_id)
            .map_err(|error| TrackMetadataError {
                message: error.to_string(),
            })?;
        let source = RemoteByteSource {
            fs: resolved.fs,
            path: join_path(&resolved.base_path, path),
            shared: std::sync::Mutex::new(None),
        };
        // A read that fails on a slow or busy link is tried once more before it is reported (a timeout on a mobile
        // connection is common when many tracks are read at once).
        match player_track_metadata::read_track_metadata(&source) {
            Ok(metadata) => Ok(metadata),
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(500));
                player_track_metadata::read_track_metadata(&source)
            }
        }
    }
}

/// What the listing workers share.
struct Walk {
    queue: Mutex<Vec<String>>,
    /// Directories taken from the queue and not yet finished: the walk is over when this is 0 and the queue is empty.
    in_flight: AtomicUsize,
    tracks: Mutex<Vec<MediaTrackRecord>>,
    errors: Mutex<Vec<MediaSourceError>>,
    incomplete: AtomicBool,
    cancelled: AtomicBool,
    /// Entries seen (files and folders), for progress.
    seen: AtomicU64,
    /// `.lrc` files found beside audio files, from the folder listings.
    sidecars: Mutex<Vec<LyricSidecar>>,
}

impl MediaIndex for RemoteMediaIndex {
    fn parallel_metadata_reader(&self) -> Option<&dyn MetadataReader> {
        Some(self)
    }

    fn metadata_read_threads(&self) -> usize {
        METADATA_READ_THREADS
    }

    fn scan(&mut self, root: &LibraryRoot) -> SourceScan {
        self.scan_with_progress(root, &mut |_| {}, &SyncCancellation::default())
    }

    fn take_lyric_sidecars(&mut self) -> Vec<LyricSidecar> {
        std::mem::take(&mut self.sidecars)
    }

    fn scans_lyric_sidecars(&self) -> bool {
        true
    }

    fn scan_with_progress(
        &mut self,
        root: &LibraryRoot,
        progress: &mut dyn FnMut(MediaScanProgress),
        cancellation: &SyncCancellation,
    ) -> SourceScan {
        let mut scan = SourceScan {
            source_id: root.id,
            state: SourceScanState::Unavailable {
                reason: "scan did not start".to_owned(),
            },
            tracks: Vec::new(),
            errors: Vec::new(),
        };
        if cancellation.is_cancelled() {
            scan.state = SourceScanState::Incomplete {
                reason: "remote scan cancelled before it started".to_owned(),
            };
            return scan;
        }
        if !root.enabled {
            scan.state = SourceScanState::Unavailable {
                reason: "library root is disabled".to_owned(),
            };
            return scan;
        }
        let MediaLocator::Remote { .. } = &root.locator else {
            scan.state = SourceScanState::Unavailable {
                reason: "a remote library root needs a remote locator".to_owned(),
            };
            return scan;
        };
        let resolved = match self.resolver.resolve(root.id) {
            Ok(resolved) => resolved,
            Err(error) => {
                scan.state = unavailable(&error);
                return scan;
            }
        };

        let base = resolved.base_path.clone();
        let account = self.resolver.account_id(root.id);
        if is_excluded(&base, &resolved.excluded) {
            // The whole source lies in an excluded folder: nothing to scan, nothing of it stays.
            scan.state = SourceScanState::Complete;
            return scan;
        }
        // The root must be listable, or the source is unreachable and nothing may be removed from the library.
        let first = match resolved.fs.list(&base) {
            Ok(entries) => entries,
            Err(error) => {
                scan.state = unavailable(&error);
                return scan;
            }
        };

        let walk = Walk {
            queue: Mutex::new(Vec::new()),
            in_flight: AtomicUsize::new(0),
            tracks: Mutex::new(Vec::new()),
            errors: Mutex::new(Vec::new()),
            incomplete: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            seen: AtomicU64::new(0),
            sidecars: Mutex::new(Vec::new()),
        };
        handle_entries(&walk, root.id, "", first, &base, &account, &resolved.excluded);

        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..LIST_WORKERS)
                .map(|_| {
                    scope.spawn(|| {
                        list_worker(
                            &walk,
                            root.id,
                            resolved.fs.as_ref(),
                            &base,
                            &account,
                            &resolved.excluded,
                            cancellation,
                        )
                    })
                })
                .collect();
            while !workers.iter().all(|worker| worker.is_finished()) {
                progress(MediaScanProgress {
                    processed: walk.seen.load(Ordering::Relaxed),
                    total: None,
                    unit: MediaScanProgressUnit::FilesystemEntries,
                });
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        let mut tracks = walk
            .tracks
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        // The workers finish in any order; a stable order keeps a scan reproducible.
        tracks.sort_by(|left, right| {
            left.identity
                .source_item_id
                .cmp(&right.identity.source_item_id)
        });
        scan.tracks = tracks;
        self.sidecars = walk
            .sidecars
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        scan.errors = walk
            .errors
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        scan.state = if walk.cancelled.load(Ordering::Relaxed) || cancellation.is_cancelled() {
            SourceScanState::Incomplete {
                reason: "remote scan cancelled before the whole tree was listed".to_owned(),
            }
        } else if walk.incomplete.load(Ordering::Relaxed) {
            SourceScanState::Incomplete {
                reason: "one or more remote folders could not be listed".to_owned(),
            }
        } else {
            SourceScanState::Complete
        };
        scan
    }

    fn read_metadata(
        &mut self,
        track: &MediaTrackRecord,
    ) -> Result<TrackMetadata, TrackMetadataError> {
        MetadataReader::read(self, track)
    }
}

fn unavailable(error: &RemoteError) -> SourceScanState {
    match error {
        RemoteError::Auth => SourceScanState::PermissionRevoked {
            reason: error.to_string(),
        },
        _ => SourceScanState::Unavailable {
            reason: error.to_string(),
        },
    }
}

/// Take directories off the queue and list them until the whole tree is done.
fn list_worker(
    walk: &Walk,
    source_id: SourceId,
    fs: &dyn RemoteFs,
    base: &str,
    account: &str,
    excluded: &[String],
    cancellation: &SyncCancellation,
) {
    loop {
        if cancellation.is_cancelled() {
            walk.cancelled.store(true, Ordering::Relaxed);
            return;
        }
        let next = {
            let mut queue = walk.queue.lock().unwrap_or_else(PoisonError::into_inner);
            let next = queue.pop();
            if next.is_some() {
                // Counted while the queue is locked, so another worker never sees an empty queue with nothing in
                // flight while this directory is being taken.
                walk.in_flight.fetch_add(1, Ordering::SeqCst);
            }
            next
        };
        let Some(directory) = next else {
            if walk.in_flight.load(Ordering::SeqCst) == 0 {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
            continue;
        };
        match fs.list(&join_path(base, &directory)) {
            Ok(entries) => handle_entries(walk, source_id, &directory, entries, base, account, excluded),
            Err(error) => {
                walk.incomplete.store(true, Ordering::Relaxed);
                walk.errors
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(MediaSourceError {
                        source_item_id: Some(directory.clone()),
                        message: error.to_string(),
                    });
            }
        }
        walk.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Queue the folders of a listing and record its audio files. `directory` is relative to the source root.
fn handle_entries(
    walk: &Walk,
    source_id: SourceId,
    directory: &str,
    entries: Vec<crate::fs::RemoteEntry>,
    base: &str,
    account: &str,
    excluded: &[String],
) {
    walk.seen.fetch_add(entries.len() as u64, Ordering::Relaxed);
    let mut folders = Vec::new();
    let mut files = Vec::new();
    // `.lrc` files of this folder by the lower-cased name without the extension, and the audio files by the same key.
    let mut lyrics: std::collections::HashMap<String, (String, u64, Option<i64>)> =
        std::collections::HashMap::new();
    let mut audio: Vec<(String, String)> = Vec::new();
    for entry in entries {
        if entry.name.is_empty() || entry.name.starts_with('.') {
            continue;
        }
        let relative = if directory.is_empty() {
            entry.name.clone()
        } else {
            format!("{directory}/{}", entry.name)
        };
        if entry.is_dir {
            if !is_excluded(&join_path(base, &relative), excluded) {
                folders.push(relative);
            }
        } else if entry.name.to_ascii_lowercase().ends_with(".lrc") {
            let stem = entry.name[..entry.name.len() - 4].to_lowercase();
            lyrics.insert(stem, (entry.name.clone(), entry.size, entry.modified_ms));
        } else if is_remote_library_audio(&entry.name) {
            let stem = entry
                .name
                .rsplit_once('.')
                .map_or(entry.name.as_str(), |(stem, _)| stem)
                .to_lowercase();
            audio.push((stem, relative.clone()));
            files.push(MediaTrackRecord {
                identity: TrackIdentity {
                    source_id,
                    source_item_id: relative.clone(),
                    locator_key: Some(remote_locator_key(account, &join_path(base, &relative))),
                },
                locator: MediaLocator::Remote {
                    source_id,
                    path: relative,
                },
                fingerprint: FileFingerprint {
                    size_bytes: entry.size,
                    modified_at_utc_ms: entry.modified_ms,
                    created_at_utc_ms: None,
                },
                metadata: None,
            });
        }
    }
    if !folders.is_empty() {
        walk.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(folders);
    }
    if !lyrics.is_empty() {
        let mut found = Vec::new();
        for (stem, track_item_id) in audio {
            let Some((name, size, modified)) = lyrics.get(&stem) else {
                continue;
            };
            let path = if directory.is_empty() {
                name.clone()
            } else {
                format!("{directory}/{name}")
            };
            found.push(LyricSidecar {
                track_item_id,
                locator: MediaLocator::Remote { source_id, path },
                name: name.clone(),
                size_bytes: *size,
                modified_at_utc_ms: *modified,
            });
        }
        if !found.is_empty() {
            walk.sidecars
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(found);
        }
    }
    if !files.is_empty() {
        walk.tracks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(files);
    }
}
