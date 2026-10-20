//! The remote file abstraction every protocol client implements (ADR 0026), and the cached range reader that turns an
//! opened remote file into the plain `Read + Seek` every decoder, tag reader and the spectrogram already work with.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    /// The server's modified time in UTC Unix milliseconds, when it gave one.
    pub modified_ms: Option<i64>,
}

/// Why a remote operation failed. The kind decides what the user is told and whether a retry makes sense.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    /// The server refused the account name or password.
    Auth,
    NotFound,
    /// The server could not be reached or the link broke (a retry may help).
    Network(String),
    /// The certificate is not trusted.
    Tls(String),
    /// The server answered something this client does not understand.
    Protocol(String),
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth => write!(f, "帳號或密碼不正確，或沒有權限。"),
            Self::NotFound => write!(f, "找不到這個檔案或資料夾。"),
            Self::Network(message) => write!(f, "無法連線到伺服器：{message}"),
            Self::Tls(message) => write!(f, "憑證不受信任：{message}"),
            Self::Protocol(message) => write!(f, "伺服器的回應無法理解：{message}"),
        }
    }
}

impl std::error::Error for RemoteError {}

impl From<RemoteError> for io::Error {
    fn from(error: RemoteError) -> Self {
        let kind = match &error {
            RemoteError::Auth => io::ErrorKind::PermissionDenied,
            RemoteError::NotFound => io::ErrorKind::NotFound,
            RemoteError::Network(_) => io::ErrorKind::ConnectionAborted,
            RemoteError::Tls(_) | RemoteError::Protocol(_) => io::ErrorKind::InvalidData,
        };
        io::Error::new(kind, error)
    }
}

/// An open remote file: its length and ranged reads. One value belongs to one reader (a protocol client may keep a
/// connection inside it); it is moved between threads, never shared.
#[allow(clippy::len_without_is_empty)]
pub trait RemoteFile: Send {
    fn len(&self) -> u64;

    /// Read up to `buf.len()` bytes starting at `offset`; fewer only at the end of the file, 0 past it.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError>;
}

/// A remote file tree. `path` is relative to the account's root, `/`-separated, without a leading slash (`""` is the
/// root).
pub trait RemoteFs: Send + Sync {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError>;
    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError>;
    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError>;

    /// Blocks a sequential reader should fetch per request on this protocol (see [`RangeReader::with_read_ahead`]).
    fn read_ahead_blocks(&self) -> u64 {
        READ_AHEAD_BLOCKS
    }
}

/// A remote file that opens itself again when the link broke under it: a phone that changed network or slept leaves the
/// open handle dead, and every later read on it fails. One failed read of the `Network` kind reconnects (the file system
/// connects again by itself) and is tried once more on a fresh handle; any other error is passed on.
pub struct ReopeningFile {
    fs: std::sync::Arc<dyn RemoteFs>,
    path: String,
    /// `None` until the first read that needs the server (see [`ReopeningFile::lazy`]).
    file: Option<Box<dyn RemoteFile>>,
    len: u64,
}

impl ReopeningFile {
    pub fn new(fs: std::sync::Arc<dyn RemoteFs>, path: String, file: Box<dyn RemoteFile>) -> Self {
        let len = file.len();
        Self { fs, path, file: Some(file), len }
    }

    /// A file whose size is already known and which is not opened until a read needs the server: a song whose chunks are
    /// all in the disk cache never touches the network.
    pub fn lazy(fs: std::sync::Arc<dyn RemoteFs>, path: String, len: u64) -> Self {
        Self { fs, path, file: None, len }
    }

    fn open_now(&mut self) -> Result<&mut Box<dyn RemoteFile>, RemoteError> {
        if self.file.is_none() {
            let fresh = self.fs.open(&self.path)?;
            if fresh.len() != self.len {
                return Err(RemoteError::Protocol("the file changed on the server".to_owned()));
            }
            self.file = Some(fresh);
        }
        Ok(self.file.as_mut().expect("opened above"))
    }
}

