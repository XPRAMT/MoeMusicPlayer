//! A `Read + Seek` over a remote file that keeps reading ahead on a background thread (ADR 0026).
//!
//! Playback pulls samples from a decoder inside the audio thread, and the decoder reads its file synchronously. With a
//! plain [`RangeReader`](crate::fs::RangeReader) every read that misses the cache waits for the network, a stall of a
//! round trip or more that is heard as a gap. This reader fetches the file in chunks ahead of the position on its own
//! thread, so a sequential reader always finds its bytes already here; only a seek to a place that is not buffered waits
//! (and the audio output is expected to be paused or buffering around a seek anyway).
//!
//! When the link is lost (a [`RemoteError::Network`] failure) the thread does not give up: it keeps what it has, waits
//! ([`RETRY_BACKOFF`]) and fetches the same place again (the file underneath reopens itself and the file system
//! connects again), until it works or [`RETRY_GIVE_UP`] has passed since the outage began. Only then does a read fail,
//! with a message starting with [`player_core::REMOTE_LINK_LOST`], which the players take as "stop here" rather than
//! "the track ended". Any other error (the file changed, it was released, the server refused) fails at once. A seek
//! cuts the wait short.

use std::collections::VecDeque;
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::fs::{RemoteError, RemoteFile};

/// One request to the server.
pub const PREFETCH_CHUNK: usize = 1024 * 1024;
/// How far ahead of the reader the thread fetches.
pub const PREFETCH_AHEAD: u64 = 16 * 1024 * 1024;
/// Bytes kept behind the reader, so a decoder that steps back a little (re-reading a frame header) does not refetch.
const HISTORY: u64 = 2 * 1024 * 1024;
/// Waits before fetching again after the link was lost; the last one repeats. The first retry is at once (a dropped
/// connection on a working network is back within one connect); later ones back off so a phone without a network does
/// not spin, capped so a link that returns is used within ten seconds.
pub const RETRY_BACKOFF: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(10),
];
/// How long after an outage began the thread stops retrying (a router or server restart is back well within this).
pub const RETRY_GIVE_UP: Duration = Duration::from_secs(180);
/// A reader never waits longer than this for one read, whatever the fetch thread does: the give-up time, plus one
/// read that may run its full limit, plus a margin.
const READER_LIMIT: Duration = Duration::from_secs(180 + 120 + 60);

/// When to retry after the link is lost and when to give up.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub backoff: Vec<Duration>,
    pub give_up_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            backoff: RETRY_BACKOFF.to_vec(),
            give_up_after: RETRY_GIVE_UP,
        }
    }
}

impl RetryPolicy {
    fn delay(&self, attempt: usize) -> Duration {
        self.backoff
            .get(attempt)
            .or(self.backoff.last())
            .copied()
            .unwrap_or(Duration::ZERO)
    }
}

/// The link was lost while fetching: since when, and how many retries so far.
struct Outage {
    since: Instant,
    attempts: usize,
}

struct State {
    /// Contiguous chunks; the first starts at `start`, together they end at `end`.
    chunks: VecDeque<Vec<u8>>,
    start: u64,
    end: u64,
    /// Where the reader is (the fetch thread keeps `PREFETCH_AHEAD` bytes beyond it).
    reader_position: u64,
    /// Incremented whenever the buffer is thrown away for a seek, so a fetch that was in flight is discarded.
    generation: u64,
    error: Option<RemoteError>,
    /// Retrying stopped: the message the reader fails with (starts with `REMOTE_LINK_LOST`).
    gave_up: Option<String>,
    outage: Option<Outage>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    /// The reader moved or the buffer drained: the fetch thread may have work.
    worker: Condvar,
    /// A chunk arrived or the fetch failed: the reader may proceed.
    reader: Condvar,
    len: u64,
    /// When the request now in flight began (`Instant` offset in ms from `born`); 0 while the fetch thread is not inside
    /// a request. Read by a stalled reader to say what the fetch thread is doing.
    in_flight_since_ms: std::sync::atomic::AtomicU64,
    born: std::time::Instant,
    reader_limit: Duration,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct PrefetchReader {
    shared: Arc<Shared>,
    position: u64,
}

impl PrefetchReader {
    pub fn new(file: Box<dyn RemoteFile>) -> Self {
        Self::with_geometry(file, PREFETCH_CHUNK, PREFETCH_AHEAD)
    }

    pub fn with_geometry(file: Box<dyn RemoteFile>, chunk: usize, ahead: u64) -> Self {
        Self::with_policy(file, chunk, ahead, RetryPolicy::default())
    }

