//! Trace the byte ranges the library scan's tag parser reads from local files (ADR 0027).
//!
//! cargo run --release -p player-track-metadata --example trace_tag_reads -- <music-root> [files-per-extension]
//!
//! Set `TRACE_RANGES=1` to also print each file's merged byte ranges (`RANGES <ext> len=<bytes> [(start, end), ...]`).
//! Reads only. Prints one summary line per extension and a JSON line per file.

use std::{
    collections::BTreeMap,
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use player_track_metadata::{read_track_metadata, ByteSource, PathSource, ReadSeek};

#[derive(Default)]
struct Trace {
    opens: u64,
    reads: u64,
    seeks: u64,
    bytes: u64,
    /// Requested ranges (start, end), merged afterwards.
    ranges: Vec<(u64, u64)>,
}

struct Counting<R> {
    inner: R,
    position: u64,
    trace: Arc<Mutex<Trace>>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        let mut trace = self.trace.lock().unwrap();
        trace.reads += 1;
        trace.bytes += n as u64;
        if n > 0 {
            trace.ranges.push((self.position, self.position + n as u64));
        }
        self.position += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for Counting<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = self.inner.seek(pos)?;
        if to != self.position {
            self.trace.lock().unwrap().seeks += 1;
        }
        self.position = to;
        Ok(to)
    }
}

/// Wraps the file a source opens, counting what the parser asks of the disk.
struct Traced<S> {
    inner: S,
    path: PathBuf,
    trace: Arc<Mutex<Trace>>,
}

impl<S: ByteSource> ByteSource for Traced<S> {
    fn file_name(&self) -> String {
        self.inner.file_name()
    }
    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        self.trace.lock().unwrap().opens += 1;
        let _ = &self.path;
        Ok(Box::new(Counting { inner: self.inner.open()?, position: 0, trace: Arc::clone(&self.trace) }))
    }
    fn open_correction(&self) -> Option<Box<dyn ReadSeek>> {
        self.inner.open_correction()
    }
}

/// Distinct regions after merging ranges that touch or lie within `gap` bytes: each region is one disk seek when cold.
fn regions(ranges: &mut Vec<(u64, u64)>, gap: u64) -> (usize, u64, u64) {
    ranges.sort_unstable();
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for &(start, end) in ranges.iter() {
        match merged.last_mut() {
            Some(last) if start <= last.1 + gap => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    let unique: u64 = merged.iter().map(|(s, e)| e - s).sum();
    let max_end = merged.iter().map(|r| r.1).max().unwrap_or(0);
    (merged.len(), unique, max_end)
}

fn walk(root: &Path, out: &mut BTreeMap<String, Vec<PathBuf>>, per_ext: usize) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, per_ext);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext = ext.to_ascii_lowercase();
            if matches!(ext.as_str(), "flac" | "mp3" | "m4a" | "dsf" | "opus" | "wav" | "wv" | "ogg" | "mka" | "aac" | "ape" | "dff") {
                let list = out.entry(ext).or_default();
                if list.len() < per_ext {
                    list.push(path);
                }
            }
        }
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(args.next().expect("music root"));
    let per_ext: usize = args.next().map(|v| v.parse().unwrap()).unwrap_or(30);
    let mode = "file";
    let mut files = BTreeMap::new();
    walk(&root, &mut files, per_ext);
    for (ext, paths) in &files {
        let (mut n, mut opens, mut reads, mut seeks, mut bytes, mut regs, mut regs64, mut unique, mut errors) =
            (0u64, 0, 0, 0, 0, 0usize, 0usize, 0, 0);
        let mut bytes_list = Vec::new();
        for path in paths {
            let trace = Arc::new(Mutex::new(Trace::default()));
            let result = read_track_metadata(&Traced { inner: PathSource(path), path: path.clone(), trace: Arc::clone(&trace) });
            let mut t = trace.lock().unwrap();
            let (r0, u, max_end) = regions(&mut t.ranges.clone(), 0);
            if std::env::var_os("TRACE_RANGES").is_some() { let mut m = t.ranges.clone(); m.sort_unstable(); let mut g: Vec<(u64,u64)> = Vec::new(); for (s,e) in m { match g.last_mut() { Some(l) if s <= l.1 => l.1 = l.1.max(e), _ => g.push((s,e)) } } println!("RANGES {ext} len={} {:?}", std::fs::metadata(path).map(|m| m.len()).unwrap_or(0), g); }
            let (r64, _, _) = regions(&mut t.ranges, 64 * 1024);
            n += 1;
            opens += t.opens;
            reads += t.reads;
            seeks += t.seeks;
            bytes += t.bytes;
            regs += r0;
            regs64 += r64;
            unique += u;
            bytes_list.push(t.bytes);
            if result.is_err() {
                errors += 1;
            }
            println!(
                "{{\"ext\":\"{ext}\",\"opens\":{},\"reads\":{},\"seeks\":{},\"bytes\":{},\"unique\":{u},\"regions\":{r0},\"regions_gap64k\":{r64},\"max_offset\":{max_end},\"ok\":{},\"path\":{:?}}}",
                t.opens, t.reads, t.seeks, t.bytes, result.is_ok(), path.display().to_string()
            );
        }
        bytes_list.sort_unstable();
        let f = n.max(1) as f64;
        eprintln!(
            "SUMMARY {mode} {ext:5} files={n} opens={:.1} reads={:.1} seeks={:.1} bytes_kb={:.1} (p50 {:.1}, max {:.1}) unique_kb={:.1} regions={:.1} regions_gap64k={:.1} errors={errors}",
            opens as f64 / f, reads as f64 / f, seeks as f64 / f, bytes as f64 / f / 1024.0,
            bytes_list[bytes_list.len() / 2] as f64 / 1024.0, *bytes_list.last().unwrap() as f64 / 1024.0,
            unique as f64 / f / 1024.0, regs as f64 / f, regs64 as f64 / f,
        );
    }
}
