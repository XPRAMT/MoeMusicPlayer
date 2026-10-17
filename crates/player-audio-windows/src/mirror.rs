//! Playing one track on several devices at once (ADR 0020, phase 0).
//!
//! The first enabled device is the primary and keeps the whole existing path. Every other device is a mirror:
//!
//! ```text
//! track ─► Tee ─┬─► DelayLine ─► MixSource ─► Player ─► primary device
//!               ├─► ring ─► MirrorSource (resample, drift fix, delay, channel map, gain) ─► mirror device 1
//!               └─► ring ─► MirrorSource ─► mirror device 2
//! ```
//!
//! The devices run on their own clocks, so a mirror never reads at exactly the rate the track is produced. Its
//! `MirrorSource` keeps the ring's fill level at a target with a tiny resampling ratio correction (a few tenths of a
//! percent at most). The target is the jitter margin plus the device's delay; the primary is delayed by the same
//! margin plus its own delay, so with every delay at 0 the devices are aligned before their hardware latencies.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use rodio::{ChannelCount, Sample, SampleRate, Source};

use crate::output_devices::ChannelMixer;

/// Samples (all channels) one ring holds: about 5 s of stereo at 96 kHz, 2.7 s at 192 kHz.
pub(crate) const RING_SAMPLES: usize = 1 << 20;
/// Most channels a track or device is mixed with; more are ignored.
pub(crate) const MAX_CHANNELS: usize = 8;
/// Audio a mirror keeps buffered to ride out scheduling jitter of the two device callbacks.
pub(crate) const JITTER_MARGIN_MS: u32 = 60;
/// Largest relative change of the resampling ratio the drift correction applies.
const MAX_RATIO_ADJUST: f64 = 0.005;

/// One producer, one consumer ring of interleaved samples. The producer is the primary device's render callback and
/// must never wait; the consumer is a mirror device's callback.
pub(crate) struct Ring {
    data: Box<[AtomicU32]>,
    /// Samples written since the last reset (monotonic).
    write: AtomicU64,
    /// Bumped by anyone that wants the consumer to throw its buffered audio away or re-read the format.
    epoch: AtomicU64,
    channels: AtomicU32,
    rate: AtomicU32,
}

impl Ring {
    fn new() -> Self {
        Self {
            data: (0..RING_SAMPLES).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicU64::new(0),
            epoch: AtomicU64::new(1),
            channels: AtomicU32::new(2),
            rate: AtomicU32::new(48_000),
        }
    }

    fn push_frame(&self, frame: &[Sample]) {
        let start = self.write.load(Ordering::Relaxed);
        for (offset, sample) in frame.iter().enumerate() {
            self.data[((start + offset as u64) % RING_SAMPLES as u64) as usize]
                .store(sample.to_bits(), Ordering::Relaxed);
        }
        self.write
            .store(start + frame.len() as u64, Ordering::Release);
    }

    fn configure(&self, channels: usize, rate: u32) {
        self.channels.store(channels as u32, Ordering::Relaxed);
        self.rate.store(rate, Ordering::Relaxed);
        self.epoch.fetch_add(1, Ordering::Release);
    }

    fn flush(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
    }
}

/// The consumer end: owns the read position.
pub(crate) struct RingReader {
    ring: Arc<Ring>,
    read: u64,
    epoch: u64,
}

impl RingReader {
    pub(crate) fn new(ring: Arc<Ring>) -> Self {
        let read = ring.write.load(Ordering::Acquire);
        let epoch = ring.epoch.load(Ordering::Acquire);
        Self { ring, read, epoch }
    }

    /// Notice a flush or a new format: discard what is buffered. Returns true when that happened.
    fn sync(&mut self) -> bool {
        let epoch = self.ring.epoch.load(Ordering::Acquire);
        if epoch == self.epoch {
            return false;
        }
        self.epoch = epoch;
        self.read = self.ring.write.load(Ordering::Acquire);
        true
    }

    fn channels(&self) -> usize {
        (self.ring.channels.load(Ordering::Relaxed) as usize).clamp(1, MAX_CHANNELS)
    }

    fn rate(&self) -> u32 {
        self.ring.rate.load(Ordering::Relaxed).max(1)
    }

