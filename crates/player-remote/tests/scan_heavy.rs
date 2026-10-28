//! Which files cost many requests when their tags are read (see the 「遠端掃描的傳輸量」 notes in docs/status/remote-sources.md), and why.
//! Four ignored steps, each configured by environment variables (nothing secret is printed):
//!
//! 1. `list_library` (needs `MOE_SMB_*`, as in `scan_threads.rs`): lists the music folder once and saves
//!    `path<TAB>size` lines to `MOE_HEAVY_LIST`.
//! 2. `sample_server` (needs `MOE_SMB_*`): reads the tags of the files in `MOE_HEAVY_RANGES` (for example
//!    `2200-2240,3900-3940`, indexes into the list) one at a time with a pause, and writes
//!    `index<TAB>path<TAB>size<TAB>requests<TAB>bytes<TAB>offset:len,...` lines to `MOE_HEAVY_OUT`.
//! 3. `copy_heads_and_tails` (needs `MOE_SMB_*`): for the paths listed in `MOE_HEAVY_PICK` (first column of each line)
//!    copies the first and last `MOE_HEAVY_EDGE_KIB` (default 2048) bytes into a file of the same length (zeros in
//!    between) in `MOE_HEAVY_DIR`, named `<n>.<extension>`; writes `MOE_HEAVY_DIR/map.txt`.
//! 4. `analyze_local`: reads the tags of every file in `MOE_HEAVY_DIR` through a counting `DirFs` and prints the trace
//!    of each file, flagging requests that fall outside the copied edges.
//!
//! Run: `cargo test -p player-remote --test scan_heavy --release -- --ignored --nocapture <step>`.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use player_core::{MediaLocator, MediaTrackRecord, MetadataReader, SourceId, TrackIdentity};
use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::local::DirFs;
use player_remote::smb::{SmbFs, SmbSettings};
use zeroize::Zeroizing;

fn settings() -> Option<SmbSettings> {
    let host = std::env::var("MOE_SMB_HOST").ok()?;
    let share = std::env::var("MOE_SMB_SHARE").ok()?;
    let file = std::fs::read_to_string(std::env::var("MOE_SMB_FILE").ok()?).ok()?;
    let mut lines = file.lines();
    Some(SmbSettings {
        host,
        port: std::env::var("MOE_SMB_PORT")
            .ok()
            .and_then(|p| p.parse().ok()),
        share,
        base_path: std::env::var("MOE_SMB_DIR").unwrap_or_default(),
        user: lines.next()?.trim().to_owned(),
        password: Zeroizing::new(lines.next()?.trim().to_owned()),
    })
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")))
}

/// Requests of the file being read right now (one file at a time).
type Trace = Arc<Mutex<Vec<(u64, usize)>>>;

struct TracingFile {
    inner: Box<dyn RemoteFile>,
    trace: Trace,
}

impl RemoteFile for TracingFile {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let read = self.inner.read_at(offset, buf)?;
        self.trace.lock().unwrap().push((offset, read));
        Ok(read)
    }
}

struct TracingFs {
    inner: Arc<dyn RemoteFs>,
    trace: Trace,
}

