//! The disk cache of remote playback (ADR 0026): chunks of the files that were played, kept in a folder under the user's
//! data so a song played again, or a seek back to a place already played, does not go to the server.
//!
//! The limit is a total size in bytes and the eviction is **first in, first out**: when the folder grows past the limit
//! the chunks written longest ago are deleted first, whether or not they were read since. Chunks are written once and
//! never rewritten, so the order of their modification times is the order they came in; the order is rebuilt from the
//! files when the program starts.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::fs::{RemoteError, RemoteFile};

/// Bytes per cached chunk (the playback prefetcher's request size).
pub const CACHE_CHUNK: usize = 1024 * 1024;

/// What names a cached file: the file's identity as the server reports it. A different size or modified time is a
/// different file, so an edited song is never served from stale chunks.
pub fn cache_key(account_id: &str, path: &str, len: u64, modified_ms: Option<i64>) -> String {
    let text = format!("{account_id}\n{path}\n{len}\n{}", modified_ms.unwrap_or(-1));
    Sha256::digest(text.as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// What the cache remembers of a file between plays: its size and modified time as the server last reported them. With
/// it a file whose chunks are on disk is opened without asking the server anything (the answer is checked in the
/// background), so a song played before starts at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileMeta {
    pub len: u64,
    pub modified_ms: Option<i64>,
}

fn path_id(account_id: &str, path: &str) -> String {
    let text = format!("{account_id}\n{path}");
    Sha256::digest(text.as_bytes())
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Entry {
    name: String,
    size: u64,
}

struct Index {
    /// Oldest first.
    order: VecDeque<Entry>,
    sizes: HashMap<String, u64>,
    total: u64,
    limit: u64,
}

pub struct DiskCache {
    dir: PathBuf,
    index: Mutex<Index>,
}

fn chunk_name(key: &str, chunk: u64) -> String {
    format!("{key}-{chunk}")
}

impl DiskCache {
    /// Open (and create) the folder and learn what is already in it, oldest first.
    pub fn open(dir: impl Into<PathBuf>, limit_bytes: u64) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("meta"))?;
        let mut found: Vec<(SystemTime, String, u64)> = Vec::new();
        for item in fs::read_dir(&dir)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            let metadata = item.metadata()?;
            if !metadata.is_file() {
                continue;
            }
            // A leftover of a write that did not finish.
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(item.path());
                continue;
            }
            found.push((
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                name,
                metadata.len(),
            ));
        }
        found.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        let mut index = Index {
            order: VecDeque::new(),
            sizes: HashMap::new(),
            total: 0,
            limit: limit_bytes,
        };
        for (_, name, size) in found {
            index.total += size;
            index.sizes.insert(name.clone(), size);
            index.order.push_back(Entry { name, size });
        }
        let cache = Self {
            dir,
            index: Mutex::new(index),
        };
        cache.evict();
        Ok(cache)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Delete the oldest chunks until the total fits the limit.
    fn evict(&self) {
        let mut index = self.lock();
        while index.total > index.limit {
            let Some(oldest) = index.order.pop_front() else {
                break;
            };
            index.sizes.remove(&oldest.name);
            index.total = index.total.saturating_sub(oldest.size);
            let _ = fs::remove_file(self.dir.join(&oldest.name));
        }
    }

    pub fn limit(&self) -> u64 {
        self.lock().limit
    }

    /// Change the limit; a smaller one evicts at once.
    pub fn set_limit(&self, limit_bytes: u64) {
        self.lock().limit = limit_bytes;
        self.evict();
    }

    /// Bytes in the cache now.
    pub fn used(&self) -> u64 {
        self.lock().total
    }

    /// Delete everything.
    pub fn clear(&self) {
        let mut index = self.lock();
        for entry in index.order.drain(..) {
            let _ = fs::remove_file(self.dir.join(&entry.name));
        }
        index.sizes.clear();
        index.total = 0;
        if let Ok(items) = fs::read_dir(self.dir.join("meta")) {
            for item in items.flatten() {
                let _ = fs::remove_file(item.path());
            }
        }
    }

    /// What was last learned of the file `path` of `account_id` (see [`FileMeta`]).
    pub fn meta(&self, account_id: &str, path: &str) -> Option<FileMeta> {
        let text =
            fs::read_to_string(self.dir.join("meta").join(path_id(account_id, path))).ok()?;
        let mut lines = text.lines();
        let len = lines.next()?.trim().parse().ok()?;
        let modified_ms = match lines.next()?.trim().parse::<i64>().ok()? {
            -1 => None,
            value => Some(value),
        };
        Some(FileMeta { len, modified_ms })
    }

    /// Whether every chunk of the file named by `key` (`len` bytes) is on disk. Answered from the in-memory index, so it
    /// is cheap and follows eviction, a smaller limit and `clear` at once. An empty file is never complete (nothing of it
    /// is ever stored).
    pub fn is_complete_key(&self, key: &str, len: u64) -> bool {
        if len == 0 {
            return false;
        }
        let chunk = CACHE_CHUNK as u64;
        let count = len.div_ceil(chunk);
        let index = self.lock();
        (0..count).all(|number| {
            let expected = (len - number * chunk).min(chunk);
            index.sizes.get(&chunk_name(key, number)) == Some(&expected)
        })
    }

    /// What the cache knows of the file `path` of `account_id` when the whole file is on disk (its size and time are
    /// remembered and every chunk of that version is present); `None` otherwise. A song for which this is `Some` plays
    /// without the server.
    pub fn complete_meta(&self, account_id: &str, path: &str) -> Option<FileMeta> {
        let meta = self.meta(account_id, path)?;
        let key = cache_key(account_id, path, meta.len, meta.modified_ms);
        self.is_complete_key(&key, meta.len).then_some(meta)
    }

    /// Remember the size and modified time of a file. Written to a temporary name and renamed.
    pub fn put_meta(&self, account_id: &str, path: &str, meta: FileMeta) {
        let target = self.dir.join("meta").join(path_id(account_id, path));
        let temporary = target.with_extension("tmp");
        let text = format!("{}\n{}\n", meta.len, meta.modified_ms.unwrap_or(-1));
        if fs::write(&temporary, text).is_err() || fs::rename(&temporary, &target).is_err() {
            let _ = fs::remove_file(&temporary);
        }
    }

    /// The cached chunk, or `None` (not cached, or the file went away).
    pub fn get(&self, key: &str, chunk: u64) -> Option<Vec<u8>> {
        let name = chunk_name(key, chunk);
        if !self.lock().sizes.contains_key(&name) {
            return None;
        }
        fs::read(self.dir.join(name)).ok()
    }

    /// Store a chunk. It is written to a temporary name and renamed, so a chunk is either whole or absent. A chunk
    /// larger than the whole limit is not stored.
    pub fn put(&self, key: &str, chunk: u64, bytes: &[u8]) {
        let size = bytes.len() as u64;
        let name = chunk_name(key, chunk);
        {
            let index = self.lock();
            if size == 0 || size > index.limit || index.sizes.contains_key(&name) {
                return;
            }
        }
        let temporary = self.dir.join(format!("{name}.tmp"));
        let target = self.dir.join(&name);
        if fs::write(&temporary, bytes).is_err() || fs::rename(&temporary, &target).is_err() {
            let _ = fs::remove_file(&temporary);
            return;
        }
        {
            let mut index = self.lock();
            if index.sizes.insert(name.clone(), size).is_none() {
                index.total += size;
                index.order.push_back(Entry { name, size });
            }
        }
        self.evict();
    }
}