    /// Whole frames buffered.
    fn available_frames(&mut self) -> usize {
        let channels = self.ring.channels.load(Ordering::Relaxed).max(1) as u64;
        let written = self.ring.write.load(Ordering::Acquire);
        // The producer lapped us (the mirror stalled): jump to the newest audio.
        if written - self.read > RING_SAMPLES as u64 - 2 * channels {
            self.read = written - RING_SAMPLES as u64 / 2;
            self.read -= self.read % channels;
        }
        ((written - self.read) / channels) as usize
    }

    fn skip_frames(&mut self, frames: usize) {
        let channels = self.ring.channels.load(Ordering::Relaxed).max(1) as u64;
        self.read += frames as u64 * channels;
    }

    fn pop_frame(&mut self, out: &mut [f32; MAX_CHANNELS]) -> bool {
        if self.available_frames() == 0 {
            return false;
        }
        let ring_channels = self.ring.channels.load(Ordering::Relaxed).max(1) as usize;
        for channel in 0..ring_channels {
            let value = f32::from_bits(
                self.ring.data[((self.read + channel as u64) % RING_SAMPLES as u64) as usize]
                    .load(Ordering::Relaxed),
            );
            if channel < MAX_CHANNELS {
                out[channel] = value;
            }
        }
        self.read += ring_channels as u64;
        true
    }
}

/// Where the tee copies the track to. Shared by the backend (which adds and removes mirrors) and the tee.
pub(crate) struct MirrorHub {
    rings: RwLock<Arc<Vec<Arc<Ring>>>>,
    version: AtomicU64,
    /// Bumped on pause, seek, stop and track change so delay lines and mirrors drop what they buffered.
    flushes: AtomicU64,
    format: Mutex<(usize, u32)>,
}

impl MirrorHub {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            rings: RwLock::new(Arc::new(Vec::new())),
            version: AtomicU64::new(1),
            flushes: AtomicU64::new(0),
            format: Mutex::new((2, 48_000)),
        })
    }

    pub(crate) fn add_ring(&self) -> Arc<Ring> {
        let ring = Arc::new(Ring::new());
        let (channels, rate) = *self
            .format
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ring.configure(channels, rate);
        let mut rings = self
            .rings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = rings.as_ref().clone();
        next.push(Arc::clone(&ring));
        *rings = Arc::new(next);
        self.version.fetch_add(1, Ordering::Release);
        ring
    }

    pub(crate) fn remove_ring(&self, ring: &Arc<Ring>) {
        let mut rings = self
            .rings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next: Vec<_> = rings
            .iter()
            .filter(|existing| !Arc::ptr_eq(existing, ring))
            .cloned()
            .collect();
        *rings = Arc::new(next);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn snapshot(&self) -> Arc<Vec<Arc<Ring>>> {
        Arc::clone(
            &self
                .rings
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// A track with this format starts: every mirror re-reads it and starts buffering again.
    pub(crate) fn configure(&self, channels: usize, rate: u32) {
        *self
            .format
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (channels, rate);
        for ring in self.snapshot().iter() {
            ring.configure(channels, rate);
        }
        self.flushes.fetch_add(1, Ordering::Release);
    }

    /// Make the mirrors drop what they buffered (pause), leaving the primary's delay line alone.
    pub(crate) fn flush_mirrors(&self) {
        for ring in self.snapshot().iter() {
            ring.flush();
        }
    }

    /// Throw away everything buffered (seek, stop, a new track).
    pub(crate) fn flush(&self) {
        for ring in self.snapshot().iter() {
            ring.flush();
        }
        self.flushes.fetch_add(1, Ordering::Release);
    }

    fn flush_count(&self) -> u64 {
        self.flushes.load(Ordering::Acquire)
    }
}

/// Copies every frame of the track into the mirrors' rings and passes it on unchanged.
pub(crate) struct Tee<S: Source<Item = Sample>> {
    inner: S,
    hub: Arc<MirrorHub>,
    channels: usize,
    frame: [Sample; MAX_CHANNELS],
    filled: usize,
    rings: Arc<Vec<Arc<Ring>>>,
    seen_version: u64,
}

impl<S: Source<Item = Sample>> Tee<S> {
    pub(crate) fn new(inner: S, hub: Arc<MirrorHub>) -> Self {
        let channels = usize::from(inner.channels().get()).clamp(1, MAX_CHANNELS);
        hub.configure(channels, inner.sample_rate().get());
        let rings = hub.snapshot();
        let seen_version = hub.version.load(Ordering::Acquire);
        Self {
            inner,
            hub,
            channels,
            frame: [0.0; MAX_CHANNELS],
            filled: 0,
            rings,
            seen_version,
        }
    }
}

impl<S: Source<Item = Sample>> Iterator for Tee<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let sample = self.inner.next()?;
        let source_channels = usize::from(self.inner.channels().get());
        if self.filled < MAX_CHANNELS {
            self.frame[self.filled] = sample;
        }
        self.filled += 1;
        if self.filled >= source_channels {
            self.filled = 0;
            let version = self.hub.version.load(Ordering::Acquire);
            if version != self.seen_version {
                self.seen_version = version;
                self.rings = self.hub.snapshot();
            }
            for ring in self.rings.iter() {
                ring.push_frame(&self.frame[..self.channels]);
            }
        }
        Some(sample)
    }
}

