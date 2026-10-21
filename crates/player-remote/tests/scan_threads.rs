//! How many parallel tag reads make a remote scan fastest on a real SMB server (files on spinning disks: switching
//! between files costs a seek, so more threads is not always faster). Read-only, tag-sized reads only.
//!
//! Environment (same as `real_smb.rs`): `MOE_SMB_HOST`, `MOE_SMB_PORT`, `MOE_SMB_SHARE`, `MOE_SMB_FILE` (user name on
//! line 1, password on line 2), `MOE_SMB_DIR` (music folder inside the share). Optional: `MOE_SCAN_PER_RUN` (files per
//! run, default 120), `MOE_SCAN_PAUSE_S` (seconds between runs, default 4). Nothing secret is printed.
//! Run: `cargo test -p player-remote --test scan_threads --release -- --ignored --nocapture`.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use player_core::{MediaLocator, MediaTrackRecord, MetadataReader, SourceId, TrackIdentity};
use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::smb::{SmbFs, SmbSettings};
use zeroize::Zeroizing;

fn settings() -> Option<SmbSettings> {
    let host = std::env::var("MOE_SMB_HOST").ok()?;
    let share = std::env::var("MOE_SMB_SHARE").ok()?;
    let file = std::fs::read_to_string(std::env::var("MOE_SMB_FILE").ok()?).ok()?;
    let mut lines = file.lines();
    Some(SmbSettings {
        host,
        port: std::env::var("MOE_SMB_PORT").ok().and_then(|p| p.parse().ok()),
        share,
        base_path: std::env::var("MOE_SMB_DIR").unwrap_or_default(),
        user: lines.next()?.trim().to_owned(),
        password: Zeroizing::new(lines.next()?.trim().to_owned()),
    })
}

#[derive(Default)]
struct Counters {
    bytes: AtomicU64,
    requests: AtomicU64,
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
        self.counters.bytes.fetch_add(read as u64, Ordering::Relaxed);
        self.counters.requests.fetch_add(1, Ordering::Relaxed);
        Ok(read)
    }
}

struct CountingFs {
    inner: Arc<SmbFs>,
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
        Ok(Box::new(CountingFile { inner: self.inner.open(path)?, counters: Arc::clone(&self.counters) }))
    }
}

struct Fixed(Arc<dyn RemoteFs>);

impl RemoteResolver for Fixed {
    fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
        Ok(ResolvedSource { fs: Arc::clone(&self.0), base_path: String::new(), excluded: Vec::new() })
    }
}

fn collect(fs: &dyn RemoteFs, dir: &str, out: &mut Vec<(String, u64)>) {
    let Ok(entries) = fs.list(dir) else { return };
    for entry in entries {
        let path = if dir.is_empty() { entry.name.clone() } else { format!("{dir}/{}", entry.name) };
        if entry.is_dir {
            collect(fs, &path, out);
        } else if player_remote::index::is_remote_library_audio(&entry.name) {
            out.push((path, entry.size));
        }
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, (self.next() % (i as u64 + 1)) as usize);
        }
    }
}

struct RunResult {
    threads: usize,
    label: &'static str,
    files: usize,
    wall: f64,
    p50: f64,
    p95: f64,
    max: f64,
    requests: u64,
    bytes: u64,
    failures: usize,
}

impl RunResult {
    fn rate(&self) -> f64 {
        self.files as f64 / self.wall
    }
}

fn run(index: &RemoteMediaIndex, counters: &Counters, files: &[(String, u64)], threads: usize, label: &'static str) -> RunResult {
    let source_id = SourceId::new();
    let next = AtomicUsize::new(0);
    let latencies = Mutex::new(Vec::with_capacity(files.len()));
    let failures = AtomicUsize::new(0);
    let (b0, r0) = (counters.bytes.load(Ordering::Relaxed), counters.requests.load(Ordering::Relaxed));
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some((path, size)) = files.get(i) else { break };
                let track = MediaTrackRecord {
                    identity: TrackIdentity { source_id, source_item_id: path.clone(), locator_key: None },
                    locator: MediaLocator::Remote { source_id, path: path.clone() },
                    fingerprint: player_core::FileFingerprint { size_bytes: *size, modified_at_utc_ms: None, created_at_utc_ms: None },
                    metadata: None,
                };
                let t = Instant::now();
                if index.read(&track).is_err() {
                    failures.fetch_add(1, Ordering::Relaxed);
                }
                latencies.lock().unwrap().push(t.elapsed().as_secs_f64() * 1000.0);
            });
        }
    });
    let wall = started.elapsed().as_secs_f64();
    let mut l = latencies.into_inner().unwrap();
    l.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| l[((l.len() as f64 * p) as usize).min(l.len() - 1)];
    RunResult {
        threads,
        label,
        files: files.len(),
        wall,
        p50: pct(0.5),
        p95: pct(0.95),
        max: *l.last().unwrap(),
        requests: counters.requests.load(Ordering::Relaxed) - r0,
        bytes: counters.bytes.load(Ordering::Relaxed) - b0,
        failures: failures.load(Ordering::Relaxed),
    }
}

fn print(r: &RunResult) {
    println!(
        "{:<8} threads {:>2} | {:>4} files {:>7.2} s | {:>6.1} files/s | p50 {:>6.0} ms p95 {:>6.0} ms max {:>6.0} ms | {:>5} req {:>6.0} KiB | failures {}",
        r.label, r.threads, r.files, r.wall, r.rate(), r.p50, r.p95, r.max, r.requests, r.bytes as f64 / 1024.0, r.failures
    );
}

