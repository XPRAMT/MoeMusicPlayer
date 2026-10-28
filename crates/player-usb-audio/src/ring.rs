//! The buffer between Media3's playback thread and the USB streaming thread (ADR 0047): one producer, one consumer,
//! no lock.
//!
//! Three byte counters, all growing: `written` (the producer has copied this many bytes in), `sent` (the consumer has
//! packed this many into URBs; private to the consumer, [`Cursor`]) and `played` (this many came back from the bus in
//! completed URBs). The producer may fill up to `played + capacity`, so bytes that sit in a URB still in flight are
//! never overwritten: when streaming stops (pause) the consumer rewinds `sent` to `played` and the unplayed bytes go
//! out again on resume. The output's position is `played`: the frames the device has actually been sent.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::dsd;
use crate::pcm::InputEncoding;

/// What a native-DSD ring (ADR 0049) does on top of a PCM ring: it drops the start of what Media3 writes (Media3's
/// volume ramp scaled those words) and sends idle fill for it.
struct DsdSide {
    /// Bytes of the first writes still to be replaced by idle fill.
    discard_left: AtomicU64,
}

pub struct Ring {
    buffer: Box<[UnsafeCell<u8>]>,
    encoding: InputEncoding,
    channels: u8,
    frame_bytes: usize,
    written: AtomicU64,
    played: AtomicU64,
    /// Media3 called `stop()`: nothing more comes, an empty ring is the end, not an underrun.
    ended: AtomicBool,
    /// The device failed or was taken away while this ring was attached: the output is stalled.
    failed: AtomicBool,
    underruns: AtomicU64,
    dsd: Option<DsdSide>,
}

// SAFETY: the producer only writes bytes in `[written, played + capacity)` and publishes them with a release store of
// `written`; the consumer only reads bytes in `[sent, written)` after an acquire load of `written`, and frees them with
// a release store of `played` once the bus is done with them. The two regions never overlap.
unsafe impl Sync for Ring {}
unsafe impl Send for Ring {}

impl Ring {
    pub fn new(capacity_frames: usize, encoding: InputEncoding, channels: u8) -> Self {
        let frame_bytes = encoding.bytes() * usize::from(channels.max(1));
        let capacity = capacity_frames.max(1) * frame_bytes;
        Self {
            buffer: (0..capacity).map(|_| UnsafeCell::new(0)).collect(),
            encoding,
            channels,
            frame_bytes,
            written: AtomicU64::new(0),
            played: AtomicU64::new(0),
            ended: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            dsd: None,
        }
    }

    /// A ring for native DSD at `rate` frames per second (ADR 0049). The carrier is 4 channels of 16-bit PCM, 8 bytes
    /// per frame, the same as two 32-bit slots. The first [`dsd::discard_frames`] of what is written are replaced by
    /// idle fill.
    pub fn new_dsd(capacity_frames: usize, rate: u32) -> Self {
        let mut ring = Self::new(capacity_frames, InputEncoding::S16, 4);
        debug_assert_eq!(ring.frame_bytes, dsd::FRAME_BYTES);
        ring.dsd = Some(DsdSide {
            discard_left: AtomicU64::new((dsd::discard_frames(rate) * dsd::FRAME_BYTES) as u64),
        });
        ring
    }

    pub fn is_dsd(&self) -> bool {
        self.dsd.is_some()
    }

    pub fn encoding(&self) -> InputEncoding {
        self.encoding
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    pub fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }

    pub fn capacity_frames(&self) -> usize {
        self.buffer.len() / self.frame_bytes
    }

    /// Producer: copy as many whole frames of `data` as fit; returns the bytes taken (never blocks).
    pub fn push(&self, data: &[u8]) -> usize {
        let written = self.written.load(Ordering::Relaxed);
        let played = self.played.load(Ordering::Acquire);
        let capacity = self.buffer.len() as u64;
        let free = capacity - (written - played);
        let take = (data.len().min(free as usize) / self.frame_bytes) * self.frame_bytes;
        let start = (written % capacity) as usize;
        let first = take.min(self.buffer.len() - start);
        // SAFETY: `[written, written + take)` lies in the producer's region (see the `Sync` impl).
        unsafe {
            let base = self.buffer.as_ptr() as *mut u8;
            std::ptr::copy_nonoverlapping(data.as_ptr(), base.add(start), first);
            std::ptr::copy_nonoverlapping(data.as_ptr().add(first), base, take - first);
        }
        if let Some(side) = &self.dsd {
            let drop = side.discard_left.load(Ordering::Relaxed).min(take as u64);
            if drop > 0 {
                side.discard_left.fetch_sub(drop, Ordering::Relaxed);
                self.fill_range(start, drop as usize, dsd::IDLE_BYTE);
            }
        }
        self.written.store(written + take as u64, Ordering::Release);
        take
    }

