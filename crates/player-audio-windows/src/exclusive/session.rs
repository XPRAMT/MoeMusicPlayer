//! One open exclusive stream and the two threads around it: a decode thread that fills a sample ring, and a render
//! thread that moves the ring's samples into the endpoint buffer when the device asks (ADR 0020). The ring is the only
//! thing the two share; it carries a generation number so a seek or a new track can throw away what was queued without
//! racing the decode thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::endpoint::{Endpoint, RenderStream};
use super::format::{encode, negotiate, silence, Negotiated, StreamFormat};

/// How much audio the ring holds (the decode thread stays this far ahead of the device).
const RING_SECONDS: f64 = 0.4;
/// How long the starter waits for the endpoint to negotiate and open.
const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
/// How long closing a session waits for its threads (and so for the device to be released).
const CLOSE_WAIT: Duration = Duration::from_millis(1500);
/// How often the render thread looks at the stop flag while the device is quiet.
const WAIT_SLICE: Duration = Duration::from_millis(100);
/// Gain changes ease in over about 500 samples, like the shared path's.
const GAIN_EASE: f32 = 0.002;

enum Push {
    Accepted,
    Full,
    /// The ring has moved to a newer generation; the caller's source is obsolete.
    Stale,
}

struct RingState {
    data: VecDeque<f32>,
    generation: u64,
    /// The current generation's source ended and everything it produced was queued.
    done: bool,
    /// A source has been started and has queued its first samples; before that an empty ring is just the source
    /// still starting up, not an underrun.
    active: bool,
    /// Frames the render thread has taken from the ring, ever.
    played_frames: u64,
}

pub(crate) struct SampleRing {
    state: Mutex<RingState>,
    space: Condvar,
    capacity: usize,
    channels: usize,
}