/// A remote file read through the cache: every request is served from whole chunks, fetched from the server only when
/// a chunk is not on disk.
pub struct CachedFile {
    inner: Box<dyn RemoteFile>,
    cache: std::sync::Arc<DiskCache>,
    key: String,
    hits: u32,
    misses: u32,
    /// What is fetched is not stored (see [`CachedFile::read_only`]).
    read_only: bool,
}

impl Drop for CachedFile {
    fn drop(&mut self) {
        if self.hits + self.misses > 0 {
            eprintln!(
                "[remote] cache {}: {} chunk(s) from disk, {} from the server",
                &self.key[..self.key.len().min(8)],
                self.hits,
                self.misses
            );
        }
    }
}

impl CachedFile {
    pub fn new(inner: Box<dyn RemoteFile>, cache: std::sync::Arc<DiskCache>, key: String) -> Self {
        Self {
            inner,
            cache,
            key,
            hits: 0,
            misses: 0,
            read_only: false,
        }
    }

    /// As [`new`](Self::new), but what the cache holds is used and what it lacks is fetched without being stored. For the
    /// small reads of a cover or a tag: a song that was played needs no network for them, and the many covers of an album
    /// page do not push the songs that were played out of the cache.
    pub fn read_only(
        inner: Box<dyn RemoteFile>,
        cache: std::sync::Arc<DiskCache>,
        key: String,
    ) -> Self {
        let mut file = Self::new(inner, cache, key);
        file.read_only = true;
        file
    }