impl<S: Source<Item = Sample>> Source for Tee<S> {
    fn current_span_len(&self) -> Option<usize> {
        self.inner.current_span_len()
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        let result = self.inner.try_seek(position);
        self.filled = 0;
        self.hub.flush();
        result
    }
}

/// Delays a source by a number of milliseconds (silence in front). The delay is read live from `delay_ms`; the queue
/// is dropped when the hub is flushed or the source seeks, so a pause or a seek leaves no old audio behind.
pub(crate) struct DelayLine<S: Source<Item = Sample>> {
    inner: S,
    delay_ms: Arc<AtomicU32>,
    hub: Arc<MirrorHub>,
    queue: VecDeque<Sample>,
    seen_flush: u64,
    draining: bool,
    /// Samples of inserted silence played since the last seek: the position the player reports counts them, the
    /// backend takes them off again.
    silence: Arc<AtomicU64>,
}

impl<S: Source<Item = Sample>> DelayLine<S> {
    pub(crate) fn new(
        inner: S,
        delay_ms: Arc<AtomicU32>,
        hub: Arc<MirrorHub>,
        silence: Arc<AtomicU64>,
    ) -> Self {
        let seen_flush = hub.flush_count();
        Self {
            inner,
            delay_ms,
            hub,
            silence,
            queue: VecDeque::new(),
            seen_flush,
            draining: false,
        }
    }

    fn delay_samples(&self) -> usize {
        let channels = usize::from(self.inner.channels().get());
        let frames = (u64::from(self.delay_ms.load(Ordering::Relaxed))
            * u64::from(self.inner.sample_rate().get())
            / 1000) as usize;
        frames * channels
    }
}

impl<S: Source<Item = Sample>> Iterator for DelayLine<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let flushes = self.hub.flush_count();
        if flushes != self.seen_flush {
            self.seen_flush = flushes;
            self.queue.clear();
            self.draining = false;
        }
        let delay = self.delay_samples();
        if delay == 0 && self.queue.is_empty() && !self.draining {
            return self.inner.next();
        }
        if !self.draining {
            match self.inner.next() {
                Some(sample) => self.queue.push_back(sample),
                None => self.draining = true,
            }
        }
        if self.queue.len() > delay || (self.draining && !self.queue.is_empty()) {
            // A shorter delay than before drops the surplus here (one click, only while the user is adjusting it).
            while self.queue.len() > delay + usize::from(self.inner.channels().get()) * 64
                && !self.draining
            {
                self.queue.pop_front();
            }
            return self.queue.pop_front();
        }
        if self.draining {
            None
        } else {
            self.silence.fetch_add(1, Ordering::Relaxed);
            Some(0.0)
        }
    }
}

impl<S: Source<Item = Sample>> Source for DelayLine<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        self.inner.channels()
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        let result = self.inner.try_seek(position);
        self.queue.clear();
        self.draining = false;
        self.silence.store(0, Ordering::Relaxed);
        result
    }
}

/// Mixes a source's channels into the device's channels (the primary's channel map).
pub(crate) struct MixSource<S: Source<Item = Sample>> {
    inner: S,
    mixer: ChannelMixer,
    input: [Sample; MAX_CHANNELS],
    output: [Sample; MAX_CHANNELS],
    out_pos: usize,
    device_channels: usize,
}