    pub fn with_policy(file: Box<dyn RemoteFile>, chunk: usize, ahead: u64, policy: RetryPolicy) -> Self {
        let len = file.len();
        let reader_limit = if policy.give_up_after >= RETRY_GIVE_UP {
            READER_LIMIT
        } else {
            policy.give_up_after + Duration::from_secs(5)
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                chunks: VecDeque::new(),
                start: 0,
                end: 0,
                reader_position: 0,
                generation: 0,
                error: None,
                gave_up: None,
                outage: None,
                stopped: false,
            }),
            worker: Condvar::new(),
            reader: Condvar::new(),
            len,
            in_flight_since_ms: std::sync::atomic::AtomicU64::new(0),
            born: std::time::Instant::now(),
            reader_limit,
        });
        let thread_shared = Arc::clone(&shared);
        // Detached: it ends by itself once the reader is dropped (after the request in flight returns).
        let _ = std::thread::Builder::new()
            .name("moe-remote-prefetch".to_owned())
            .spawn(move || fetch_loop(thread_shared, file, chunk.max(1), ahead.max(1), policy));
        Self {
            shared,
            position: 0,
        }
    }

    pub fn len(&self) -> u64 {
        self.shared.len
    }

    pub fn is_empty(&self) -> bool {
        self.shared.len == 0
    }
}

impl Drop for PrefetchReader {
    fn drop(&mut self) {
        lock(&self.shared.state).stopped = true;
        self.shared.worker.notify_all();
    }
}