impl RemoteFs for TracingFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.inner.list(dir)
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.inner.stat(path)
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        Ok(Box::new(TracingFile {
            inner: self.inner.open(path)?,
            trace: Arc::clone(&self.trace),
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

fn collect(fs: &dyn RemoteFs, dir: &str, out: &mut Vec<(String, u64)>) {
    let Ok(entries) = fs.list(dir) else { return };
    for entry in entries {
        let path = if dir.is_empty() {
            entry.name.clone()
        } else {
            format!("{dir}/{}", entry.name)
        };
        if entry.is_dir {
            collect(fs, &path, out);
        } else if player_remote::index::is_remote_library_audio(&entry.name) {
            out.push((path, entry.size));
        }
    }
}

fn read_list(path: &PathBuf) -> Vec<(String, u64)> {
    std::fs::read_to_string(path)
        .expect("list file")
        .lines()
        .filter_map(|line| {
            let (path, size) = line.split_once('\t')?;
            Some((path.to_owned(), size.parse().ok()?))
        })
        .collect()
}

fn track(source_id: SourceId, path: &str, size: u64) -> MediaTrackRecord {
    MediaTrackRecord {
        identity: TrackIdentity {
            source_id,
            source_item_id: path.to_owned(),
            locator_key: None,
        },
        locator: MediaLocator::Remote {
            source_id,
            path: path.to_owned(),
        },
        fingerprint: player_core::FileFingerprint {
            size_bytes: size,
            modified_at_utc_ms: None,
            created_at_utc_ms: None,
        },
        metadata: None,
    }
}

fn format_trace(trace: &[(u64, usize)]) -> String {
    trace
        .iter()
        .map(|(o, l)| format!("{o}:{l}"))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
#[ignore = "needs MOE_SMB_* and a real server"]
fn list_library() {
    let Some(settings) = settings() else { return };
    let smb = SmbFs::connect(settings).expect("connect");
    let started = Instant::now();
    let mut files = Vec::new();
    collect(&smb, "", &mut files);
    let mut out = std::fs::File::create(env_path("MOE_HEAVY_LIST")).unwrap();
    for (path, size) in &files {
        writeln!(out, "{path}\t{size}").unwrap();
    }
    println!(
        "listed {} files in {:.1} s",
        files.len(),
        started.elapsed().as_secs_f64()
    );
}

#[test]
#[ignore = "needs MOE_SMB_* and a real server"]
fn sample_server() {
    let Some(settings) = settings() else { return };
    let files = read_list(&env_path("MOE_HEAVY_LIST"));
    let smb: Arc<dyn RemoteFs> = Arc::new(SmbFs::connect(settings).expect("connect"));
    let trace: Trace = Arc::default();
    let fs: Arc<dyn RemoteFs> = Arc::new(TracingFs {
        inner: smb,
        trace: Arc::clone(&trace),
    });
    let index = RemoteMediaIndex::new(Arc::new(Fixed(fs)));
    let source_id = SourceId::new();
    let ranges = std::env::var("MOE_HEAVY_RANGES").unwrap_or_default();
    // `MOE_HEAVY_INDEXES`: a file with one list index per line, read instead of (or after) the ranges.
    let mut picked: Vec<usize> = Vec::new();
    for range in ranges.split(',').filter(|r| !r.is_empty()) {
        let (a, b) = range.split_once('-').expect("a-b");
        picked.extend(a.parse::<usize>().unwrap()..b.parse::<usize>().unwrap());
    }
    if let Ok(file) = std::env::var("MOE_HEAVY_INDEXES") {
        picked.extend(
            std::fs::read_to_string(file)
                .unwrap()
                .lines()
                .filter_map(|l| l.trim().parse::<usize>().ok()),
        );
    }
    let pause = Duration::from_millis(
        std::env::var("MOE_HEAVY_PAUSE_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(150),
    );
    let mut out = std::fs::File::create(env_path("MOE_HEAVY_OUT")).unwrap();
    let mut total = 0;
    for i in picked {
        if i >= files.len() {
            continue;
        }
        let (path, size) = &files[i];
        trace.lock().unwrap().clear();
        let result = index.read(&track(source_id, path, *size));
        let taken: Vec<(u64, usize)> = std::mem::take(&mut *trace.lock().unwrap());
        let bytes: usize = taken.iter().map(|t| t.1).sum();
        total += taken.len();
        writeln!(
            out,
            "{i}	{path}	{size}	{}	{bytes}	{}{}",
            taken.len(),
            format_trace(&taken),
            if result.is_err() { "	ERR" } else { "" }
        )
        .unwrap();
        std::thread::sleep(pause);
    }
    println!("total requests {total}");
}

#[test]
#[ignore = "needs MOE_SMB_* and a real server"]
fn copy_heads_and_tails() {
    let Some(settings) = settings() else { return };
    let smb = SmbFs::connect(settings).expect("connect");
    let dir = env_path("MOE_HEAVY_DIR");
    std::fs::create_dir_all(&dir).unwrap();
    let edge: u64 = std::env::var("MOE_HEAVY_EDGE_KIB")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2048)
        * 1024;
    let picks = std::fs::read_to_string(env_path("MOE_HEAVY_PICK")).unwrap();
    let mut map = std::fs::File::create(dir.join("map.txt")).unwrap();
    for (n, line) in picks.lines().enumerate() {
        let path = line.split('\t').next().unwrap();
        // Index lines carry the path in column 2.
        let path = if line.split('\t').count() > 3 {
            line.split('\t').nth(1).unwrap()
        } else {
            path
        };
        let mut file = smb.open(path).expect("open");
        let len = file.len();
        let ext = path.rsplit('.').next().unwrap();
        let local = dir.join(format!("{n}.{ext}"));
        let out = std::fs::File::create(&local).unwrap();
        out.set_len(len).unwrap();
        use std::os::windows::fs::FileExt;
        let mut ranges = vec![(0, edge.min(len))];
        if len > edge {
            ranges.push((len - edge.min(len - edge), edge.min(len - edge)));
        }
        for (start, count) in ranges {
            let mut offset = start;
            while offset < start + count {
                let mut buf = vec![0u8; ((start + count - offset).min(256 * 1024)) as usize];
                let read = file.read_at(offset, &mut buf).expect("read");
                if read == 0 {
                    break;
                }
                out.seek_write(&buf[..read], offset).unwrap();
                offset += read as u64;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        writeln!(map, "{n}\t{path}\t{len}\t{edge}").unwrap();
        println!("copied {n}: {path} ({len} bytes)");
    }
}

#[test]
#[ignore = "needs MOE_HEAVY_DIR"]
fn analyze_local() {
    let dir = env_path("MOE_HEAVY_DIR");
    let trace: Trace = Arc::default();
    let fs: Arc<dyn RemoteFs> = Arc::new(TracingFs {
        inner: Arc::new(DirFs::new(dir.clone())),
        trace: Arc::clone(&trace),
    });
    let index = RemoteMediaIndex::new(Arc::new(Fixed(fs)));
    let source_id = SourceId::new();
    let map = std::fs::read_to_string(dir.join("map.txt")).unwrap();
    for line in map.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        let (n, orig, len, edge): (usize, &str, u64, u64) = (
            cols[0].parse().unwrap(),
            cols[1],
            cols[2].parse().unwrap(),
            cols[3].parse().unwrap(),
        );
        let ext = orig.rsplit('.').next().unwrap();
        let name = format!("{n}.{ext}");
        trace.lock().unwrap().clear();
        let result = index.read(&track(source_id, &name, len));
        let taken: Vec<(u64, usize)> = std::mem::take(&mut *trace.lock().unwrap());
        let bytes: usize = taken.iter().map(|t| t.1).sum();
        let outside = taken
            .iter()
            .filter(|(o, l)| *o + *l as u64 > edge && *o < len.saturating_sub(edge))
            .count();
        println!(
            "#{n} {ext} {} KiB | {} req {} KiB | outside-copied-edges {outside} | {} | {:?}",
            len / 1024,
            taken.len(),
            bytes / 1024,
            format_trace(&taken),
            result
                .as_ref()
                .map(|m| (m.codec.clone(), m.duration_ms))
                .map_err(|e| e.message.clone())
        );
        println!("   {orig}");
    }
}