impl RemoteFile for ReopeningFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        match self.open_now()?.read_at(offset, buf) {
            Err(RemoteError::Network(_)) => {
                // The link broke under the open handle: open again (the file system connects again by itself).
                self.file = None;
                self.open_now()?.read_at(offset, buf)
            }
            other => other,
        }
    }
}

/// A cached `Read + Seek` over a remote file, with the read-ahead that suits the protocol.
pub fn open_reader(fs: &dyn RemoteFs, path: &str) -> Result<RangeReader, RemoteError> {
    Ok(RangeReader::new(fs.open(path)?).with_read_ahead(fs.read_ahead_blocks()))
}

/// Block size of the range reader's cache.
pub const BLOCK_SIZE: usize = 256 * 1024;
/// Blocks kept in memory (16 MiB).
const DEFAULT_CACHED_BLOCKS: usize = 64;
/// A sequential reader fetches this many blocks per request, so a long track is a few big requests, not thousands of
/// small ones.
const READ_AHEAD_BLOCKS: u64 = 4;

/// `Read + Seek` over a remote file with an in-memory block cache.
///
/// A read that misses the cache fetches the block(s) it needs; when it continues right after the previous read the
/// fetch is [`READ_AHEAD_BLOCKS`] blocks long. Seeking only moves the position (nothing is fetched until a read), and a
/// jump back to a place still in the cache (a decoder re-reading a header or a tag at the end of the file) costs
/// nothing. Memory is bounded: the oldest block is dropped first.
pub struct RangeReader {
    file: Box<dyn RemoteFile>,
    len: u64,
    position: u64,
    block_size: usize,
    capacity: usize,
    blocks: HashMap<u64, Vec<u8>>,
    /// Block numbers, oldest first.
    order: VecDeque<u64>,
    /// The offset right after the last fetch, to recognise a sequential reader.
    next_sequential: u64,
    /// Number of requests made to the server (for tests and diagnostics).
    requests: u64,
    /// Blocks fetched per request by a sequential reader.
    read_ahead: u64,
}

impl RangeReader {
    pub fn new(file: Box<dyn RemoteFile>) -> Self {
        Self::with_geometry(file, BLOCK_SIZE, DEFAULT_CACHED_BLOCKS)
    }

    pub fn with_geometry(file: Box<dyn RemoteFile>, block_size: usize, capacity: usize) -> Self {
        let len = file.len();
        Self {
            file,
            len,
            position: 0,
            block_size: block_size.max(1),
            capacity: capacity.max(READ_AHEAD_BLOCKS as usize),
            blocks: HashMap::new(),
            order: VecDeque::new(),
            next_sequential: 0,
            requests: 0,
            read_ahead: READ_AHEAD_BLOCKS,
        }
    }

