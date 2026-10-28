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

use crate::pcm::InputEncoding;

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
        }
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
        self.written.store(written + take as u64, Ordering::Release);
        take
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
}