impl SampleRing {
    pub(crate) fn new(format: StreamFormat) -> Self {
        let channels = usize::from(format.channels).max(1);
        let frames = (f64::from(format.rate) * RING_SECONDS) as usize;
        Self {
            state: Mutex::new(RingState {
                data: VecDeque::new(),
                generation: 0,
                done: false,
                active: false,
                played_frames: 0,
            }),
            space: Condvar::new(),
            capacity: frames.max(1024) * channels,
            channels,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RingState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queue as much of `samples` as fits (whole frames only; the slice is advanced past what was taken).
    fn push(&self, generation: u64, samples: &mut &[f32]) -> Push {
        let mut state = self.lock();
        if state.generation != generation {
            return Push::Stale;
        }
        let room = self.capacity - state.data.len();
        let room = room - room % self.channels;
        if room == 0 {
            return Push::Full;
        }
        let take = room.min(samples.len());
        state.data.extend(samples[..take].iter().copied());
        state.active = true;
        *samples = &samples[take..];
        Push::Accepted
    }

    fn wait_space(&self, timeout: Duration) {
        let state = self.lock();
        if self.capacity - state.data.len() >= self.channels {
            return;
        }
        let _ = self.space.wait_timeout(state, timeout);
    }

    /// Take up to `out.len()` samples (whole frames); returns how many were written to `out`.
    pub(crate) fn pop_into(&self, out: &mut [f32]) -> usize {
        let mut state = self.lock();
        let mut count = out.len().min(state.data.len());
        count -= count % self.channels;
        for (slot, sample) in out.iter_mut().zip(state.data.drain(..count)) {
            *slot = sample;
        }
        state.played_frames += (count / self.channels) as u64;
        drop(state);
        self.space.notify_all();
        count
    }

    /// Throw away what is queued and start a new generation. Returns it and the frames played so far (taken under
    /// the same lock, so positions are counted from exactly here).
    pub(crate) fn new_generation(&self) -> (u64, u64) {
        let mut state = self.lock();
        state.data.clear();
        state.generation += 1;
        state.done = false;
        state.active = false;
        let result = (state.generation, state.played_frames);
        drop(state);
        self.space.notify_all();
        result
    }

    fn mark_done(&self, generation: u64) {
        let mut state = self.lock();
        if state.generation == generation {
            state.done = true;
        }
    }

    /// The current source ended and the device has taken everything it produced.
    pub(crate) fn finished(&self) -> bool {
        let state = self.lock();
        state.done && state.data.is_empty()
    }

    /// An empty ring now would be an underrun (a source is playing and has not ended).
    fn expecting_audio(&self) -> bool {
        let state = self.lock();
        state.active && !state.done
    }

    pub(crate) fn played_frames(&self) -> u64 {
        self.lock().played_frames
    }

    #[cfg(test)]
    pub(crate) fn queued_samples(&self) -> usize {
        self.lock().data.len()
    }
}

/// What the render thread and its owner share.
pub(crate) struct RenderControl {
    pub(crate) paused: AtomicBool,
    /// Linear gain as `f32` bits; 1.0 is a pure pass-through.
    pub(crate) volume: AtomicU32,
    /// Times the device asked for audio the ring did not have.
    pub(crate) underruns: AtomicU64,
    /// Frames of audio the endpoint buffer holds ahead of what is audible.
    pub(crate) latency_frames: AtomicU64,
    /// Why the stream stopped, if it did.
    pub(crate) failure: Mutex<Option<String>>,
}

impl RenderControl {
    fn new(volume: f32, paused: bool) -> Self {
        Self {
            paused: AtomicBool::new(paused),
            volume: AtomicU32::new(volume.to_bits()),
            underruns: AtomicU64::new(0),
            latency_frames: AtomicU64::new(0),
            failure: Mutex::new(None),
        }
    }

    pub(crate) fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn fail(&self, reason: String) {
        let mut failure = self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if failure.is_none() {
            *failure = Some(reason);
        }
    }
}

/// A source of interleaved `f32` samples at the stream's rate and channel count.
pub(crate) type SampleSource = Box<dyn Iterator<Item = f32> + Send>;

pub(crate) struct DecodeJob {
    pub(crate) generation: u64,
    pub(crate) source: SampleSource,
}

pub(crate) struct Session {
    pub(crate) negotiated: Negotiated,
    pub(crate) ring: Arc<SampleRing>,
    pub(crate) control: Arc<RenderControl>,
    jobs: Option<mpsc::Sender<DecodeJob>>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl Session {
    /// Open the endpoint (built on the render thread, which owns it from then on), negotiate a format for a track at
    /// `source_rate` with `source_channels`, start the stream and the two threads.
    pub(crate) fn start(
        make_endpoint: impl FnOnce() -> Result<Box<dyn Endpoint>, String> + Send + 'static,
        source_rate: u32,
        source_channels: u16,
        volume: f32,
        paused: bool,
    ) -> Result<Session, String> {
        let control = Arc::new(RenderControl::new(volume, paused));
        let stop = Arc::new(AtomicBool::new(false));
        let (opened_tx, opened_rx) =
            mpsc::sync_channel::<Result<(Negotiated, Arc<SampleRing>), String>>(1);
        let render_control = Arc::clone(&control);
        let render_stop = Arc::clone(&stop);
        let render = thread::Builder::new()
            .name("moe-exclusive-render".to_owned())
            .spawn(move || {
                let _boost = super::boost_render_thread();
                let opened = (|| {
                    let mut endpoint = make_endpoint()?;
                    let negotiated = negotiate(&mut *endpoint, source_rate, source_channels)
                        .ok_or_else(|| "這個裝置不接受任何可用的格式。".to_owned())?;
                    let stream = endpoint.open(&negotiated.format)?;
                    Ok::<_, String>((endpoint, negotiated, stream))
                })();
                let (endpoint, negotiated, mut stream) = match opened {
                    Ok(opened) => opened,
                    Err(reason) => {
                        let _ = opened_tx.send(Err(reason));
                        return;
                    }
                };
                let ring = Arc::new(SampleRing::new(negotiated.format));
                if opened_tx.send(Ok((negotiated, Arc::clone(&ring)))).is_err() {
                    return;
                }
                if let Err(reason) = render_loop(
                    &mut *stream,
                    negotiated.format,
                    &ring,
                    &render_control,
                    &render_stop,
                ) {
                    render_control.fail(reason);
                }
                drop(stream);
                drop(endpoint);
            })
            .map_err(|error| error.to_string())?;
        let (negotiated, ring) = match opened_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(opened)) => opened,
            Ok(Err(reason)) => {
                let _ = render.join();
                return Err(reason);
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                return Err("開啟裝置逾時。".to_owned());
            }
        };
        let (jobs, job_rx) = mpsc::channel();
        let decode_ring = Arc::clone(&ring);
        let decode_stop = Arc::clone(&stop);
        let channels = usize::from(negotiated.format.channels).max(1);
        let decode = thread::Builder::new()
            .name("moe-exclusive-decode".to_owned())
            .spawn(move || decode_loop(&decode_ring, &job_rx, &decode_stop, channels))
            .map_err(|error| error.to_string())?;
        Ok(Session {
            negotiated,
            ring,
            control,
            jobs: Some(jobs),
            stop,
            threads: vec![render, decode],
        })
    }

    /// Start playing `source` from now on; whatever was queued is dropped. Returns the frames played before this
    /// moment (the position of the new source is counted from it).
    pub(crate) fn play_source(&self, source: SampleSource) -> u64 {
        let (generation, played) = self.ring.new_generation();
        if let Some(jobs) = self.jobs.as_ref() {
            let _ = jobs.send(DecodeJob { generation, source });
        }
        played
    }

    /// Stop the threads. The endpoint is released when the render thread ends, and a new exclusive stream cannot open
    /// before that, so this waits for it a little; a driver call that never returns is left to finish on its own
    /// thread instead of taking the caller with it.
    pub(crate) fn close(mut self) {
        self.stop.store(true, Ordering::Release);
        self.jobs = None;
        let threads = std::mem::take(&mut self.threads);
        let deadline = std::time::Instant::now() + CLOSE_WAIT;
        while std::time::Instant::now() < deadline && !threads.iter().all(JoinHandle::is_finished) {
            thread::sleep(Duration::from_millis(2));
        }
        let _ = thread::Builder::new()
            .name("moe-exclusive-close".to_owned())
            .spawn(move || {
                for thread in threads {
                    let _ = thread.join();
                }
            });
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn decode_loop(
    ring: &SampleRing,
    jobs: &mpsc::Receiver<DecodeJob>,
    stop: &AtomicBool,
    channels: usize,
) {
    let chunk = channels * 1024;
    let mut pending: Option<DecodeJob> = None;
    let mut buffer: Vec<f32> = Vec::with_capacity(chunk);
    'jobs: loop {
        let mut job = match pending.take() {
            Some(job) => job,
            None => match jobs.recv_timeout(WAIT_SLICE) {
                Ok(job) => job,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
        };
        loop {
            if stop.load(Ordering::Acquire) {
                return;
            }
            if let Ok(newer) = jobs.try_recv() {
                pending = Some(newer);
                continue 'jobs;
            }
            buffer.clear();
            buffer.extend(job.source.by_ref().take(chunk));
            let ended = buffer.len() < chunk;
            let whole = buffer.len() - buffer.len() % channels;
            let mut rest = &buffer[..whole];
            while !rest.is_empty() {
                match ring.push(job.generation, &mut rest) {
                    Push::Accepted => {}
                    Push::Stale => continue 'jobs,
                    Push::Full => {
                        ring.wait_space(Duration::from_millis(20));
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        if let Ok(newer) = jobs.try_recv() {
                            pending = Some(newer);
                            continue 'jobs;
                        }
                    }
                }
            }
            if ended {
                ring.mark_done(job.generation);
                continue 'jobs;
            }
        }
    }
}

fn render_loop(
    stream: &mut dyn RenderStream,
    format: StreamFormat,
    ring: &SampleRing,
    control: &RenderControl,
    stop: &AtomicBool,
) -> Result<(), String> {
    let channels = usize::from(format.channels).max(1);
    let mut samples: Vec<f32> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    // Fill the buffer with silence before starting, as event-driven streams require.
    let free = stream.free_frames()?;
    silence(format, free, &mut bytes);
    stream.write(free, &bytes)?;
    control
        .latency_frames
        .store(stream.buffer_frames() as u64, Ordering::Relaxed);
    stream.start()?;
    let mut gain_now = f32::from_bits(control.volume.load(Ordering::Relaxed));
    while !stop.load(Ordering::Acquire) {
        if !stream.wait(WAIT_SLICE)? {
            continue;
        }
        let free = stream.free_frames()?;
        if free == 0 {
            continue;
        }
        samples.clear();
        samples.resize(free * channels, 0.0);
        let mut got = 0;
        if !control.paused.load(Ordering::Acquire) {
            got = ring.pop_into(&mut samples);
            if got < samples.len() && ring.expecting_audio() {
                control.underruns.fetch_add(1, Ordering::Relaxed);
            }
        }
        let target = f32::from_bits(control.volume.load(Ordering::Relaxed));
        // At full volume the samples pass through untouched, so what reaches the device is what the file holds.
        if got > 0 && (target < 0.9999 || gain_now < 0.9999) {
            for frame in samples[..got].chunks_mut(channels) {
                gain_now += (target - gain_now) * GAIN_EASE;
                if (target - gain_now).abs() < 1e-4 {
                    gain_now = target;
                }
                for sample in frame {
                    *sample *= gain_now;
                }
            }
        } else {
            gain_now = target;
        }
        bytes.clear();
        encode(&samples, format.container, &mut bytes);
        stream.write(free, &bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive::endpoint::fake::{Capture, FakeEndpoint};
    use crate::exclusive::format::Container;
    use std::time::Instant;

    fn format(rate: u32, channels: u16, container: Container) -> StreamFormat {
        StreamFormat {
            rate,
            channels,
            container,
        }
    }

    fn start(
        accepts: Vec<StreamFormat>,
        rate: u32,
        channels: u16,
        volume: f32,
        paused: bool,
    ) -> (Session, Arc<Capture>) {
        let capture = Arc::new(Capture::default());
        let device_capture = Arc::clone(&capture);
        let session = Session::start(
            move || {
                Ok(Box::new(FakeEndpoint {
                    accepts,
                    capture: device_capture,
                    period_frames: 256,
                    tick: Duration::from_micros(300),
                }) as Box<dyn Endpoint>)
            },
            rate,
            channels,
            volume,
            paused,
        )
        .expect("session");
        (session, capture)
    }

    fn wait_for(what: &str, mut condition: impl FnMut() -> bool) {
        let started = Instant::now();
        while !condition() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "timed out waiting for {what}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// 24-bit sample values 1, 2, 3 ... counted up per frame (left) and down (right), as `f32` the way decoders
    /// produce them.
    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|index| {
                let left = (index as i32 + 1) * 1000;
                let right = -((index as i32 + 1) * 1000);
                [left as f32 / 8_388_608.0, right as f32 / 8_388_608.0]
            })
            .collect()
    }

    fn decode_24_in_32(bytes: &[u8]) -> Vec<i32> {
        bytes
            .chunks(4)
            .map(|word| i32::from_le_bytes([word[0], word[1], word[2], word[3]]) >> 8)
            .collect()
    }

    #[test]
    fn what_the_file_holds_is_what_reaches_the_device_bit_for_bit() {
        let (session, capture) = start(
            vec![format(44_100, 2, Container::Int24In32)],
            44_100,
            2,
            1.0,
            false,
        );
        assert_eq!(
            session.negotiated.format,
            format(44_100, 2, Container::Int24In32)
        );
        assert!(!session.negotiated.resampled());
        let source = ramp(5_000);
        session.play_source(Box::new(source.into_iter()));
        wait_for("the track to finish", || session.ring.finished());
        thread::sleep(Duration::from_millis(30));
        let all = decode_24_in_32(&capture.bytes.lock().unwrap());
        let start = all
            .iter()
            .position(|&value| value != 0)
            .expect("audio reached the device");
        let expected: Vec<i32> = (0..5_000)
            .flat_map(|index| [(index + 1) * 1000, -((index + 1) * 1000)])
            .collect();
        assert_eq!(
            &all[start..start + expected.len()],
            &expected[..],
            "no sample changed on the way"
        );
        assert!(
            all[start + expected.len()..]
                .iter()
                .all(|&value| value == 0),
            "silence after the end"
        );
        assert_eq!(session.control.underruns.load(Ordering::Relaxed), 0);
        session.close();
    }

    #[test]
    fn a_paused_stream_plays_silence_and_keeps_the_queue_for_resuming() {
        let (session, capture) = start(
            vec![format(48_000, 2, Container::Int24In32)],
            48_000,
            2,
            1.0,
            true,
        );
        session.play_source(Box::new(ramp(20_000).into_iter()));
        wait_for("the ring to fill", || {
            session.ring.queued_samples() > 10_000
        });
        thread::sleep(Duration::from_millis(30));
        assert!(
            decode_24_in_32(&capture.bytes.lock().unwrap())
                .iter()
                .all(|&value| value == 0),
            "nothing but silence while paused"
        );
        let played = session.ring.played_frames();
        session.control.paused.store(false, Ordering::Release);
        wait_for("playing to resume", || {
            session.ring.played_frames() > played
        });
        session.close();
    }

    #[test]
    fn a_new_source_replaces_what_was_queued_without_old_audio_leaking_after_it() {
        let (session, capture) = start(
            vec![format(48_000, 2, Container::Int24In32)],
            48_000,
            2,
            1.0,
            false,
        );
        let old: Vec<f32> = std::iter::repeat_n([0.25f32, 0.25], 100_000)
            .flatten()
            .collect();
        session.play_source(Box::new(old.into_iter()));
        wait_for("old audio to play", || {
            capture.bytes.lock().unwrap().iter().any(|&byte| byte != 0)
        });
        let new: Vec<f32> = std::iter::repeat_n([-0.5f32, -0.5], 2_000)
            .flatten()
            .collect();
        session.play_source(Box::new(new.into_iter()));
        wait_for("the new source to finish", || session.ring.finished());
        thread::sleep(Duration::from_millis(30));
        let values = decode_24_in_32(&capture.bytes.lock().unwrap());
        let first_new = values
            .iter()
            .position(|&value| value == -4_194_304)
            .expect("new audio played");
        assert!(
            values[first_new..]
                .iter()
                .all(|&value| value == -4_194_304 || value == 0),
            "after the new source starts, no old sample appears"
        );
        assert_eq!(
            values.iter().filter(|&&value| value == -4_194_304).count(),
            4_000
        );
        session.close();
    }

    #[test]
    fn the_volume_scales_samples_and_full_volume_is_a_pass_through() {
        let (session, capture) = start(
            vec![format(48_000, 2, Container::Int24In32)],
            48_000,
            2,
            0.5,
            false,
        );
        // The gain eases in from the starting volume, which is already 0.5, so it is steady from the first sample.
        let source: Vec<f32> = std::iter::repeat_n([0.5f32, 0.5], 3_000)
            .flatten()
            .collect();
        session.play_source(Box::new(source.into_iter()));
        wait_for("the track to finish", || session.ring.finished());
        thread::sleep(Duration::from_millis(30));
        let values = decode_24_in_32(&capture.bytes.lock().unwrap());
        assert_eq!(
            values.iter().filter(|&&value| value == 2_097_152).count(),
            6_000,
            "0.5 x 0.5 of full scale"
        );
        session.close();
    }

    #[test]
    fn a_device_that_disappears_is_reported() {
        let (session, capture) = start(
            vec![format(48_000, 2, Container::Int24In32)],
            48_000,
            2,
            1.0,
            false,
        );
        session.play_source(Box::new(ramp(100_000).into_iter()));
        capture.fail.store(true, Ordering::Relaxed);
        wait_for("the failure", || session.control.failure().is_some());
        assert!(session.control.failure().unwrap().contains("中斷"));
        session.close();
    }

    #[test]
    fn a_source_that_cannot_keep_up_counts_underruns_instead_of_stopping() {
        struct Slow(usize);
        impl Iterator for Slow {
            type Item = f32;
            fn next(&mut self) -> Option<f32> {
                if self.0 == 0 {
                    return None;
                }
                self.0 -= 1;
                if self.0 % 2_000 == 0 {
                    thread::sleep(Duration::from_millis(20));
                }
                Some(0.1)
            }
        }
        let (session, _capture) = start(
            vec![format(48_000, 2, Container::Int24In32)],
            48_000,
            2,
            1.0,
            false,
        );
        session.play_source(Box::new(Slow(20_000)));
        wait_for("the track to finish", || session.ring.finished());
        assert!(session.control.underruns.load(Ordering::Relaxed) > 0);
        assert!(session.control.failure().is_none());
        session.close();
    }

    #[test]
    fn a_device_with_no_usable_format_fails_to_open_with_a_reason() {
        let capture = Arc::new(Capture::default());
        let result = Session::start(
            move || {
                Ok(Box::new(FakeEndpoint {
                    accepts: Vec::new(),
                    capture,
                    period_frames: 256,
                    tick: Duration::from_micros(300),
                }) as Box<dyn Endpoint>)
            },
            44_100,
            2,
            1.0,
            false,
        );
        assert!(result.err().unwrap().contains("格式"));
    }
}