    /// Fetch `blocks` blocks per request when reading sequentially. A link with a long round trip (a server on the
    /// internet) needs bigger requests to keep its throughput; the cache grows so a fetch always fits twice.
    pub fn with_read_ahead(mut self, blocks: u64) -> Self {
        self.read_ahead = blocks.max(1);
        self.capacity = self.capacity.max(self.read_ahead as usize * 2);
        self
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn requests(&self) -> u64 {
        self.requests
    }

    fn block_len(&self, block: u64) -> usize {
        let start = block * self.block_size as u64;
        (self.len.saturating_sub(start)).min(self.block_size as u64) as usize
    }

    /// Make sure `block` is cached, fetching it (and read-ahead blocks after it) when it is not.
    fn ensure(&mut self, block: u64) -> io::Result<()> {
        if self.blocks.contains_key(&block) {
            return Ok(());
        }
        let last_block = self.len.saturating_sub(1) / self.block_size as u64;
        let sequential = block * self.block_size as u64 == self.next_sequential && block > 0;
        let count = if sequential { self.read_ahead } else { 1 };
        let count = count.min(last_block + 1 - block);
        let start = block * self.block_size as u64;
        let wanted = (0..count)
            .map(|index| self.block_len(block + index))
            .sum::<usize>();
        let mut buffer = vec![0_u8; wanted];
        let mut filled = 0;
        while filled < wanted {
            let read = self
                .file
                .read_at(start + filled as u64, &mut buffer[filled..])?;
            self.requests += 1;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the server's file is shorter than it said",
                ));
            }
            filled += read;
        }
        let mut offset = 0;
        for index in 0..count {
            let length = self.block_len(block + index);
            self.insert(block + index, buffer[offset..offset + length].to_vec());
            offset += length;
        }
        self.next_sequential = start + wanted as u64;
        Ok(())
    }

    fn insert(&mut self, block: u64, data: Vec<u8>) {
        while self.blocks.len() >= self.capacity {
            match self.order.pop_front() {
                Some(oldest) => {
                    self.blocks.remove(&oldest);
                }
                None => break,
            }
        }
        if self.blocks.insert(block, data).is_none() {
            self.order.push_back(block);
        }
    }
}

impl Read for RangeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.position >= self.len {
            return Ok(0);
        }
        let block = self.position / self.block_size as u64;
        self.ensure(block)?;
        let data = &self.blocks[&block];
        let start = (self.position - block * self.block_size as u64) as usize;
        let count = (data.len() - start).min(out.len());
        out[..count].copy_from_slice(&data[start..start + count]);
        self.position += count as u64;
        Ok(count)
    }
}