impl<S: Source<Item = Sample>> MixSource<S> {
    pub(crate) fn new(inner: S, mixer: ChannelMixer) -> Self {
        let device_channels = mixer.device_channels().clamp(1, MAX_CHANNELS);
        Self {
            inner,
            mixer,
            input: [0.0; MAX_CHANNELS],
            output: [0.0; MAX_CHANNELS],
            out_pos: device_channels,
            device_channels,
        }
    }
}

impl<S: Source<Item = Sample>> Iterator for MixSource<S> {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.out_pos >= self.device_channels {
            let source_channels = self.mixer.source_channels().clamp(1, MAX_CHANNELS);
            for channel in 0..source_channels {
                self.input[channel] = self.inner.next()?;
            }
            // Channels beyond the ones mixed (a source with more than MAX_CHANNELS) are skipped.
            for _ in source_channels..usize::from(self.inner.channels().get()) {
                self.inner.next()?;
            }
            self.mixer.mix(
                &self.input[..source_channels],
                &mut self.output[..self.device_channels],
            );
            self.out_pos = 0;
        }
        let sample = self.output[self.out_pos];
        self.out_pos += 1;
        Some(sample)
    }
}

impl<S: Source<Item = Sample>> Source for MixSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.device_channels as u16).unwrap_or(ChannelCount::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        self.inner.sample_rate()
    }

    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, position: Duration) -> Result<(), rodio::source::SeekError> {
        self.out_pos = self.device_channels;
        self.inner.try_seek(position)
    }
}

/// What the backend changes while a mirror plays.
pub(crate) struct MirrorControl {
    /// Linear gain (master volume times the device's volume), as `f32` bits.
    pub(crate) gain: AtomicU32,
    pub(crate) delay_ms: AtomicU32,
    /// Frames of source audio the mirror consumed; the backend reads it to see that the device is alive.
    pub(crate) consumed: AtomicU64,
    /// Times the mirror ran dry and re-buffered (diagnostics).
    pub(crate) underruns: AtomicU64,
    /// The device's channel map; the mirror picks a change up within about 10 ms.
    map: Mutex<Option<Vec<crate::output_devices::ChannelRoute>>>,
    map_version: AtomicU64,
}

impl MirrorControl {
    pub(crate) fn new(gain: f32, delay_ms: u32) -> Arc<Self> {
        Arc::new(Self {
            gain: AtomicU32::new(gain.to_bits()),
            delay_ms: AtomicU32::new(delay_ms),
            consumed: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            map: Mutex::new(None),
            map_version: AtomicU64::new(0),
        })
    }

    pub(crate) fn set_map(&self, map: Option<Vec<crate::output_devices::ChannelRoute>>) {
        *self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = map;
        self.map_version.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
}

/// The audio of one mirror device: reads the ring at the device's rate with a drift-correcting ratio, mixes the
/// channels and applies the gain.
pub(crate) struct MirrorSource {
    reader: RingReader,
    control: Arc<MirrorControl>,
    device_channels: usize,
    device_rate: u32,
    map: Option<Vec<crate::output_devices::ChannelRoute>>,
    map_seen: u64,
    mixer: ChannelMixer,
    source_rate: u32,
    source_channels: usize,
    /// Four most recent source frames for the cubic interpolation (`history[2]` is the one just before `phase`).
    history: [[f32; MAX_CHANNELS]; 4],
    phase: f64,
    ratio: f64,
    buffering: bool,
    fill_average: f64,
    frames_until_check: u32,
    out: [Sample; MAX_CHANNELS],
    out_pos: usize,
    gain_now: f32,
}

impl MirrorSource {
    pub(crate) fn new(
        ring: Arc<Ring>,
        control: Arc<MirrorControl>,
        device_channels: usize,
        device_rate: u32,
        map: Option<Vec<crate::output_devices::ChannelRoute>>,
    ) -> Self {
        let device_channels = device_channels.clamp(1, MAX_CHANNELS);
        let reader = RingReader::new(ring);
        let source_channels = reader.channels();
        let source_rate = reader.rate();
        let mixer = ChannelMixer::new(map.as_deref(), source_channels, device_channels);
        let gain_now = f32::from_bits(control.gain.load(Ordering::Relaxed));
        let map_seen = control.map_version.load(Ordering::Acquire);
        Self {
            reader,
            control,
            device_channels,
            device_rate: device_rate.max(1),
            map,
            map_seen,
            mixer,
            source_rate,
            source_channels,
            history: [[0.0; MAX_CHANNELS]; 4],
            phase: 0.0,
            ratio: f64::from(source_rate) / f64::from(device_rate.max(1)),
            buffering: true,
            fill_average: 0.0,
            frames_until_check: 0,
            out: [0.0; MAX_CHANNELS],
            out_pos: device_channels,
            gain_now,
        }
    }