    /// Producer: set `count` bytes from ring offset `start` (wrapping) to `byte`, inside the region it just wrote.
    fn fill_range(&self, start: usize, count: usize, byte: u8) {
        let first = count.min(self.buffer.len() - start);
        // SAFETY: the range lies in the producer's region (see the `Sync` impl).
        unsafe {
            let base = self.buffer.as_ptr() as *mut u8;
            std::ptr::write_bytes(base.add(start), byte, first);
            std::ptr::write_bytes(base, byte, count - first);
        }
    }

    pub fn written_frames(&self) -> u64 {
        self.written.load(Ordering::Acquire) / self.frame_bytes as u64
    }

    /// Frames that came back from the bus: the output's position.
    pub fn played_frames(&self) -> u64 {
        self.played.load(Ordering::Acquire) / self.frame_bytes as u64
    }

    pub fn end(&self) {
        self.ended.store(true, Ordering::Release);
    }

    pub fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub fn fail(&self) {
        self.failed.store(true, Ordering::Release);
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Consumer: the ring ran dry while a track played.
    pub fn count_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// The consumer's side of one ring: what has been packed into URBs.
#[derive(Debug, Default)]
pub struct Cursor {
    sent: u64,
}

impl Cursor {
    /// A cursor at the ring's current play position.
    pub fn at(ring: &Ring) -> Self {
        Self {
            sent: ring.played.load(Ordering::Acquire),
        }
    }

    /// Bytes written but not yet packed.
    pub fn unsent(&self, ring: &Ring) -> usize {
        (ring.written.load(Ordering::Acquire) - self.sent) as usize
    }

    /// Copy up to `max` bytes (whole frames) that were not packed yet into `out`; returns how many.
    pub fn take(&mut self, ring: &Ring, max: usize, out: &mut [u8]) -> usize {
        let count =
            (self.unsent(ring).min(max).min(out.len()) / ring.frame_bytes) * ring.frame_bytes;
        let capacity = ring.buffer.len();
        let start = (self.sent % capacity as u64) as usize;
        let first = count.min(capacity - start);
        // SAFETY: `[sent, sent + count)` was published by the producer and is not freed until `complete`.
        unsafe {
            let base = ring.buffer.as_ptr() as *const u8;
            std::ptr::copy_nonoverlapping(base.add(start), out.as_mut_ptr(), first);
            std::ptr::copy_nonoverlapping(base, out.as_mut_ptr().add(first), count - first);
        }
        self.sent += count as u64;
        count
    }

    /// The bus is done with `bytes` more (a completed packet): they count as played and their room is free again.
    pub fn complete(ring: &Ring, bytes: usize) {
        ring.played.fetch_add(bytes as u64, Ordering::Release);
    }

    /// Streaming stopped with URBs discarded: what was packed but never played goes out again.
    pub fn rewind(&mut self, ring: &Ring) {
        self.sent = ring.played.load(Ordering::Acquire);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(frames: usize) -> Ring {
        Ring::new(frames, InputEncoding::S16, 2)
    }

    #[test]
    fn only_whole_frames_fit_and_room_comes_back_when_played() {
        let ring = ring(4);
        assert_eq!(ring.capacity_frames(), 4);
        assert_eq!(ring.push(&[1; 7]), 4);
        assert_eq!(ring.push(&[2; 20]), 12);
        assert_eq!(ring.push(&[3; 4]), 0, "full");
        let mut cursor = Cursor::at(&ring);
        let mut out = [0; 16];
        assert_eq!(cursor.take(&ring, 6, &mut out), 4);
        assert_eq!(&out[..4], &[1; 4]);
        // Taken but not played: still no room.
        assert_eq!(ring.push(&[3; 4]), 0);
        Cursor::complete(&ring, 4);
        assert_eq!(ring.played_frames(), 1);
        assert_eq!(ring.push(&[3; 4]), 4);
        assert_eq!(cursor.take(&ring, 100, &mut out), 16);
        assert_eq!(
            &out[..16],
            &[2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3]
        );
        assert_eq!(ring.written_frames(), 5);
    }

    #[test]
    fn rewinding_sends_the_unplayed_bytes_again() {
        let ring = ring(8);
        let data: Vec<u8> = (0..32).collect();
        assert_eq!(ring.push(&data), 32);
        let mut cursor = Cursor::at(&ring);
        let mut out = [0; 32];
        assert_eq!(cursor.take(&ring, 16, &mut out), 16);
        Cursor::complete(&ring, 8);
        cursor.rewind(&ring);
        assert_eq!(cursor.unsent(&ring), 24);
        assert_eq!(cursor.take(&ring, 32, &mut out), 24);
        assert_eq!(&out[..24], &data[8..]);
    }

    #[test]
    fn the_ring_wraps_around_for_a_long_stream() {
        let ring = Ring::new(5, InputEncoding::S24, 1);
        let mut cursor = Cursor::at(&ring);
        let mut next = 0_u8;
        let mut expect = 0_u8;
        let mut out = [0; 15];
        for round in 0..1000 {
            let chunk: Vec<u8> = (0..(round % 4 + 1) * 3)
                .map(|_| {
                    next = next.wrapping_add(1);
                    next
                })
                .collect();
            let taken = ring.push(&chunk);
            next = next.wrapping_sub((chunk.len() - taken) as u8);
            let count = cursor.take(&ring, 9, &mut out);
            for byte in &out[..count] {
                expect = expect.wrapping_add(1);
                assert_eq!(*byte, expect);
            }
            Cursor::complete(&ring, count);
        }
        assert_eq!(
            ring.played_frames(),
            ring.written_frames() - cursor.unsent(&ring) as u64 / 3
        );
    }

    #[test]
    fn flags_are_seen_by_the_other_side() {
        let ring = ring(2);
        assert!(!ring.is_ended() && !ring.is_failed());
        ring.end();
        ring.fail();
        ring.count_underrun();
        assert!(ring.is_ended() && ring.is_failed());
        assert_eq!(ring.underruns(), 1);
    }

    #[test]
    fn a_dsd_ring_replaces_the_first_25_ms_written_by_idle_fill() {
        let ring = Ring::new_dsd(10_000, 88_200);
        assert!(ring.is_dsd());
        assert_eq!(ring.frame_bytes(), 8);
        assert_eq!(ring.capacity_frames(), 10_000);
        let discard = dsd::discard_frames(88_200) * 8;
        let music = vec![0x11_u8; discard + 80];
        assert_eq!(ring.push(&music), music.len());
        let mut cursor = Cursor::at(&ring);
        let mut got = vec![0_u8; music.len()];
        assert_eq!(cursor.take(&ring, got.len(), &mut got), music.len());
        assert!(got[..discard].iter().all(|&b| b == dsd::IDLE_BYTE));
        assert!(got[discard..].iter().all(|&b| b == 0x11));
        assert!(!got.contains(&0), "no zero word anywhere");
        Cursor::complete(&ring, got.len());
        assert_eq!(ring.played_frames(), (music.len() / 8) as u64);
    }

    #[test]
    fn the_discard_spans_several_writes_and_is_used_up_once() {
        let ring = Ring::new_dsd(100_000, 88_200);
        let discard = dsd::discard_frames(88_200) * 8;
        let mut cursor = Cursor::at(&ring);
        let mut sink = vec![0_u8; 1 << 20];
        // Three writes: the first two stay inside the discard, the third crosses its end.
        let chunk = (discard / 2 / 8 - 1) * 8;
        for _ in 0..3 {
            assert_eq!(ring.push(&vec![0x22; chunk]), chunk);
        }
        let n = cursor.take(&ring, 3 * chunk, &mut sink);
        assert_eq!(n, 3 * chunk);
        assert!(sink[..discard].iter().all(|&b| b == dsd::IDLE_BYTE));
        assert!(sink[discard..n].iter().all(|&b| b == 0x22));
        // Later writes are untouched.
        ring.push(&[0x33; 64]);
        let mut tail = [0_u8; 64];
        assert_eq!(cursor.take(&ring, 64, &mut tail), 64);
        assert!(tail.iter().all(|&b| b == 0x33));
    }

    #[test]
    fn a_discard_that_wraps_the_buffer_end_is_replaced_in_both_parts() {
        // Capacity is smaller than the discard: write and play in pieces so the discard crosses the wrap.
        let ring = Ring::new_dsd(1_000, 88_200);
        let discard = dsd::discard_frames(88_200) * 8;
        let mut cursor = Cursor::at(&ring);
        let mut seen = Vec::new();
        let mut out = vec![0_u8; 4_000];
        let mut written = 0;
        while written < discard + 800 {
            let pushed = ring.push(&[0x55; 2_000]);
            written += pushed;
            let n = cursor.take(&ring, out.len(), &mut out);
            seen.extend_from_slice(&out[..n]);
            Cursor::complete(&ring, n);
        }
        assert!(seen[..discard].iter().all(|&b| b == dsd::IDLE_BYTE));
        assert!(seen[discard..written].iter().all(|&b| b == 0x55));
    }
}