fn fetch_loop(
    shared: Arc<Shared>,
    mut file: Box<dyn RemoteFile>,
    chunk: usize,
    ahead: u64,
    policy: RetryPolicy,
) {
    loop {
        // Wait until there is something to fetch.
        let (offset, generation) = {
            let mut state = lock(&shared.state);
            loop {
                if state.stopped {
                    return;
                }
                let wanted = state.error.is_none()
                    && state.gave_up.is_none()
                    && state.end < shared.len
                    && state.end.saturating_sub(state.reader_position) < ahead;
                if wanted {
                    break (state.end, state.generation);
                }
                state = shared
                    .worker
                    .wait_timeout(state, Duration::from_millis(500))
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
            }
        };
        let wanted = (shared.len - offset).min(chunk as u64) as usize;
        shared.in_flight_since_ms.store(
            shared.born.elapsed().as_millis() as u64 + 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        let mut buffer = vec![0_u8; wanted];
        let mut filled = 0;
        let mut failure = None;
        while filled < wanted {
            match file.read_at(offset + filled as u64, &mut buffer[filled..]) {
                Ok(0) => {
                    failure = Some(RemoteError::Protocol(
                        "the server's file is shorter than it said".to_owned(),
                    ));
                    break;
                }
                Ok(read) => filled += read,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        shared
            .in_flight_since_ms
            .store(0, std::sync::atomic::Ordering::Relaxed);
        let mut state = lock(&shared.state);
        if state.stopped {
            return;
        }
        if state.generation != generation {
            // The reader jumped while this was in flight.
            continue;
        }
        if filled > 0 {
            buffer.truncate(filled);
            state.end += filled as u64;
            state.chunks.push_back(buffer);
        }
        match failure {
            None => {
                if let Some(outage) = state.outage.take() {
                    eprintln!(
                        "[remote] link back after {} ms and {} retr{}; reading on at {}",
                        outage.since.elapsed().as_millis(),
                        outage.attempts,
                        if outage.attempts == 1 { "y" } else { "ies" },
                        offset
                    );
                }
            }
            Some(error @ RemoteError::Network(_)) => {
                let outage = state.outage.get_or_insert_with(|| Outage {
                    since: Instant::now(),
                    attempts: 0,
                });
                let lasted = outage.since.elapsed();
                if lasted >= policy.give_up_after {
                    eprintln!(
                        "[remote] giving up at {offset} after {} s and {} retries: {error}",
                        lasted.as_secs(),
                        outage.attempts
                    );
                    state.gave_up = Some(format!(
                        "{}（{} 秒內重試 {} 次都失敗：{error}）",
                        player_core::REMOTE_LINK_LOST,
                        lasted.as_secs(),
                        outage.attempts
                    ));
                } else {
                    let delay = policy.delay(outage.attempts);
                    outage.attempts += 1;
                    eprintln!(
                        "[remote] read at {offset} failed ({error}); retry {} in {} ms",
                        outage.attempts,
                        delay.as_millis()
                    );
                    // Wait, unless the reader is dropped or seeks (a seek retries at once, at the new place).
                    let deadline = Instant::now() + delay;
                    loop {
                        if state.stopped {
                            return;
                        }
                        let now = Instant::now();
                        if state.generation != generation || now >= deadline {
                            break;
                        }
                        state = shared
                            .worker
                            .wait_timeout(state, deadline - now)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                }
            }
            Some(error) => state.error = Some(error),
        }
        shared.reader.notify_all();
    }
}

impl State {
    /// Drop chunks that lie wholly before the history window.
    fn trim(&mut self) {
        let keep_from = self.reader_position.saturating_sub(HISTORY);
        while let Some(front) = self.chunks.front() {
            let front_end = self.start + front.len() as u64;
            if front_end <= keep_from {
                self.start = front_end;
                self.chunks.pop_front();
            } else {
                break;
            }
        }
    }

    /// Copy from the buffer at `position`.
    fn copy_out(&self, position: u64, out: &mut [u8]) -> usize {
        let mut chunk_start = self.start;
        let mut written = 0;
        for chunk in &self.chunks {
            let chunk_end = chunk_start + chunk.len() as u64;
            let wanted = position + written as u64;
            if wanted < chunk_end && wanted >= chunk_start {
                let from = (wanted - chunk_start) as usize;
                let count = (chunk.len() - from).min(out.len() - written);
                out[written..written + count].copy_from_slice(&chunk[from..from + count]);
                written += count;
                if written == out.len() {
                    break;
                }
            }
            chunk_start = chunk_end;
        }
        written
    }

    fn reset_to(&mut self, position: u64) {
        self.chunks.clear();
        self.start = position;
        self.end = position;
        self.generation += 1;
        self.error = None;
        // A seek after giving up tries again, with a fresh allowance.
        if self.gave_up.take().is_some() {
            self.outage = None;
        }
    }
}

impl Read for PrefetchReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.position >= self.shared.len {
            return Ok(0);
        }
        let mut state = lock(&self.shared.state);
        let waiting_since = std::time::Instant::now();
        let mut next_report = Duration::from_secs(3);
        loop {
            if self.position >= state.start && self.position < state.end {
                let count = state.copy_out(self.position, out);
                self.position += count as u64;
                state.reader_position = self.position;
                state.trim();
                drop(state);
                self.shared.worker.notify_all();
                return Ok(count);
            }
            if self.position != state.end || self.position < state.start {
                // Not buffered and not the very next byte: a seek. Fetch from here.
                state.reset_to(self.position);
                state.reader_position = self.position;
                self.shared.worker.notify_all();
            }
            if let Some(error) = state.error.clone() {
                return Err(error.into());
            }
            if let Some(message) = state.gave_up.clone() {
                return Err(io::Error::new(io::ErrorKind::ConnectionAborted, message));
            }
            let waited = waiting_since.elapsed();
            if waited >= next_report {
                let in_flight = self.shared.in_flight_since_ms.load(std::sync::atomic::Ordering::Relaxed);
                let in_flight_for = if in_flight == 0 {
                    None
                } else {
                    Some((self.shared.born.elapsed().as_millis() as u64 + 1).saturating_sub(in_flight))
                };
                eprintln!(
                    "[remote] prefetch stall: waited {} ms at {} of {}; buffered {}..{}; request in flight for {:?} ms; retries {:?}",
                    waited.as_millis(),
                    self.position,
                    self.shared.len,
                    state.start,
                    state.end,
                    in_flight_for,
                    state.outage.as_ref().map(|outage| outage.attempts),
                );
                next_report += Duration::from_secs(3);
                // Nothing is being fetched although the reader waits: wake the fetch thread (a lost wake-up would
                // otherwise wait for its own timeout forever).
                if in_flight_for.is_none() {
                    self.shared.worker.notify_all();
                }
            }
            // Normally the fetch thread gives up first (and says why); this only guards against a thread that is gone.
            if waited >= self.shared.reader_limit {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    format!(
                        "{}（{} 秒沒有收到檔案的下一段）",
                        player_core::REMOTE_LINK_LOST,
                        waited.as_secs()
                    ),
                ));
            }
            state = self
                .shared
                .reader
                .wait_timeout(state, Duration::from_millis(250))
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Seek for PrefetchReader {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let next = match target {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.shared.len.checked_add_signed(delta),
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    /// A file of `len` bytes (each byte is its offset mod 251) whose every request takes `latency`.
    struct SlowFile {
        len: u64,
        latency: Duration,
        requests: Arc<AtomicU64>,
        /// Given the request number (from 0), the error that request fails with, if any.
        fail: Box<dyn Fn(u64) -> Option<RemoteError> + Send>,
    }

    impl RemoteFile for SlowFile {
        fn len(&self) -> u64 {
            self.len
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            let count = self.requests.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(self.latency);
            if let Some(error) = (self.fail)(count) {
                return Err(error);
            }
            if offset >= self.len {
                return Ok(0);
            }
            let n = (self.len - offset).min(buf.len() as u64) as usize;
            for (index, byte) in buf[..n].iter_mut().enumerate() {
                *byte = ((offset + index as u64) % 251) as u8;
            }
            Ok(n)
        }
    }

    fn expected(offset: u64, len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| ((offset + i as u64) % 251) as u8)
            .collect()
    }

    fn reader(
        len: u64,
        latency_ms: u64,
        fail_after: Option<u64>,
    ) -> (PrefetchReader, Arc<AtomicU64>) {
        failing_reader(
            len,
            latency_ms,
            move |count| {
                fail_after
                    .is_some_and(|limit| count >= limit)
                    .then(|| RemoteError::Network("link dropped".to_owned()))
            },
            RetryPolicy::default(),
        )
    }

    fn failing_reader(
        len: u64,
        latency_ms: u64,
        fail: impl Fn(u64) -> Option<RemoteError> + Send + 'static,
        policy: RetryPolicy,
    ) -> (PrefetchReader, Arc<AtomicU64>) {
        let requests = Arc::new(AtomicU64::new(0));
        let file = SlowFile {
            len,
            latency: Duration::from_millis(latency_ms),
            requests: Arc::clone(&requests),
            fail: Box::new(fail),
        };
        (
            PrefetchReader::with_policy(Box::new(file), 64 * 1024, 512 * 1024, policy),
            requests,
        )
    }

    fn quick_policy(backoff_ms: &[u64], give_up_ms: u64) -> RetryPolicy {
        RetryPolicy {
            backoff: backoff_ms.iter().map(|&ms| Duration::from_millis(ms)).collect(),
            give_up_after: Duration::from_millis(give_up_ms),
        }
    }

    fn network() -> Option<RemoteError> {
        Some(RemoteError::Network("link dropped".to_owned()))
    }

    #[test]
    fn reads_the_whole_file_in_order_and_ends() {
        let (mut reader, _) = reader(1_000_003, 0, None);
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 1_000_003);
        assert_eq!(all, expected(0, 1_000_003));
        let mut byte = [0_u8; 1];
        assert_eq!(reader.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn seeks_anywhere_and_reads_the_right_bytes() {
        let (mut reader, _) = reader(2_000_000, 0, None);
        let mut buffer = vec![0_u8; 5000];
        // Jumps forward, back (into the history and before it), to the end, and sequential reads after a jump.
        for &offset in &[1_500_000_u64, 10, 1_999_000, 700_000, 700_100, 0, 1_234_567] {
            reader.seek(SeekFrom::Start(offset)).unwrap();
            reader
                .read_exact(&mut buffer[..4000.min((2_000_000 - offset) as usize)])
                .unwrap();
            let n = 4000.min((2_000_000 - offset) as usize);
            assert_eq!(&buffer[..n], &expected(offset, n)[..], "at {offset}");
        }
        assert_eq!(reader.seek(SeekFrom::End(-10)).unwrap(), 1_999_990);
        assert_eq!(reader.seek(SeekFrom::Current(5)).unwrap(), 1_999_995);
        assert!(reader.seek(SeekFrom::Current(-3_000_000)).is_err());
    }

    /// The point of the type: a reader that consumes at a steady pace never waits for a slow server once it has started.
    #[test]
    fn a_steady_reader_is_not_stalled_by_a_slow_server() {
        // 40 ms per request of 64 KiB is ~1.6 MB/s from the server; the reader takes 1 MB/s.
        let (mut reader, _) = reader(3_000_000, 40, None);
        let mut buffer = vec![0_u8; 10_000];
        // The first read waits for the first chunk (and is allowed to).
        reader.read_exact(&mut buffer).unwrap();
        let mut worst = Duration::ZERO;
        for _ in 0..200 {
            std::thread::sleep(Duration::from_millis(10));
            let started = Instant::now();
            reader.read_exact(&mut buffer).unwrap();
            worst = worst.max(started.elapsed());
        }
        assert!(
            worst < Duration::from_millis(30),
            "a read waited {worst:?} although the server is faster than the reader"
        );
    }

    #[test]
    fn the_thread_stops_fetching_far_ahead_and_resumes_as_the_reader_moves() {
        let (mut reader, requests) = reader(50_000_000, 0, None);
        let mut buffer = vec![0_u8; 1000];
        reader.read_exact(&mut buffer).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        // 512 KiB ahead in 64 KiB chunks, a few more for the history/rounding: not the whole 50 MB.
        let early = requests.load(Ordering::SeqCst);
        assert!(early <= 16, "{early} requests with nothing consuming");
        reader.seek(SeekFrom::Start(400_000)).unwrap();
        reader.read_exact(&mut buffer).unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(requests.load(Ordering::SeqCst) > early);
    }

    #[test]
    fn a_link_that_stays_lost_is_an_error_after_the_buffered_bytes_and_the_retries() {
        // From the 4th request on every one fails: three chunks (192 KiB) are readable, then (after retrying for the
        // allowed time) the "link lost" error.
        let started = Instant::now();
        let (mut reader, requests) = failing_reader(
            10_000_000,
            0,
            |count| if count >= 3 { network() } else { None },
            quick_policy(&[0, 20, 40], 400),
        );
        let mut all = Vec::new();
        let error = reader.read_to_end(&mut all).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
        assert!(player_core::is_remote_link_lost(&error.to_string()), "{error}");
        assert_eq!(all.len(), 3 * 64 * 1024);
        assert_eq!(all, expected(0, all.len()));
        assert!(started.elapsed() >= Duration::from_millis(400), "{:?}", started.elapsed());
        // 0, 20, 40, 40, … ms apart over 400 ms: several retries, not one and not hundreds.
        let tries = requests.load(Ordering::SeqCst) - 3;
        assert!((5..=30).contains(&tries), "{tries} tries");
    }

    #[test]
    fn a_link_that_comes_back_is_read_on_without_a_gap_or_a_repeat() {
        // Requests 5 to 9 fail (an outage of five tries), then the server answers again.
        let (mut reader, requests) = failing_reader(
            3_000_000,
            1,
            |count| if (5..10).contains(&count) { network() } else { None },
            quick_policy(&[0, 10, 20, 40], 10_000),
        );
        let mut all = Vec::new();
        reader.read_to_end(&mut all).unwrap();
        assert_eq!(all.len(), 3_000_000);
        assert_eq!(all, expected(0, all.len()));
        assert!(requests.load(Ordering::SeqCst) >= 10);
    }

    #[test]
    fn the_retries_wait_longer_each_time_up_to_the_last_step() {
        let policy = quick_policy(&[0, 1000, 2000, 4000, 8000, 10_000], 1);
        let waits: Vec<u64> = (0..8).map(|attempt| policy.delay(attempt).as_millis() as u64).collect();
        assert_eq!(waits, [0, 1000, 2000, 4000, 8000, 10_000, 10_000, 10_000]);
        assert_eq!(RetryPolicy::default().backoff, RETRY_BACKOFF.to_vec());
        assert_eq!(RetryPolicy::default().give_up_after, Duration::from_secs(180));
    }

    #[test]
    fn an_error_that_is_not_the_network_fails_at_once() {
        let started = Instant::now();
        let (mut reader, requests) = failing_reader(
            10_000_000,
            0,
            |count| (count >= 2).then_some(RemoteError::Changed),
            quick_policy(&[1000], 60_000),
        );
        let mut all = Vec::new();
        let error = reader.read_to_end(&mut all).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(!player_core::is_remote_link_lost(&error.to_string()));
        assert_eq!(all.len(), 2 * 64 * 1024);
        assert!(started.elapsed() < Duration::from_millis(900));
        assert_eq!(requests.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_seek_cuts_the_wait_between_retries_short() {
        // Request 2 fails and the next try would only come after 30 s; the reader seeks away meanwhile.
        let (mut reader, _) = failing_reader(
            10_000_000,
            0,
            |count| if count == 2 { network() } else { None },
            quick_policy(&[30_000], 120_000),
        );
        let mut buffer = vec![0_u8; 64 * 1024];
        reader.read_exact(&mut buffer).unwrap();
        // Let the failure happen and the thread start waiting.
        std::thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        reader.seek(SeekFrom::Start(5_000_000)).unwrap();
        reader.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, expected(5_000_000, buffer.len()));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    }

    #[test]
    fn a_seek_after_giving_up_tries_again() {
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let flag = Arc::clone(&fail);
        let (mut reader, _) = failing_reader(
            1_000_000,
            0,
            move |count| (count >= 1 && flag.load(Ordering::SeqCst)).then(|| RemoteError::Network("down".into())),
            quick_policy(&[0, 10], 100),
        );
        let mut all = Vec::new();
        assert!(reader.read_to_end(&mut all).is_err());
        fail.store(false, Ordering::SeqCst);
        reader.seek(SeekFrom::Start(500_000)).unwrap();
        let mut buffer = vec![0_u8; 1000];
        reader.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, expected(500_000, 1000));
    }
}