    fn target_frames(&self) -> usize {
        let milliseconds =
            u64::from(JITTER_MARGIN_MS) + u64::from(self.control.delay_ms.load(Ordering::Relaxed));
        let wanted = (milliseconds * u64::from(self.source_rate) / 1000) as usize;
        // Never ask for more than the ring can hold.
        wanted.min(RING_SAMPLES / self.source_channels.max(1) / 2)
    }

    fn base_ratio(&self) -> f64 {
        f64::from(self.source_rate) / f64::from(self.device_rate)
    }

    fn reconfigure(&mut self) {
        self.source_channels = self.reader.channels();
        self.source_rate = self.reader.rate();
        self.mixer = ChannelMixer::new(
            self.map.as_deref(),
            self.source_channels,
            self.device_channels,
        );
        self.history = [[0.0; MAX_CHANNELS]; 4];
        self.phase = 0.0;
        self.ratio = self.base_ratio();
        self.buffering = true;
        self.fill_average = 0.0;
    }

    fn start_playing(&mut self) {
        // Prime the interpolation window with the first frames.
        for slot in 0..4 {
            let mut frame = [0.0; MAX_CHANNELS];
            if !self.reader.pop_frame(&mut frame) {
                self.buffering = true;
                return;
            }
            self.history[slot] = frame;
        }
        self.phase = 0.0;
        self.buffering = false;
        self.fill_average = self.reader.available_frames() as f64;
        self.frames_until_check = 0;
    }

    /// Keep the ring's fill level at the target: a few tenths of a percent faster or slower, more only while the
    /// user is moving the delay.
    fn correct_drift(&mut self) {
        let available = self.reader.available_frames() as f64;
        self.fill_average += 0.1 * (available - self.fill_average);
        let target = self.target_frames() as f64;
        let error_seconds = (self.fill_average - target) / f64::from(self.source_rate);
        // A big gap (the delay was changed): jump instead of waiting for the slow correction.
        if error_seconds.abs() > 0.25 {
            let available = self.reader.available_frames();
            if available as f64 > target {
                self.reader.skip_frames(available - target as usize);
                self.fill_average = target;
            } else {
                self.buffering = true;
            }
            return;
        }
        let adjust = (error_seconds * 0.2).clamp(-MAX_RATIO_ADJUST, MAX_RATIO_ADJUST);
        self.ratio = self.base_ratio() * (1.0 + adjust);
    }

    fn next_frame(&mut self) {
        let channels = self.source_channels;
        if self.reader.sync() {
            self.reconfigure();
        }
        if self.buffering {
            self.out[..self.device_channels].fill(0.0);
            if self.reader.available_frames() >= self.target_frames().max(4) {
                self.start_playing();
            }
            return;
        }
        if self.frames_until_check == 0 {
            self.frames_until_check = 512;
            let map_version = self.control.map_version.load(Ordering::Acquire);
            if map_version != self.map_seen {
                self.map_seen = map_version;
                self.map = self
                    .control
                    .map
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                self.mixer = ChannelMixer::new(
                    self.map.as_deref(),
                    self.source_channels,
                    self.device_channels,
                );
            }
            self.correct_drift();
            if self.buffering {
                self.out[..self.device_channels].fill(0.0);
                return;
            }
        }
        self.frames_until_check -= 1;
        // Advance the source position by `ratio` frames.
        self.phase += self.ratio;
        while self.phase >= 1.0 {
            let mut frame = [0.0; MAX_CHANNELS];
            if !self.reader.pop_frame(&mut frame) {
                // Ran dry: the producer paused or stalled. Output silence and re-buffer.
                self.control.underruns.fetch_add(1, Ordering::Relaxed);
                self.buffering = true;
                self.out[..self.device_channels].fill(0.0);
                return;
            }
            self.history.rotate_left(1);
            self.history[3] = frame;
            self.phase -= 1.0;
            self.control.consumed.fetch_add(1, Ordering::Relaxed);
        }
        let fraction = self.phase as f32;
        let mut source = [0.0f32; MAX_CHANNELS];
        for (channel, slot) in source.iter_mut().enumerate().take(channels) {
            *slot = hermite(
                self.history[0][channel],
                self.history[1][channel],
                self.history[2][channel],
                self.history[3][channel],
                fraction,
            );
        }
        let target_gain = f32::from_bits(self.control.gain.load(Ordering::Relaxed));
        self.gain_now += (target_gain - self.gain_now) * 0.002;
        let gain = if (target_gain - self.gain_now).abs() < 1e-4 {
            target_gain
        } else {
            self.gain_now
        };
        self.gain_now = gain;
        self.mixer
            .mix(&source[..channels], &mut self.out[..self.device_channels]);
        for sample in &mut self.out[..self.device_channels] {
            *sample *= gain;
        }
    }
}

/// Four-point cubic Hermite interpolation between `y1` and `y2` at `t` in [0, 1).
fn hermite(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    let c0 = y1;
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + c0
}

impl Iterator for MirrorSource {
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        if self.out_pos >= self.device_channels {
            self.next_frame();
            self.out_pos = 0;
        }
        let sample = self.out[self.out_pos];
        self.out_pos += 1;
        Some(sample)
    }
}