    /// Fetch a chunk the cache does not hold (and keep it, unless this file only reads).
    fn fetch_chunk(&mut self, index: u64) -> Result<Vec<u8>, RemoteError> {
        self.misses += 1;
        let len = self.inner.len();
        let start = index * CACHE_CHUNK as u64;
        let wanted = (len.saturating_sub(start)).min(CACHE_CHUNK as u64) as usize;
        let mut buffer = vec![0_u8; wanted];
        let mut filled = 0;
        while filled < wanted {
            let read = self
                .inner
                .read_at(start + filled as u64, &mut buffer[filled..])?;
            if read == 0 {
                return Err(RemoteError::Protocol(
                    "the server's file is shorter than it said".to_owned(),
                ));
            }
            filled += read;
        }
        if !self.read_only {
            self.cache.put(&self.key, index, &buffer);
        }
        Ok(buffer)
    }
}

impl RemoteFile for CachedFile {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let len = self.inner.len();
        if offset >= len || buf.is_empty() {
            return Ok(0);
        }
        let wanted = ((len - offset).min(buf.len() as u64)) as usize;
        let mut done = 0;
        while done < wanted {
            let position = offset + done as u64;
            let index = position / CACHE_CHUNK as u64;
            let inside = (position - index * CACHE_CHUNK as u64) as usize;
            let chunk = match self.cache.get(&self.key, index) {
                Some(bytes) => {
                    self.hits += 1;
                    bytes
                }
                None if self.read_only => {
                    // A tag or a cover is a few kilobytes of a chunk of a megabyte: only the bytes asked for are fetched,
                    // and (as nothing is kept) nothing more.
                    let in_chunk = CACHE_CHUNK - inside;
                    let want = (wanted - done).min(in_chunk);
                    // A reader whose work was cancelled asks the server for nothing more.
                    player_core::check_read_cancel()
                        .map_err(|_| RemoteError::Protocol("the read was cancelled".to_owned()))?;
                    self.misses += 1;
                    let read = self.inner.read_at(position, &mut buf[done..done + want])?;
                    if read == 0 {
                        break;
                    }
                    done += read;
                    continue;
                }
                None => self.fetch_chunk(index)?,
            };
            let count = (chunk.len().saturating_sub(inside)).min(wanted - done);
            if count == 0 {
                break;
            }
            buf[done..done + count].copy_from_slice(&chunk[inside..inside + count]);
            done += count;
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-cache-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    /// A file of `len` bytes (each byte is its offset mod 251) that counts the requests made to it.
    struct Counting {
        len: u64,
        requests: Arc<AtomicU64>,
    }

    impl RemoteFile for Counting {
        fn len(&self) -> u64 {
            self.len
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            self.requests.fetch_add(1, Ordering::SeqCst);
            let n = (self.len.saturating_sub(offset)).min(buf.len() as u64) as usize;
            for (i, byte) in buf[..n].iter_mut().enumerate() {
                *byte = ((offset + i as u64) % 251) as u8;
            }
            Ok(n)
        }
    }

    fn expected(offset: u64, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| ((offset + i as u64) % 251) as u8)
            .collect()
    }

