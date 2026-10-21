//! How much a remote scan pulls from the server per track. `MOE_SCAN_DIR` names a folder of music (read through a
//! counting `RemoteFs`, nothing goes over a network); `MOE_SCAN_FILES` limits how many files are read (default 40).
//! Run: `MOE_SCAN_DIR=D:\Music cargo test -p player-remote --test scan_transfer --release -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use player_core::{MediaLocator, MediaTrackRecord, MetadataReader, SourceId, TrackIdentity};
use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::local::DirFs;

#[derive(Default)]
struct Counters {
    bytes: AtomicU64,
    requests: AtomicU64,
    opens: AtomicU64,
}

struct CountingFile {
    inner: Box<dyn RemoteFile>,
    counters: Arc<Counters>,
}

impl RemoteFile for CountingFile {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let read = self.inner.read_at(offset, buf)?;
        if std::env::var_os("MOE_SCAN_TRACE").is_some() {
            println!("  request: offset {offset:>10}  asked {:>6}  got {read:>6}  (file {} bytes)", buf.len(), self.inner.len());
        }
        self.counters.bytes.fetch_add(read as u64, Ordering::Relaxed);
        self.counters.requests.fetch_add(1, Ordering::Relaxed);
        Ok(read)
    }
}

struct CountingFs {
    inner: DirFs,
    counters: Arc<Counters>,
}

impl RemoteFs for CountingFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.inner.list(dir)
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.inner.stat(path)
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        self.counters.opens.fetch_add(1, Ordering::Relaxed);
        Ok(Box::new(CountingFile {
            inner: self.inner.open(path)?,
            counters: Arc::clone(&self.counters),
        }))
    }
}

struct Fixed(Arc<dyn RemoteFs>);

impl RemoteResolver for Fixed {
    fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
        Ok(ResolvedSource {
            fs: Arc::clone(&self.0),
            base_path: String::new(),
            excluded: Vec::new(),
        })
    }
}

fn collect(fs: &dyn RemoteFs, dir: &str, out: &mut Vec<(String, u64)>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(entries) = fs.list(dir) else { return };
    for entry in entries {
        if out.len() >= limit {
            return;
        }
        let path = if dir.is_empty() { entry.name.clone() } else { format!("{dir}/{}", entry.name) };
        if entry.is_dir {
            collect(fs, &path, out, limit);
        } else if matches!(
            path.rsplit('.').next().map(str::to_ascii_lowercase).as_deref(),
            Some("flac" | "mp3" | "m4a" | "wv" | "opus" | "ogg" | "wav" | "dsf")
        ) {
            out.push((path, entry.size));
        }
    }
}

#[test]
#[ignore = "needs MOE_SCAN_DIR"]
fn a_scan_does_not_pull_whole_files_or_their_covers() {
    let Ok(dir) = std::env::var("MOE_SCAN_DIR") else {
        return;
    };
    let limit = std::env::var("MOE_SCAN_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let counters = Arc::new(Counters::default());
    let fs = Arc::new(CountingFs { inner: DirFs::new(PathBuf::from(&dir)), counters: Arc::clone(&counters) });
    let mut files = Vec::new();
    collect(fs.as_ref(), "", &mut files, limit);
    let index = RemoteMediaIndex::new(Arc::new(Fixed(fs.clone() as Arc<dyn RemoteFs>)));
    let source_id = SourceId::new();
    let mut per_kind: BTreeMap<String, (u64, u64, u64, u64)> = BTreeMap::new();
    let mut failures = 0;
    for (path, size) in &files {
        let before = (counters.bytes.load(Ordering::Relaxed), counters.requests.load(Ordering::Relaxed), counters.opens.load(Ordering::Relaxed));
        let track = MediaTrackRecord {
            identity: TrackIdentity { source_id, source_item_id: path.clone(), locator_key: None },
            locator: MediaLocator::Remote { source_id, path: path.clone() },
            fingerprint: player_core::FileFingerprint { size_bytes: *size, modified_at_utc_ms: None, created_at_utc_ms: None },
            metadata: None,
        };
        if index.read(&track).is_err() {
            failures += 1;
        }
        let kind = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let entry = per_kind.entry(kind).or_default();
        entry.0 += 1;
        entry.1 += counters.bytes.load(Ordering::Relaxed) - before.0;
        entry.2 += counters.requests.load(Ordering::Relaxed) - before.1;
        entry.3 += *size;
    }
    println!("files read: {} (failed {failures})", files.len());
    let (mut total_bytes, mut total_files) = (0, 0);
    for (kind, (count, bytes, requests, sizes)) in &per_kind {
        println!(
            "{kind:>5}: {count:>3} files | {:>8.1} KiB/file from the server | {:>5.1} requests/file | files average {:>8.1} KiB",
            *bytes as f64 / *count as f64 / 1024.0,
            *requests as f64 / *count as f64,
            *sizes as f64 / *count as f64 / 1024.0
        );
        total_bytes += bytes;
        total_files += count;
    }
    println!("all: {:.1} KiB per file, {} opens", total_bytes as f64 / total_files.max(1) as f64 / 1024.0, counters.opens.load(Ordering::Relaxed));
}