impl Source for MirrorSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(self.device_channels as u16).unwrap_or(ChannelCount::MIN)
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.device_rate).unwrap_or(SampleRate::MIN)
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output_devices::ChannelRoute;
    use rodio::buffer::SamplesBuffer;

    fn rate(value: u32) -> SampleRate {
        SampleRate::new(value).unwrap()
    }

    fn channels(value: u16) -> ChannelCount {
        ChannelCount::new(value).unwrap()
    }

    fn ramp(frames: usize, channel_count: u16, sample_rate: u32) -> SamplesBuffer {
        let data: Vec<f32> = (0..frames * usize::from(channel_count))
            .map(|index| {
                (index / usize::from(channel_count)) as f32 / frames as f32
                    + (index % usize::from(channel_count)) as f32 * 10.0
            })
            .collect();
        SamplesBuffer::new(channels(channel_count), rate(sample_rate), data)
    }

    #[test]
    fn the_tee_copies_every_frame_to_every_mirror_and_passes_the_track_on_unchanged() {
        let hub = MirrorHub::new();
        let first = hub.add_ring();
        let second = hub.add_ring();
        let source = ramp(100, 2, 48_000);
        let expected: Vec<f32> = ramp(100, 2, 48_000).collect();
        let tee = Tee::new(source, Arc::clone(&hub));
        let passed: Vec<f32> = tee.collect();
        assert_eq!(passed, expected);
        for ring in [first, second] {
            assert_eq!(ring.channels.load(Ordering::Relaxed), 2);
            assert_eq!(ring.rate.load(Ordering::Relaxed), 48_000);
            assert_eq!(ring.write.load(Ordering::Relaxed), 200);
            let mut reader = RingReader::new(Arc::clone(&ring));
            reader.read = 0;
            reader.epoch = ring.epoch.load(Ordering::Relaxed);
            let mut frame = [0.0; MAX_CHANNELS];
            for index in 0..100 {
                assert!(reader.pop_frame(&mut frame));
                assert_eq!(frame[0], expected[index * 2]);
                assert_eq!(frame[1], expected[index * 2 + 1]);
            }
            assert!(!reader.pop_frame(&mut frame));
        }
    }

    #[test]
    fn a_flush_makes_a_mirror_forget_what_it_buffered() {
        let hub = MirrorHub::new();
        let ring = hub.add_ring();
        let tee = Tee::new(ramp(50, 1, 8_000), Arc::clone(&hub));
        let _: Vec<f32> = tee.collect();
        let mut reader = RingReader::new(Arc::clone(&ring));
        reader.read = 0;
        reader.epoch = 0;
        assert!(reader.sync(), "the first read sees the new format");
        hub.flush();
        assert!(reader.sync());
        assert_eq!(reader.available_frames(), 0);
        assert!(!reader.sync(), "nothing changed since");
    }

    #[test]
    fn a_delay_line_puts_silence_in_front_keeps_the_audio_and_drains_at_the_end() {
        let hub = MirrorHub::new();
        let delay = Arc::new(AtomicU32::new(10)); // 10 ms at 1 kHz = 10 frames
        let source = SamplesBuffer::new(
            channels(1),
            rate(1_000),
            (1..=20).map(|value| value as f32).collect::<Vec<_>>(),
        );
        let silence = Arc::new(AtomicU64::new(0));
        let line = DelayLine::new(source, delay, hub, Arc::clone(&silence));
        let out: Vec<f32> = line.collect();
        assert_eq!(
            silence.load(Ordering::Relaxed),
            10,
            "the inserted silence is counted"
        );
        let mut expected = vec![0.0; 10];
        expected.extend((1..=20).map(|value| value as f32));
        assert_eq!(out, expected);
    }

    #[test]
    fn a_zero_delay_is_a_pass_through() {
        let hub = MirrorHub::new();
        let source = SamplesBuffer::new(channels(2), rate(48_000), vec![0.1, 0.2, 0.3, 0.4]);
        let out: Vec<f32> = DelayLine::new(
            source,
            Arc::new(AtomicU32::new(0)),
            hub,
            Arc::new(AtomicU64::new(0)),
        )
        .collect();
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn the_mix_source_applies_the_channel_map() {
        let routes = [
            ChannelRoute {
                source: Some(1),
                gain_percent: 100,
            },
            ChannelRoute {
                source: Some(1),
                gain_percent: 50,
            },
        ];
        let source = SamplesBuffer::new(channels(2), rate(48_000), vec![0.8, -0.4, 0.6, -0.2]);
        let mixer = ChannelMixer::new(Some(&routes), 2, 2);
        let out: Vec<f32> = MixSource::new(source, mixer).collect();
        assert_eq!(out, vec![-0.4, -0.2, -0.2, -0.1]);
        let mono = SamplesBuffer::new(channels(2), rate(48_000), vec![0.5, 0.5, 0.25, 0.25]);
        let mixer = ChannelMixer::new(None, 2, 1);
        let source = MixSource::new(mono, mixer);
        assert_eq!(source.channels().get(), 1);
    }

    /// Feed `seconds` of a sine into a mirror's ring in 10 ms steps and pull the mirror's output the same way.
    /// `producer_rate_error` is how much faster than nominal the producer's clock runs.
    fn run_mirror(
        source_rate: u32,
        device_rate: u32,
        delay_ms: u32,
        producer_rate_error: f64,
        seconds: u32,
    ) -> (Vec<f32>, Arc<MirrorControl>, MirrorSource) {
        let hub = MirrorHub::new();
        hub.configure(1, source_rate);
        let ring = hub.add_ring();
        let control = MirrorControl::new(1.0, delay_ms);
        let mut mirror = MirrorSource::new(
            Arc::clone(&ring),
            Arc::clone(&control),
            1,
            device_rate,
            None,
        );
        let mut produced = 0.0f64;
        let mut written = 0u64;
        let mut consumed_time = 0.0f64;
        let mut output = Vec::new();
        for _ in 0..(seconds * 100) {
            produced += f64::from(source_rate) * 0.01 * (1.0 + producer_rate_error);
            while (written as f64) < produced {
                let value = (2.0 * std::f64::consts::PI * 440.0 * written as f64
                    / f64::from(source_rate))
                .sin() as f32
                    * 0.5;
                ring.push_frame(&[value]);
                written += 1;
            }
            consumed_time += 0.01;
            let wanted = (consumed_time * f64::from(device_rate)) as usize - output.len();
            for _ in 0..wanted {
                output.push(mirror.next().unwrap());
            }
        }
        (output, control, mirror)
    }

    fn dominant_frequency(samples: &[f32], rate: u32) -> f64 {
        let crossings = samples
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        crossings as f64 / (samples.len() as f64 / f64::from(rate))
    }

    #[test]
    fn a_mirror_at_another_rate_keeps_the_pitch() {
        let (output, _, _) = run_mirror(44_100, 48_000, 0, 0.0, 4);
        let tail = &output[output.len() / 2..];
        let frequency = dominant_frequency(tail, 48_000);
        assert!((frequency - 440.0).abs() < 2.0, "{frequency}");
        let peak = tail
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
        assert!((peak - 0.5).abs() < 0.02, "amplitude kept: {peak}");
    }

    #[test]
    fn a_mirror_follows_a_producer_clock_that_runs_fast_or_slow_without_running_dry_or_overflowing()
    {
        for error in [0.0003, -0.0003, 0.002] {
            let (_, control, mut mirror) = run_mirror(48_000, 48_000, 0, error, 30);
            let fill_ms = mirror.reader.available_frames() as f64 * 1000.0 / 48_000.0;
            assert!(
                (fill_ms - f64::from(JITTER_MARGIN_MS)).abs() < 25.0,
                "error {error}: fill {fill_ms} ms"
            );
            assert!(
                control.underruns.load(Ordering::Relaxed) <= 1,
                "error {error}: {} underruns",
                control.underruns.load(Ordering::Relaxed)
            );
        }
    }

    #[test]
    fn the_delay_setting_moves_the_mirror_later_by_that_many_milliseconds() {
        let (without, _, mut mirror_zero) = run_mirror(48_000, 48_000, 0, 0.0, 3);
        let (with, _, mut mirror_delay) = run_mirror(48_000, 48_000, 200, 0.0, 3);
        let first_sound = |samples: &[f32]| {
            samples
                .iter()
                .position(|sample| sample.abs() > 0.01)
                .unwrap()
        };
        let difference_ms =
            (first_sound(&with) as f64 - first_sound(&without) as f64) * 1000.0 / 48_000.0;
        assert!(
            (difference_ms - 200.0).abs() < 15.0,
            "delayed by {difference_ms} ms"
        );
        let fill_zero = mirror_zero.reader.available_frames();
        let fill_delay = mirror_delay.reader.available_frames();
        assert!(
            fill_delay > fill_zero + 8_000,
            "the ring holds the delay: {fill_zero} vs {fill_delay}"
        );
    }

    #[test]
    fn a_mirror_that_cannot_keep_up_goes_silent_and_buffers_again() {
        let hub = MirrorHub::new();
        hub.configure(1, 48_000);
        let ring = hub.add_ring();
        let control = MirrorControl::new(1.0, 0);
        let mut mirror =
            MirrorSource::new(Arc::clone(&ring), Arc::clone(&control), 1, 48_000, None);
        for _ in 0..4_800 {
            ring.push_frame(&[0.5]);
        }
        let played: Vec<f32> = (0..2_000).map(|_| mirror.next().unwrap()).collect();
        assert!(played[1_000..]
            .iter()
            .all(|sample| (sample - 0.5).abs() < 1e-3));
        // Nothing more is written: the mirror drains its buffer, then outputs silence and counts the underrun.
        let rest: Vec<f32> = (0..6_000).map(|_| mirror.next().unwrap()).collect();
        assert!(rest[5_000..].iter().all(|sample| *sample == 0.0));
        assert!(control.underruns.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn the_mirror_gain_changes_smoothly_and_maps_channels() {
        let hub = MirrorHub::new();
        hub.configure(2, 48_000);
        let ring = hub.add_ring();
        let control = MirrorControl::new(1.0, 0);
        let routes = vec![
            ChannelRoute {
                source: Some(1),
                gain_percent: 100,
            },
            ChannelRoute {
                source: None,
                gain_percent: 100,
            },
        ];
        let mut mirror = MirrorSource::new(
            Arc::clone(&ring),
            Arc::clone(&control),
            2,
            48_000,
            Some(routes),
        );
        for _ in 0..3_000 {
            ring.push_frame(&[0.2, 0.6]);
        }
        // A producer that keeps up: one frame in for every frame out.
        let mut pull = |count: usize| -> Vec<f32> {
            (0..count)
                .map(|index| {
                    if index % 2 == 0 {
                        ring.push_frame(&[0.2, 0.6]);
                    }
                    mirror.next().unwrap()
                })
                .collect()
        };
        let first = pull(4_000);
        // Right channel to the left output, silence on the right.
        assert!(
            (first[3_000] - 0.6).abs() < 1e-3 && first[3_001] == 0.0,
            "{} {}",
            first[3_000],
            first[3_001]
        );
        control.set_gain(0.5);
        let later = pull(16_000);
        assert!((later[15_000] - 0.3).abs() < 2e-3, "{}", later[15_000]);
        let step = (later[2] - first[3_998]).abs();
        assert!(step < 0.01, "the gain does not jump: {step}");
    }
}
