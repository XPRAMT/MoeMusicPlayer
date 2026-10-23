//! What keeping the creation time costs a remote folder scan: requests per file and wall time of a first sync, an
//! incremental resync of an unchanged library, and the one resync that fills the creation time in for a library made
//! before it was kept. The music is read through a counting `RemoteFs` over a local folder that reports a creation time
//! the way SMB does (nothing goes over a network).
//! Run: `MOE_SCAN_DIR=D:\Music cargo test -p player-remote --test scan_created_timing --release -- --ignored --nocapture`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use player_core::{LibraryRoot, MediaLocator, MediaSourceKind, SourceId, SyncEngine};
use player_db::Database;
use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::local::DirFs;

#[derive(Default)]
struct Counters {
    lists: AtomicU64,
    stats: AtomicU64,
    opens: AtomicU64,
    reads: AtomicU64,
}

impl Counters {
    fn take(&self) -> [u64; 4] {
        [
            self.lists.swap(0, Ordering::SeqCst),
            self.stats.swap(0, Ordering::SeqCst),
            self.opens.swap(0, Ordering::SeqCst),
            self.reads.swap(0, Ordering::SeqCst),
        ]
    }
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
        self.counters.reads.fetch_add(1, Ordering::Relaxed);
        self.inner.read_at(offset, buf)
    }
}

struct CountingFs {
    inner: DirFs,
    counters: Arc<Counters>,
    report_created: AtomicBool,
}

impl CountingFs {
    fn with_time(&self, path: &str, mut entry: RemoteEntry) -> RemoteEntry {
        if !entry.is_dir && self.report_created.load(Ordering::SeqCst) {
            let days: i64 = path.bytes().map(i64::from).sum::<i64>() % 4_000;
            entry.created_utc_100ns =
                Some((1_500_000_000_000 + days * 86_400_000) * 10_000 + 1_234);
        }
        entry
    }
}

impl RemoteFs for CountingFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.counters.lists.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .inner
            .list(dir)?
            .into_iter()
            .map(|entry| {
                let path = format!("{dir}/{}", entry.name);
                self.with_time(&path, entry)
            })
            .collect())
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.counters.stats.fetch_add(1, Ordering::Relaxed);
        let entry = self.inner.stat(path)?;
        Ok(self.with_time(path, entry))
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

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

#[test]
#[ignore = "needs MOE_SCAN_DIR; a measurement, not a check"]
fn creation_times_cost_no_requests_and_no_scan_time() {
    let Some(dir) = std::env::var_os("MOE_SCAN_DIR").map(PathBuf::from) else {
        eprintln!("MOE_SCAN_DIR is not set; skipped");
        return;
    };
    let runs: usize = std::env::var("MOE_SCAN_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let counters = Arc::new(Counters::default());
    let fs = Arc::new(CountingFs {
        inner: DirFs::new(&dir),
        counters: Arc::clone(&counters),
        report_created: AtomicBool::new(true),
    });
    let id = SourceId::new();
    let root = LibraryRoot {
        id,
        kind: MediaSourceKind::Remote,
        display_name: "remote".to_owned(),
        locator: MediaLocator::Remote {
            source_id: id,
            path: String::new(),
        },
        enabled: true,
    };
    let (mut first_ms, mut resync_ms, mut backfill_ms) = (Vec::new(), Vec::new(), Vec::new());

    for run in 0..runs {
        let mut index =
            RemoteMediaIndex::new(Arc::new(Fixed(Arc::clone(&fs) as Arc<dyn RemoteFs>)));
        // A first sync into an empty library.
        let mut db = Database::open_in_memory().expect("database");
        db.save_library_root(&root).expect("root");
        fs.report_created.store(true, Ordering::SeqCst);
        counters.take();
        let started = Instant::now();
        let report = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000).expect("sync");
        first_ms.push(started.elapsed().as_millis());
        let first = counters.take();
        let tracks = report.observed;

        // An incremental resync of the unchanged library.
        let started = Instant::now();
        let again = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000).expect("again");
        resync_ms.push(started.elapsed().as_millis());
        let resync = counters.take();
        assert_eq!(again.metadata_reads, 0);

        // A library made without creation times, then the resync that sees them.
        let mut old = Database::open_in_memory().expect("database");
        old.save_library_root(&root).expect("root");
        fs.report_created.store(false, Ordering::SeqCst);
        SyncEngine::sync(&root, &mut index, &mut old, 1_800_000_000_000).expect("old sync");
        fs.report_created.store(true, Ordering::SeqCst);
        counters.take();
        let started = Instant::now();
        let backfill =
            SyncEngine::sync(&root, &mut index, &mut old, 1_800_000_100_000).expect("backfill");
        backfill_ms.push(started.elapsed().as_millis());
        let backfill_requests = counters.take();
        assert_eq!(backfill.metadata_reads, 0);
        if run == 0 {
            let per = |counts: [u64; 4]| {
                let n = tracks.max(1) as f64;
                format!(
                    "list {} stat {} open {} ({:.2}/file) read {} ({:.2}/file)",
                    counts[0],
                    counts[1],
                    counts[2],
                    counts[2] as f64 / n,
                    counts[3],
                    counts[3] as f64 / n
                )
            };
            println!("tracks: {tracks}");
            println!("first sync requests:  {}", per(first));
            println!("resync requests:      {}", per(resync));
            println!("backfill requests:    {}", per(backfill_requests));
        }
    }
    println!(
        "median of {runs}: first sync {} ms, unchanged resync {} ms, backfill resync {} ms",
        median(first_ms),
        median(resync_ms),
        median(backfill_ms)
    );
}