impl Seek for RangeReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let next = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        match next {
            Some(position) => {
                self.position = position;
                Ok(position)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of the file",
            )),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    /// A remote file in memory that counts requests and can fail on demand.
    pub struct MemoryFile {
        pub data: Arc<Vec<u8>>,
        pub requests: Arc<AtomicU64>,
        pub fail_after: Option<u64>,
        pub max_per_request: usize,
    }

    impl MemoryFile {
        pub fn new(data: Vec<u8>) -> (Self, Arc<AtomicU64>) {
            let requests = Arc::new(AtomicU64::new(0));
            (
                Self {
                    data: Arc::new(data),
                    requests: Arc::clone(&requests),
                    fail_after: None,
                    max_per_request: usize::MAX,
                },
                requests,
            )
        }
    }

    impl RemoteFile for MemoryFile {
        fn len(&self) -> u64 {
            self.data.len() as u64
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            let made = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail_after.is_some_and(|limit| made > limit) {
                return Err(RemoteError::Network("the link dropped".to_owned()));
            }
            let start = (offset as usize).min(self.data.len());
            let count = buf
                .len()
                .min(self.data.len() - start)
                .min(self.max_per_request);
            buf[..count].copy_from_slice(&self.data[start..start + count]);
            Ok(count)
        }
    }

    pub fn pattern(len: usize) -> Vec<u8> {
        (0..len)
            .map(|index| (index.wrapping_mul(31) ^ (index >> 8)) as u8)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn reads_the_whole_file_identically_whatever_the_chunk_size() {
        let data = pattern(1_000_003);
        for chunk in [1usize, 7, 4096, 65_536, 300_000] {
            let (file, _) = MemoryFile::new(data.clone());
            let mut reader = RangeReader::with_geometry(Box::new(file), 4096, 16);
            let mut out = Vec::new();
            let mut buffer = vec![0u8; chunk];
            loop {
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                out.extend_from_slice(&buffer[..count]);
            }
            assert_eq!(out, data, "chunk {chunk}");
        }
    }

    #[test]
    fn a_sequential_read_is_a_few_big_requests_not_one_per_block() {
        let data = pattern(8 * 1024 * 1024);
        let (file, requests) = MemoryFile::new(data.clone());
        let mut reader = RangeReader::new(Box::new(file));
        let mut sink = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let count = reader.read(&mut sink).unwrap();
            if count == 0 {
                break;
            }
            total += count as u64;
        }
        assert_eq!(total, data.len() as u64);
        // 8 MiB = 32 blocks; the first request is one block, the rest are four blocks each: about 9 requests.
        let made = requests.load(std::sync::atomic::Ordering::SeqCst);
        assert!(made <= 12, "{made} requests for 8 MiB");
    }

    #[test]
    fn a_tag_at_the_end_and_a_seek_back_to_the_start_are_served_from_the_cache() {
        let data = pattern(3 * 1024 * 1024);
        let (file, requests) = MemoryFile::new(data.clone());
        let mut reader = RangeReader::new(Box::new(file));
        let mut head = vec![0u8; 1000];
        reader.read_exact(&mut head).unwrap();
        reader.seek(SeekFrom::End(-500)).unwrap();
        let mut tail = vec![0u8; 500];
        reader.read_exact(&mut tail).unwrap();
        assert_eq!(tail, data[data.len() - 500..]);
        let before = requests.load(std::sync::atomic::Ordering::SeqCst);
        // Back to the start (the first block is still cached) and the tail again: no new requests.
        reader.seek(SeekFrom::Start(0)).unwrap();
        reader.read_exact(&mut head).unwrap();
        assert_eq!(head, data[..1000]);
        reader.seek(SeekFrom::End(-500)).unwrap();
        reader.read_exact(&mut tail).unwrap();
        assert_eq!(requests.load(std::sync::atomic::Ordering::SeqCst), before);
    }

    #[test]
    fn seeking_is_lazy_and_follows_the_standard_rules() {
        let data = pattern(100_000);
        let (file, requests) = MemoryFile::new(data.clone());
        let mut reader = RangeReader::with_geometry(Box::new(file), 1000, 8);
        assert_eq!(reader.seek(SeekFrom::Start(50_000)).unwrap(), 50_000);
        assert_eq!(reader.seek(SeekFrom::Current(-100)).unwrap(), 49_900);
        assert_eq!(reader.seek(SeekFrom::End(0)).unwrap(), 100_000);
        assert_eq!(
            requests.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "seeking fetches nothing"
        );
        let mut byte = [0u8; 1];
        assert_eq!(reader.read(&mut byte).unwrap(), 0, "at the end");
        reader.seek(SeekFrom::Start(200_000)).unwrap();
        assert_eq!(reader.read(&mut byte).unwrap(), 0, "past the end");
        assert!(
            reader.seek(SeekFrom::Current(-300_000)).is_err(),
            "before the start"
        );
        reader.seek(SeekFrom::Start(99_999)).unwrap();
        assert_eq!(reader.read(&mut byte).unwrap(), 1);
        assert_eq!(byte[0], data[99_999]);
    }

    #[test]
    fn memory_stays_bounded_and_evicted_blocks_are_fetched_again() {
        let data = pattern(256 * 1024);
        let (file, requests) = MemoryFile::new(data.clone());
        let mut reader = RangeReader::with_geometry(Box::new(file), 1024, 8);
        let mut byte = [0u8; 1];
        for block in 0..64u64 {
            reader.seek(SeekFrom::Start(block * 4096 + 5)).unwrap();
            reader.read_exact(&mut byte).unwrap();
            assert!(
                reader.blocks.len() <= 8,
                "the cache grew to {}",
                reader.blocks.len()
            );
        }
        let before = requests.load(std::sync::atomic::Ordering::SeqCst);
        reader.seek(SeekFrom::Start(5)).unwrap();
        reader.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], data[5]);
        assert!(
            requests.load(std::sync::atomic::Ordering::SeqCst) > before,
            "block 0 was evicted"
        );
    }

    #[test]
    fn a_server_that_answers_in_small_pieces_and_a_broken_link_are_handled() {
        let data = pattern(50_000);
        let (mut file, _) = MemoryFile::new(data.clone());
        file.max_per_request = 777;
        let mut reader = RangeReader::with_geometry(Box::new(file), 4096, 8);
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, data, "short answers are completed");

        let (mut file, _) = MemoryFile::new(data);
        file.fail_after = Some(2);
        let mut reader = RangeReader::with_geometry(Box::new(file), 4096, 8);
        let error = reader.read_to_end(&mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(error.to_string().contains("無法連線"), "{error}");
    }

    #[test]
    fn an_empty_remote_file_reads_as_empty() {
        let (file, _) = MemoryFile::new(Vec::new());
        let mut reader = RangeReader::new(Box::new(file));
        assert!(reader.is_empty());
        let mut out = Vec::new();
        assert_eq!(reader.read_to_end(&mut out).unwrap(), 0);
    }
}