    #[test]
    fn a_chunk_is_stored_once_and_served_from_disk() {
        let dir = temp_dir("serve");
        let cache = DiskCache::open(&dir, 10_000_000).unwrap();
        assert!(cache.get("k", 0).is_none());
        cache.put("k", 0, b"hello");
        assert_eq!(cache.get("k", 0).as_deref(), Some(&b"hello"[..]));
        assert_eq!(cache.used(), 5);
        // A second write of the same chunk changes nothing.
        cache.put("k", 0, b"other");
        assert_eq!(cache.get("k", 0).as_deref(), Some(&b"hello"[..]));
        assert_eq!(cache.used(), 5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_oldest_chunks_go_first_whether_or_not_they_were_read() {
        let dir = temp_dir("fifo");
        let cache = DiskCache::open(&dir, 30).unwrap();
        for (index, byte) in [(0_u64, 1_u8), (1, 2), (2, 3)] {
            cache.put("k", index, &[byte; 10]);
        }
        assert_eq!(cache.used(), 30);
        // Reading the oldest chunk does not save it (first in, first out, not least recently used).
        assert!(cache.get("k", 0).is_some());
        cache.put("k", 3, &[4; 10]);
        assert!(
            cache.get("k", 0).is_none(),
            "chunk 0 came in first and goes first"
        );
        assert!(cache.get("k", 1).is_some());
        assert_eq!(cache.used(), 30);
        cache.put("k", 4, &[5; 20]);
        // 20 bytes more: chunks 1 and 2 go (oldest first).
        assert!(cache.get("k", 1).is_none() && cache.get("k", 2).is_none());
        assert!(cache.get("k", 3).is_some() && cache.get("k", 4).is_some());
        assert_eq!(cache.used(), 30);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_smaller_limit_evicts_at_once_and_clear_empties() {
        let dir = temp_dir("limit");
        let cache = DiskCache::open(&dir, 100).unwrap();
        for index in 0..5 {
            cache.put("k", index, &[7; 20]);
        }
        assert_eq!(cache.used(), 100);
        cache.set_limit(40);
        assert_eq!(cache.used(), 40);
        assert!(cache.get("k", 4).is_some() && cache.get("k", 3).is_some());
        assert!(cache.get("k", 0).is_none());
        // A chunk bigger than the whole limit is not stored.
        cache.put("big", 0, &[1; 41]);
        assert!(cache.get("big", 0).is_none());
        cache.clear();
        assert_eq!(cache.used(), 0);
        // Only the (empty) folder of remembered file sizes is left.
        let left: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|item| item.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("meta")]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_order_and_total_survive_a_restart() {
        let dir = temp_dir("restart");
        {
            let cache = DiskCache::open(&dir, 1_000).unwrap();
            for index in 0..4 {
                cache.put("k", index, &[1; 10]);
                // Distinct modified times, as separate writes minutes apart would have.
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }
        let cache = DiskCache::open(&dir, 25).unwrap();
        // 40 bytes on disk, limit 25: the two oldest were evicted while opening.
        assert_eq!(cache.used(), 20);
        assert!(cache.get("k", 0).is_none() && cache.get("k", 1).is_none());
        assert!(cache.get("k", 2).is_some() && cache.get("k", 3).is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_read_only_cached_file_uses_what_is_cached_and_stores_nothing_new() {
        let dir = temp_dir("read-only");
        let cache = Arc::new(DiskCache::open(&dir, 100_000_000).unwrap());
        let requests = Arc::new(AtomicU64::new(0));
        let len = 3 * CACHE_CHUNK as u64;
        let key = cache_key("a", "x.flac", len, Some(1));
        let counting = || {
            Box::new(Counting {
                len,
                requests: Arc::clone(&requests),
            })
        };
        // The song was played: its first chunk is in the cache.
        let mut played = CachedFile::new(counting(), Arc::clone(&cache), key.clone());
        let mut buffer = vec![0_u8; 1000];
        played.read_at(0, &mut buffer).unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        let used_before = cache.used();

        let mut reader = CachedFile::read_only(counting(), Arc::clone(&cache), key.clone());
        // The cached chunk answers without the server.
        reader.read_at(0, &mut buffer).unwrap();
        assert_eq!(buffer, expected(0, 1000));
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        // A chunk the cache lacks is fetched, correctly, and not stored.
        let at = CACHE_CHUNK as u64 + 10;
        reader.read_at(at, &mut buffer).unwrap();
        assert_eq!(buffer, expected(at, 1000));
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(cache.used(), used_before, "a read-only read stored a chunk");
        reader.read_at(at, &mut buffer).unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 3, "it was fetched again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that records the bytes asked of it.
    struct Recording {
        len: u64,
        asked: Arc<AtomicU64>,
    }

    impl RemoteFile for Recording {
        fn len(&self) -> u64 {
            self.len
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            let count = buf.len().min((self.len - offset) as usize);
            self.asked.fetch_add(count as u64, Ordering::SeqCst);
            buf[..count].fill(7);
            Ok(count)
        }
    }

    #[test]
    fn a_read_only_cached_file_asks_the_server_for_the_bytes_it_needs_not_the_chunk() {
        let dir = temp_dir("read-only-exact");
        let cache = Arc::new(DiskCache::open(&dir, 100_000_000).unwrap());
        let asked = Arc::new(AtomicU64::new(0));
        let len = 30 * CACHE_CHUNK as u64;
        let mut file = CachedFile::read_only(
            Box::new(Recording {
                len,
                asked: Arc::clone(&asked),
            }),
            Arc::clone(&cache),
            cache_key("a", "song.flac", len, Some(1)),
        );
        // A tag at the head, one across a chunk border and one at the very end of a 30 MiB song.
        let mut buffer = vec![0_u8; 16 * 1024];
        assert_eq!(file.read_at(0, &mut buffer).unwrap(), buffer.len());
        assert_eq!(
            file.read_at(CACHE_CHUNK as u64 - 100, &mut buffer).unwrap(),
            buffer.len()
        );
        assert_eq!(file.read_at(len - 128, &mut buffer).unwrap(), 128);
        assert_eq!(
            asked.load(Ordering::SeqCst),
            2 * 16 * 1024 + 128,
            "exactly the bytes asked for, not the megabyte chunks around them"
        );
        assert_eq!(cache.used(), 0, "nothing was kept");

        // A cancelled lookup asks for nothing more.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let before = asked.load(Ordering::SeqCst);
        player_core::with_read_cancel(flag, || {
            assert!(file.read_at(5 * CACHE_CHUNK as u64, &mut buffer).is_err());
        });
        assert_eq!(asked.load(Ordering::SeqCst), before);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cached_file_reads_like_the_file_and_asks_the_server_once_per_chunk() {
        let dir = temp_dir("file");
        let cache = Arc::new(DiskCache::open(&dir, 100_000_000).unwrap());
        let requests = Arc::new(AtomicU64::new(0));
        let len = 3 * CACHE_CHUNK as u64 + 12_345;
        let key = cache_key("a", "x.flac", len, Some(1));
        let mut file = CachedFile::new(
            Box::new(Counting {
                len,
                requests: Arc::clone(&requests),
            }),
            Arc::clone(&cache),
            key.clone(),
        );
        // A read across a chunk border, one in the last (short) chunk, one past the end.
        let mut buffer = vec![0_u8; 4000];
        let at = CACHE_CHUNK as u64 - 2000;
        assert_eq!(file.read_at(at, &mut buffer).unwrap(), 4000);
        assert_eq!(buffer, expected(at, 4000));
        let tail = len - 100;
        let mut small = vec![0_u8; 500];
        assert_eq!(file.read_at(tail, &mut small).unwrap(), 100);
        assert_eq!(&small[..100], &expected(tail, 100)[..]);
        assert_eq!(file.read_at(len, &mut small).unwrap(), 0);
        let before = requests.load(Ordering::SeqCst);
        assert_eq!(before, 3, "chunks 0, 1 and 3 were fetched");

        // The same ranges again, and from a new CachedFile (another play of the song): no server request at all.
        assert_eq!(file.read_at(at, &mut buffer).unwrap(), 4000);
        let mut again = CachedFile::new(
            Box::new(Counting {
                len,
                requests: Arc::clone(&requests),
            }),
            Arc::clone(&cache),
            key,
        );
        assert_eq!(again.read_at(at, &mut buffer).unwrap(), 4000);
        assert_eq!(buffer, expected(at, 4000));
        assert_eq!(again.read_at(tail, &mut small).unwrap(), 100);
        assert_eq!(requests.load(Ordering::SeqCst), before);
        let _ = fs::remove_dir_all(dir);
    }

    /// Plays a file of `len` bytes through the cache so every chunk is stored.
    fn store_whole(cache: &Arc<DiskCache>, key: &str, len: u64) {
        let mut file = CachedFile::new(
            Box::new(Counting {
                len,
                requests: Arc::new(AtomicU64::new(0)),
            }),
            Arc::clone(cache),
            key.to_owned(),
        );
        let mut buffer = vec![0_u8; len as usize];
        assert_eq!(file.read_at(0, &mut buffer).unwrap(), len as usize);
    }

    #[test]
    fn a_file_is_complete_only_when_every_chunk_is_on_disk() {
        let dir = temp_dir("complete");
        let cache = Arc::new(DiskCache::open(&dir, 100_000_000).unwrap());
        let len = 2 * CACHE_CHUNK as u64 + 777;
        let key = cache_key("a", "x.flac", len, Some(5));
        assert!(!cache.is_complete_key(&key, len));
        assert!(!cache.is_complete_key(&key, 0));
        cache.put_meta(
            "a",
            "x.flac",
            FileMeta {
                len,
                modified_ms: Some(5),
            },
        );
        // Only the first chunk: partial.
        cache.put(&key, 0, &expected(0, CACHE_CHUNK));
        assert!(!cache.is_complete_key(&key, len));
        assert_eq!(cache.complete_meta("a", "x.flac"), None);
        store_whole(&cache, &key, len);
        assert!(cache.is_complete_key(&key, len));
        assert_eq!(
            cache.complete_meta("a", "x.flac"),
            Some(FileMeta {
                len,
                modified_ms: Some(5)
            })
        );
        // Another version of the file (the server's size or time changed) is not complete.
        assert!(!cache.is_complete_key(&cache_key("a", "x.flac", len, Some(6)), len));
        cache.put_meta(
            "a",
            "x.flac",
            FileMeta {
                len,
                modified_ms: Some(6),
            },
        );
        assert_eq!(cache.complete_meta("a", "x.flac"), None);
        // A restart keeps the answer.
        cache.put_meta(
            "a",
            "x.flac",
            FileMeta {
                len,
                modified_ms: Some(5),
            },
        );
        drop(cache);
        let again = DiskCache::open(&dir, 100_000_000).unwrap();
        assert!(again.complete_meta("a", "x.flac").is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn evicting_one_chunk_makes_a_cached_file_incomplete_and_clear_resets_it() {
        let dir = temp_dir("complete-evict");
        let len = 3 * CACHE_CHUNK as u64;
        // Room for the file and a little more.
        let cache = Arc::new(DiskCache::open(&dir, len + 10).unwrap());
        let key = cache_key("a", "y.flac", len, Some(1));
        cache.put_meta(
            "a",
            "y.flac",
            FileMeta {
                len,
                modified_ms: Some(1),
            },
        );
        store_whole(&cache, &key, len);
        assert!(cache.complete_meta("a", "y.flac").is_some());
        // Another file's chunk pushes the oldest chunk (0) of this one out (first in, first out).
        cache.put("other", 0, &[1; 20]);
        assert!(cache.get(&key, 0).is_none());
        assert!(!cache.is_complete_key(&key, len));
        assert_eq!(cache.complete_meta("a", "y.flac"), None);

        // A smaller limit does the same, and clear forgets everything.
        let dir2 = temp_dir("complete-limit");
        let cache2 = Arc::new(DiskCache::open(&dir2, 100_000_000).unwrap());
        let key2 = cache_key("a", "z.flac", len, Some(1));
        cache2.put_meta(
            "a",
            "z.flac",
            FileMeta {
                len,
                modified_ms: Some(1),
            },
        );
        store_whole(&cache2, &key2, len);
        assert!(cache2.complete_meta("a", "z.flac").is_some());
        cache2.set_limit(len - 1);
        assert_eq!(cache2.complete_meta("a", "z.flac"), None);
        cache2.set_limit(100_000_000);
        store_whole(&cache2, &key2, len);
        assert!(cache2.complete_meta("a", "z.flac").is_some());
        cache2.clear();
        assert_eq!(cache2.complete_meta("a", "z.flac"), None);
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(dir2);
    }

    #[test]
    fn a_chunk_too_big_for_the_limit_is_never_stored_so_the_file_is_never_complete() {
        let dir = temp_dir("complete-small-limit");
        let len = 2 * CACHE_CHUNK as u64;
        let cache = Arc::new(DiskCache::open(&dir, CACHE_CHUNK as u64 - 1).unwrap());
        let key = cache_key("a", "w.flac", len, Some(1));
        cache.put_meta(
            "a",
            "w.flac",
            FileMeta {
                len,
                modified_ms: Some(1),
            },
        );
        store_whole(&cache, &key, len);
        assert_eq!(cache.complete_meta("a", "w.flac"), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_changed_file_is_another_key() {
        assert_ne!(
            cache_key("a", "x", 10, Some(1)),
            cache_key("a", "x", 10, Some(2))
        );
        assert_ne!(
            cache_key("a", "x", 10, Some(1)),
            cache_key("a", "x", 11, Some(1))
        );
        assert_ne!(
            cache_key("a", "x", 10, Some(1)),
            cache_key("b", "x", 10, Some(1))
        );
        assert_eq!(
            cache_key("a", "x", 10, Some(1)),
            cache_key("a", "x", 10, Some(1))
        );
    }
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn a_files_size_and_time_are_remembered_across_opens() {
        let dir = std::env::temp_dir().join(format!("moe-cache-meta-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let cache = DiskCache::open(&dir, 1_000_000).unwrap();
        assert_eq!(cache.meta("a", "x/y.flac"), None);
        let meta = FileMeta {
            len: 123,
            modified_ms: Some(456),
        };
        cache.put_meta("a", "x/y.flac", meta);
        assert_eq!(cache.meta("a", "x/y.flac"), Some(meta));
        assert_eq!(cache.meta("b", "x/y.flac"), None);
        cache.put_meta(
            "a",
            "x/y.flac",
            FileMeta {
                len: 5,
                modified_ms: None,
            },
        );
        assert_eq!(cache.meta("a", "x/y.flac").unwrap().modified_ms, None);
        // A new cache over the same folder still knows (and the meta files are not counted as chunks).
        let again = DiskCache::open(&dir, 1_000_000).unwrap();
        assert_eq!(again.meta("a", "x/y.flac").unwrap().len, 5);
        assert_eq!(again.used(), 0);
        again.clear();
        assert_eq!(again.meta("a", "x/y.flac"), None);
        let _ = fs::remove_dir_all(&dir);
    }
}