#[test]
#[ignore = "needs MOE_SMB_* and a real server"]
fn tag_read_throughput_per_thread_count() {
    let Some(settings) = settings() else {
        eprintln!("skipped: MOE_SMB_* is not set");
        return;
    };
    let per_run: usize = std::env::var("MOE_SCAN_PER_RUN").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
    let pause = Duration::from_secs(std::env::var("MOE_SCAN_PAUSE_S").ok().and_then(|v| v.parse().ok()).unwrap_or(4));
    let smb = Arc::new(SmbFs::connect(settings).expect("connect"));
    let counters = Arc::new(Counters::default());
    let fs: Arc<dyn RemoteFs> = Arc::new(CountingFs { inner: Arc::clone(&smb), counters: Arc::clone(&counters) });

    // Round trip: a stat of the base folder, a few times.
    let rtts: Vec<f64> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let _ = smb.stat("");
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    println!("stat round trips (ms): {rtts:.0?}");

    let listing = Instant::now();
    let mut files = Vec::new();
    collect(fs.as_ref(), "", &mut files);
    println!("listed {} audio files in {:.1} s", files.len(), listing.elapsed().as_secs_f64());

    // `MOE_SCAN_START` skips that many files at the front (to measure a region nothing has read recently);
    // `MOE_SCAN_COUNTS` is a comma list of thread counts.
    let skip: usize = std::env::var("MOE_SCAN_START").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let files: Vec<(String, u64)> = files.into_iter().skip(skip).collect();
    let counts: Vec<usize> = std::env::var("MOE_SCAN_COUNTS")
        .ok()
        .map(|v| v.split(',').filter_map(|c| c.trim().parse().ok()).collect())
        .unwrap_or_else(|| vec![1usize, 2, 4, 8, 12, 16, 24, 32, 48]);
    let sweep_files = counts.len() * per_run;
    let random_runs = 3;
    assert!(files.len() >= 2 * sweep_files + random_runs * per_run, "not enough files ({}) for per-run {per_run}", files.len());

    let index = RemoteMediaIndex::new(Arc::new(Fixed(Arc::clone(&fs))));
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut results: Vec<RunResult> = Vec::new();
    let mut slices: Vec<(usize, usize)> = Vec::new(); // (threads, start)

    // Two cold sweeps, each thread count on its own disjoint slice, in a random order per sweep.
    for sweep in 0..2 {
        let mut order = counts.clone();
        rng.shuffle(&mut order);
        println!("--- cold in-order sweep {} (order {:?}) ---", sweep + 1, order);
        for (n, &threads) in order.iter().enumerate() {
            let start = sweep * sweep_files + n * per_run;
            let r = run(&index, &counters, &files[start..start + per_run], threads, if sweep == 0 { "cold-1" } else { "cold-2" });
            print(&r);
            slices.push((threads, start));
            results.push(r);
            std::thread::sleep(pause);
        }
    }

    // Best three thread counts by mean rate over the two sweeps.
    let mut mean: Vec<(usize, f64)> = counts
        .iter()
        .map(|&t| {
            let v: Vec<f64> = results.iter().filter(|r| r.threads == t).map(RunResult::rate).collect();
            (t, v.iter().sum::<f64>() / v.len() as f64)
        })
        .collect();
    mean.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let best: Vec<usize> = mean.iter().take(3).map(|m| m.0).collect();
    println!("--- warm re-reads (best three: {best:?}); each re-reads a slice an earlier run already read ---");
    for (k, &threads) in best.iter().enumerate() {
        let (_, start) = slices[(k * 5 + 1) % slices.len()];
        let r = run(&index, &counters, &files[start..start + per_run], threads, "warm");
        print(&r);
        results.push(r);
        std::thread::sleep(pause);
    }

    // Random picks from the never-read tail: every file somewhere else, so each read is a seek on the disks.
    println!("--- shuffled (random files from the never-read tail) ---");
    let mut tail: Vec<(String, u64)> = files[2 * sweep_files..].to_vec();
    rng.shuffle(&mut tail);
    for (n, &threads) in [4usize, 16, 8].iter().enumerate() {
        let r = run(&index, &counters, &tail[n * per_run..(n + 1) * per_run], threads, "shuffled");
        print(&r);
        results.push(r);
        std::thread::sleep(pause);
    }

    // Warm scaling: one slice that was just read, every thread count, twice (no disk work left, only the client and the link).
    println!("--- warm scaling (same slice for every thread count, two rounds, random order) ---");
    let (_, start) = slices[0];
    for round in 0..2 {
        let mut order = counts.clone();
        rng.shuffle(&mut order);
        for &threads in &order {
            let r = run(&index, &counters, &files[start..start + per_run], threads, if round == 0 { "warmsc-1" } else { "warmsc-2" });
            print(&r);
            results.push(r);
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    println!("--- summary: mean files/s over the two cold sweeps ---");
    for (t, m) in &mean {
        println!("threads {t:>2}: {m:>6.1} files/s");
    }
    let failures: usize = results.iter().map(|r| r.failures).sum();
    println!("total failures: {failures}, total requests {}", counters.requests.load(Ordering::Relaxed));
}