#[cfg(test)]
mod reopening_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct Bytes {
        data: Vec<u8>,
        /// Reads that still fail with a network error.
        failures: Arc<AtomicUsize>,
    }

    impl RemoteFile for Bytes {
        fn len(&self) -> u64 {
            self.data.len() as u64
        }
        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            if self.failures.load(Ordering::SeqCst) > 0 {
                self.failures.fetch_sub(1, Ordering::SeqCst);
                return Err(RemoteError::Network("broken".to_owned()));
            }
            let start = (offset as usize).min(self.data.len());
            let count = buf.len().min(self.data.len() - start);
            buf[..count].copy_from_slice(&self.data[start..start + count]);
            Ok(count)
        }
    }

    struct Server {
        opens: AtomicUsize,
        len: usize,
    }

    impl RemoteFs for Server {
        fn list(&self, _: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
            Ok(Vec::new())
        }
        fn stat(&self, _: &str) -> Result<RemoteEntry, RemoteError> {
            Err(RemoteError::NotFound)
        }
        fn open(&self, _: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(Bytes {
                data: (0..self.len).map(|n| n as u8).collect(),
                failures: Arc::new(AtomicUsize::new(0)),
            }))
        }
    }

    #[test]
    fn a_broken_link_is_healed_by_opening_the_file_again() {
        let server = Arc::new(Server { opens: AtomicUsize::new(0), len: 100 });
        let dead = Box::new(Bytes {
            data: (0..100).map(|n| n as u8).collect(),
            failures: Arc::new(AtomicUsize::new(1)),
        });
        let mut file = ReopeningFile::new(server.clone(), "a".to_owned(), dead);
        let mut buffer = [0_u8; 4];
        assert_eq!(file.read_at(10, &mut buffer).unwrap(), 4);
        assert_eq!(buffer, [10, 11, 12, 13]);
        assert_eq!(server.opens.load(Ordering::SeqCst), 1);
        // Later reads use the new handle.
        assert_eq!(file.read_at(20, &mut buffer).unwrap(), 4);
        assert_eq!(server.opens.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_lazy_file_opens_only_when_a_read_needs_the_server() {
        let server = Arc::new(Server { opens: AtomicUsize::new(0), len: 100 });
        let mut file = ReopeningFile::lazy(server.clone(), "a".to_owned(), 100);
        assert_eq!(file.len(), 100);
        assert_eq!(server.opens.load(Ordering::SeqCst), 0);
        let mut buffer = [0_u8; 3];
        assert_eq!(file.read_at(5, &mut buffer).unwrap(), 3);
        assert_eq!(buffer, [5, 6, 7]);
        assert_eq!(server.opens.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_file_that_changed_meanwhile_is_not_spliced() {
        let server = Arc::new(Server { opens: AtomicUsize::new(0), len: 50 });
        let dead = Box::new(Bytes {
            data: vec![0; 100],
            failures: Arc::new(AtomicUsize::new(1)),
        });
        let mut file = ReopeningFile::new(server, "a".to_owned(), dead);
        assert!(file.read_at(0, &mut [0_u8; 4]).is_err());
    }
}
