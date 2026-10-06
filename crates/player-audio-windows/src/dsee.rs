//! Streaming Sony DSEE HX for 48 kHz / 16-bit-class stereo.
//!
//! The installed filter is a 32-bit DirectShow component. The 64-bit player
//! talks to a small helper that loads it from the fixed Music Center path.
//! The helper is our program; the Sony filter is never copied or embedded.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use symphonia::core::codecs::{CODEC_TYPE_AAC, CODEC_TYPE_MP3, CODEC_TYPE_OPUS, CODEC_TYPE_VORBIS};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::DseeHxState;

/// Sony's installed filter. Playback loads this path and nowhere else.
pub(crate) const FILTER_PATH: &str =
    r"C:\Program Files (x86)\Sony\Music Center\Sony.Earth\OmgDseeHxFilter.ax";
pub(crate) const INSTALL_NOTICE: &str = "請先安裝 Sony Music Center。DSEE HX 只會載入 C:\\Program Files (x86)\\Sony\\Music Center\\Sony.Earth\\OmgDseeHxFilter.ax，播放器不會內含或散佈這個檔案。";
pub(crate) fn plausible_hx_rate(rate: u32) -> bool {
    matches!(rate, 44_100 | 48_000 | 88_200 | 96_000 | 176_400 | 192_000)
}

pub(crate) fn expected_hx_rate(sample_rate: u32) -> u32 {
    // The installed filter keeps the 44.1 kHz family on a 176.4 kHz output
    // even when 96 kHz is requested. 48 kHz and other rates come back at 96 kHz.
    if sample_rate.is_multiple_of(11_025) {
        176_400
    } else {
        96_000
    }
}
const CODEC_MP3: u32 = 2;
const OUTPUT_REQUEST_HZ: u32 = 96_000;
const RING_CAP: usize = 176_400 * 2;

pub(crate) fn filter_installed() -> bool {
    Path::new(FILTER_PATH).is_file()
}

/// True when the source format is allowed to enter the filter.
///
/// Lossy files have no stored bit depth in this player. They qualify when the
/// rate is at most 48 kHz, and the decoded samples are sent as 16-bit PCM.
/// An explicit bit depth above 16, a higher rate, or a non-stereo file does not.
pub(crate) fn eligible(
    sample_rate: u32,
    channels: u16,
    bits_per_sample: Option<u16>,
    lossy: bool,
) -> bool {
    channels == 2
        && sample_rate > 0
        && sample_rate <= 48_000
        && match bits_per_sample {
            Some(bits) => bits > 0 && bits <= 16,
            None => lossy,
        }
}

pub(crate) fn bitrate_kbps(measured_kbps: Option<u32>, lossy: bool) -> u32 {
    match measured_kbps {
        Some(kbps) if (8..=1020).contains(&kbps) => kbps,
        _ if lossy => 320,
        _ => 1021,
    }
}

pub(crate) struct TrackSignal {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration: Option<Duration>,
}

pub(crate) enum Engagement {
    Off,
    Bypass(String),
    Unavailable(String),
    Run { bitrate_kbps: u32, input_rate: u32 },
}

pub(crate) struct DseePlan {
    pub output_rate: u32,
    pub engagement: Engagement,
}

pub(crate) fn plan(enabled: bool, path: &Path, signal: &TrackSignal) -> DseePlan {
    if !enabled {
        return DseePlan {
            output_rate: signal.sample_rate,
            engagement: Engagement::Off,
        };
    }
    if !filter_installed() {
        return DseePlan {
            output_rate: signal.sample_rate,
            engagement: Engagement::Unavailable(INSTALL_NOTICE.to_owned()),
        };
    }
    let probed = probe_file(path);
    let Some(probed) = probed else {
        return DseePlan {
            output_rate: signal.sample_rate,
            engagement: Engagement::Bypass(
                "無法確認這首曲目是否為 48 kHz／16-bit 以下，已略過 DSEE HX。".to_owned(),
            ),
        };
    };
    if !eligible(
        signal.sample_rate,
        signal.channels,
        probed.bits_per_sample,
        probed.lossy,
    ) {
        return DseePlan {
            output_rate: signal.sample_rate,
            engagement: Engagement::Bypass(
                "這首曲目不是 48 kHz／16-bit 以下的雙聲道，DSEE HX 未處理。".to_owned(),
            ),
        };
    }
    DseePlan {
        output_rate: expected_hx_rate(signal.sample_rate),
        engagement: Engagement::Run {
            bitrate_kbps: bitrate_kbps(average_kbps(path, signal.duration), probed.lossy),
            input_rate: signal.sample_rate,
        },
    }
}

pub(crate) fn state_of(engagement: &Engagement) -> (DseeHxState, Option<String>) {
    match engagement {
        Engagement::Off => (DseeHxState::Off, None),
        Engagement::Bypass(notice) => (DseeHxState::Bypassed, Some(notice.clone())),
        Engagement::Unavailable(notice) => (DseeHxState::Unavailable, Some(notice.clone())),
        Engagement::Run { .. } => (DseeHxState::Active, None),
    }
}

struct Probed {
    bits_per_sample: Option<u16>,
    lossy: bool,
}

fn probe_file(path: &Path) -> Option<Probed> {
    let file = File::open(path).ok()?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .ok()?;
    let track = probed.format.default_track()?;
    let codec = track.codec_params.codec;
    let lossy = matches!(
        codec,
        CODEC_TYPE_MP3 | CODEC_TYPE_AAC | CODEC_TYPE_VORBIS | CODEC_TYPE_OPUS
    );
    Some(Probed {
        bits_per_sample: track.codec_params.bits_per_sample.map(|bits| bits as u16),
        lossy,
    })
}

fn average_kbps(path: &Path, duration: Option<Duration>) -> Option<u32> {
    let millis = u64::try_from(duration?.as_millis()).ok()?;
    if millis == 0 {
        return None;
    }
    let bytes = std::fs::metadata(path).ok()?.len();
    u32::try_from(bytes.saturating_mul(8) / millis).ok()
}

struct RingInner {
    samples: VecDeque<f32>,
    ended: bool,
    failed: Option<String>,
}

struct Ring {
    inner: Mutex<RingInner>,
    ready: Condvar,
}

impl Ring {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(RingInner {
                samples: VecDeque::new(),
                ended: false,
                failed: None,
            }),
            ready: Condvar::new(),
        })
    }

    fn push(&self, sample: f32, stop: &AtomicBool) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        while guard.samples.len() >= RING_CAP && !guard.ended && !stop.load(Ordering::Relaxed) {
            let (next, _) = self
                .ready
                .wait_timeout(guard, Duration::from_millis(50))
                .unwrap_or_else(|error| error.into_inner());
            guard = next;
        }
        if guard.ended || stop.load(Ordering::Relaxed) {
            return false;
        }
        guard.samples.push_back(sample);
        self.ready.notify_all();
        true
    }

    fn finish(&self, failed: Option<String>) {
        let mut guard = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        guard.ended = true;
        if guard.failed.is_none() {
            guard.failed = failed;
        }
        self.ready.notify_all();
    }
}

pub(crate) enum OpenError<S> {
    /// The decoder was not consumed.
    Rejected(S, String),
    /// The helper took the decoder, then failed. The caller reopens the file.
    Lost(String),
}

pub(crate) struct DseeSource<S> {
    ring: Arc<Ring>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    feeder: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
    stderr_task: Option<JoinHandle<()>>,
    duration: Option<Duration>,
    output_rate: u32,
    _decoder: PhantomData<S>,
}

impl<S> Drop for DseeSource<S> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = child.kill();
        }
        self.ring.ready.notify_all();
        for handle in [
            self.feeder.take(),
            self.reader.take(),
            self.stderr_task.take(),
        ]
        .into_iter()
        .flatten()
        {
            thread::spawn(move || {
                let _ = handle.join();
            });
        }
    }
}

impl<S> DseeSource<S>
where
    S: Source<Item = Sample> + Send + 'static,
{
    pub(crate) fn open(
        mut decoder: S,
        input_rate: u32,
        bitrate_kbps: u32,
    ) -> Result<Self, OpenError<S>> {
        let duration = decoder.total_duration();
        let helper = match helper_exe() {
            Ok(path) => path,
            Err(message) => return Err(OpenError::Rejected(decoder, message)),
        };
        if !filter_installed() {
            return Err(OpenError::Rejected(decoder, INSTALL_NOTICE.to_owned()));
        }
        let mut command = Command::new(&helper);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return Err(OpenError::Rejected(
                    decoder,
                    format!("無法啟動 DSEE HX 輔助程式：{error}"),
                ))
            }
        };
        let mut stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let _ = child.kill();
                return Err(OpenError::Rejected(
                    decoder,
                    "DSEE HX 輔助程式沒有標準輸入。".to_owned(),
                ));
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let _ = child.kill();
                return Err(OpenError::Rejected(
                    decoder,
                    "DSEE HX 輔助程式沒有標準輸出。".to_owned(),
                ));
            }
        };
        let stderr = child.stderr.take();
        let stop = Arc::new(AtomicBool::new(false));
        let ring = Ring::new();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
        let reader_stop = Arc::clone(&stop);
        let reader_ring = Arc::clone(&ring);
        let reader = match thread::Builder::new()
            .name("dsee-hx-read".to_owned())
            .spawn(move || read_output(stdout, reader_ring, reader_stop, ready_tx))
        {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                return Err(OpenError::Rejected(
                    decoder,
                    format!("無法讀取 DSEE HX 輸出：{error}"),
                ));
            }
        };
        let stderr_task = stderr.map(|stderr| {
            thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stderr.take(4096).read_to_end(&mut sink);
            })
        });
        if let Err(error) = write_message(&mut stdin, 1, &config_payload(input_rate, bitrate_kbps))
        {
            let _ = child.kill();
            return Err(OpenError::Rejected(
                decoder,
                format!("無法設定 DSEE HX：{error}"),
            ));
        }
        let output_rate = match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(rate)) if plausible_hx_rate(rate) => rate,
            Ok(Err(message)) => {
                let _ = child.kill();
                return Err(OpenError::Rejected(decoder, message));
            }
            Ok(Ok(_)) => {
                let _ = child.kill();
                return Err(OpenError::Rejected(
                    decoder,
                    "DSEE HX 沒有提供可用的輸出格式。".to_owned(),
                ));
            }
            Err(_) => {
                let _ = child.kill();
                return Err(OpenError::Rejected(
                    decoder,
                    "DSEE HX 沒有在時限內開始處理。".to_owned(),
                ));
            }
        };
        // Send the first samples on this thread. The helper is already waiting
        // for PCM, and playback can keep the audio callback busy enough that a
        // newly spawned feeder does not deliver them before the prefill deadline.
        let prime = pull_pcm(&mut decoder, 2048);
        if !prime.is_empty() && write_message(&mut stdin, 2, &prime).is_err() {
            let _ = child.kill();
            return Err(OpenError::Lost("DSEE HX 輔助程式已中斷。".to_owned()));
        }
        let feeder_stop = Arc::clone(&stop);
        let feeder_ring = Arc::clone(&ring);
        let feeder = match thread::Builder::new()
            .name("dsee-hx-feed".to_owned())
            .spawn(move || feed(decoder, stdin, feeder_stop, feeder_ring))
        {
            Ok(feeder) => feeder,
            Err(error) => {
                let _ = child.kill();
                return Err(OpenError::Lost(format!("無法送出 DSEE HX 音訊：{error}")));
            }
        };
        let child = Arc::new(Mutex::new(Some(child)));
        let source = Self {
            ring: Arc::clone(&ring),
            stop,
            child,
            feeder: Some(feeder),
            reader: Some(reader),
            stderr_task,
            duration,
            output_rate,
            _decoder: PhantomData,
        };
        if let Err(message) = wait_prefill(&ring, &source.stop, output_rate) {
            return Err(OpenError::Lost(message));
        }
        Ok(source)
    }
}

fn wait_prefill(ring: &Ring, stop: &AtomicBool, output_rate: u32) -> Result<(), String> {
    let target = (usize::try_from(output_rate / 100).unwrap_or(960))
        .saturating_mul(2)
        .max(64);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut guard = ring.inner.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        if let Some(message) = guard.failed.clone() {
            return Err(message);
        }
        if guard.samples.len() >= target || guard.ended {
            return if guard.samples.is_empty() && guard.ended {
                Err("DSEE HX 沒有產生音訊。".to_owned())
            } else {
                Ok(())
            };
        }
        if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
            return Err("DSEE HX 沒有及時送出音訊，已改以原解碼播放。".to_owned());
        }
        let (next, _) = ring
            .ready
            .wait_timeout(guard, Duration::from_millis(20))
            .unwrap_or_else(|error| error.into_inner());
        guard = next;
    }
}

fn pull_pcm<S>(decoder: &mut S, frames: usize) -> Vec<u8>
where
    S: Source<Item = Sample>,
{
    let mut payload = Vec::with_capacity(frames * 4);
    for _ in 0..frames {
        let Some(left) = decoder.next() else {
            break;
        };
        let Some(right) = decoder.next() else {
            break;
        };
        payload.extend_from_slice(&quantize(left).to_le_bytes());
        payload.extend_from_slice(&quantize(right).to_le_bytes());
    }
    payload
}

fn feed<S>(mut decoder: S, mut stdin: impl Write, stop: Arc<AtomicBool>, ring: Arc<Ring>)
where
    S: Source<Item = Sample>,
{
    let mut payload = Vec::with_capacity(4096);
    while !stop.load(Ordering::Relaxed) {
        payload.clear();
        for _ in 0..1024 {
            let Some(left) = decoder.next() else {
                break;
            };
            let Some(right) = decoder.next() else {
                break;
            };
            if stop.load(Ordering::Relaxed) {
                return;
            }
            payload.extend_from_slice(&quantize(left).to_le_bytes());
            payload.extend_from_slice(&quantize(right).to_le_bytes());
        }
        if payload.is_empty() {
            break;
        }
        if write_message(&mut stdin, 2, &payload).is_err() {
            ring.finish(Some("DSEE HX 輔助程式已中斷。".to_owned()));
            return;
        }
    }
    let _ = write_message(&mut stdin, 3, &[]);
}

fn read_output(
    mut stdout: impl Read,
    ring: Arc<Ring>,
    stop: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<u32, String>>,
) {
    let mut announced = false;
    let mut pending = Vec::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            ring.finish(None);
            return;
        }
        let message = match read_message(&mut stdout) {
            Ok(message) => message,
            Err(_) => {
                if !announced {
                    let _ = ready.send(Err("DSEE HX 輔助程式已結束。".to_owned()));
                }
                ring.finish(Some("DSEE HX 輔助程式已結束。".to_owned()));
                return;
            }
        };
        match message.kind {
            1 if !announced && message.payload.len() == 12 => {
                announced = true;
                let rate = u32::from_le_bytes(message.payload[0..4].try_into().unwrap());
                let _ = ready.send(Ok(rate));
            }
            4 => {
                let message = decode_error(&message.payload);
                if !announced {
                    let _ = ready.send(Err(message));
                } else {
                    ring.finish(Some(message));
                }
                return;
            }
            2 => {
                pending.extend_from_slice(&message.payload);
                let mut offset = 0;
                while pending.len() - offset >= 6 {
                    let left = i24_to_f32(&pending[offset..offset + 3]);
                    let right = i24_to_f32(&pending[offset + 3..offset + 6]);
                    offset += 6;
                    if !ring.push(left, &stop) || !ring.push(right, &stop) {
                        ring.finish(None);
                        return;
                    }
                }
                if offset > 0 {
                    pending.drain(0..offset);
                }
            }
            3 => {
                ring.finish(None);
                return;
            }
            _ => {}
        }
    }
}

struct PipeMessage {
    kind: u32,
    payload: Vec<u8>,
}

fn write_message(writer: &mut impl Write, kind: u32, payload: &[u8]) -> std::io::Result<()> {
    writer.write_all(&kind.to_le_bytes())?;
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()?;
    Ok(())
}

fn read_message(reader: &mut impl Read) -> std::io::Result<PipeMessage> {
    let mut header = [0u8; 8];
    reader.read_exact(&mut header)?;
    let kind = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let size = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if size > 1024 * 1024 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "DSEE HX message is too large",
        ));
    }
    let mut payload = vec![0u8; size as usize];
    if size > 0 {
        reader.read_exact(&mut payload)?;
    }
    Ok(PipeMessage { kind, payload })
}

fn config_payload(input_rate: u32, bitrate_kbps: u32) -> [u8; 32] {
    let fields = [
        2u32,
        CODEC_MP3,
        input_rate,
        OUTPUT_REQUEST_HZ,
        16,
        24,
        0,
        bitrate_kbps,
    ];
    let mut payload = [0u8; 32];
    for (index, field) in fields.iter().enumerate() {
        payload[index * 4..index * 4 + 4].copy_from_slice(&field.to_le_bytes());
    }
    payload
}

fn decode_error(payload: &[u8]) -> String {
    if payload.len() >= 8 {
        let code = u32::from_le_bytes(payload[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(payload[4..8].try_into().unwrap()) as usize;
        if code == 1 {
            return INSTALL_NOTICE.to_owned();
        }
        if payload.len() >= 8 + len {
            let detail = String::from_utf8_lossy(&payload[8..8 + len]);
            return format!("DSEE HX 無法處理這首曲目：{detail}");
        }
    }
    "DSEE HX 無法處理這首曲目。".to_owned()
}

fn quantize(sample: f32) -> i16 {
    if !sample.is_finite() {
        return 0;
    }
    let scaled = (sample.clamp(-1.0, 1.0) * 32767.0).round();
    scaled as i16
}

fn i24_to_f32(bytes: &[u8]) -> f32 {
    let mut value = i32::from(bytes[0]) | (i32::from(bytes[1]) << 8) | (i32::from(bytes[2]) << 16);
    if value & 0x0080_0000 != 0 {
        value |= !0x00FF_FFFF;
    }
    value as f32 / 8_388_608.0
}

fn helper_exe() -> Result<PathBuf, String> {
    #[cfg(not(dsee_helper))]
    {
        Err("DSEE HX 輔助程式沒有隨這個版本建置，已改以原解碼播放。".to_owned())
    }
    #[cfg(dsee_helper)]
    {
        const BYTES: &[u8] = include_bytes!(env!("DSEE_HELPER_PATH"));
        let beside = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        if let Some(directory) = beside {
            let path = directory.join("dsee_hx_stream.exe");
            if write_if_changed(&path, BYTES).is_ok() {
                return Ok(path);
            }
        }
        let path = std::env::temp_dir().join("moemusicplayer-dsee_hx_stream.exe");
        write_if_changed(&path, BYTES)
            .map_err(|error| format!("無法寫入 DSEE HX 輔助程式：{error}"))?;
        Ok(path)
    }
}

#[cfg(dsee_helper)]
fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    std::fs::write(path, bytes)
}

impl<S> Iterator for DseeSource<S>
where
    S: Source<Item = Sample> + Send,
{
    type Item = Sample;

    fn next(&mut self) -> Option<Sample> {
        let mut guard = self
            .ring
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(sample) = guard.samples.pop_front() {
                self.ring.ready.notify_all();
                return Some(sample);
            }
            if guard.ended {
                return None;
            }
            let (next, timeout) = self
                .ring
                .ready
                .wait_timeout(guard, Duration::from_millis(8))
                .unwrap_or_else(|error| error.into_inner());
            guard = next;
            if guard.samples.is_empty() && timeout.timed_out() && !guard.ended {
                return Some(0.0);
            }
        }
    }
}

impl<S> Source for DseeSource<S>
where
    S: Source<Item = Sample> + Send,
{
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        ChannelCount::new(2).expect("stereo")
    }

    fn sample_rate(&self) -> SampleRate {
        SampleRate::new(self.output_rate).expect("DSEE output rate")
    }

    fn total_duration(&self) -> Option<Duration> {
        self.duration
    }

    fn try_seek(&mut self, _position: Duration) -> Result<(), SeekError> {
        Err(SeekError::NotSupported {
            underlying_source: "DSEE HX",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_keeps_cd_and_lossy_and_skips_hires() {
        assert!(eligible(44_100, 2, Some(16), false));
        assert!(eligible(48_000, 2, Some(16), false));
        assert!(eligible(48_000, 2, Some(8), false));
        assert!(eligible(44_100, 2, None, true));
        assert!(!eligible(48_000, 2, Some(24), false));
        assert!(!eligible(96_000, 2, Some(16), false));
        assert!(!eligible(44_100, 1, Some(16), false));
        assert!(!eligible(44_100, 2, None, false));
        assert!(!eligible(0, 2, Some(16), false));
        assert_eq!(expected_hx_rate(44_100), 176_400);
        assert_eq!(expected_hx_rate(22_050), 176_400);
        assert_eq!(expected_hx_rate(48_000), 96_000);
        assert_eq!(expected_hx_rate(32_000), 96_000);
        assert!(plausible_hx_rate(176_400));
        assert!(!plausible_hx_rate(9_365_152));
    }

    #[test]
    fn bitrate_uses_a_measured_value_inside_the_filter_range() {
        assert_eq!(bitrate_kbps(Some(320), true), 320);
        assert_eq!(bitrate_kbps(None, true), 320);
        assert_eq!(bitrate_kbps(Some(5_000), false), 1021);
        assert_eq!(bitrate_kbps(Some(1411), false), 1021);
    }

    #[test]
    fn installed_filter_streams_a_short_tone() {
        if !filter_installed() {
            return;
        }
        let rate = 44_100u32;
        let frames = rate / 5;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|frame| {
                let sample =
                    (2.0 * std::f32::consts::PI * 440.0 * frame as f32 / rate as f32).sin() * 0.2;
                [sample, sample]
            })
            .collect();
        let decoder = rodio::buffer::SamplesBuffer::new(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(rate).unwrap(),
            samples,
        );
        let source = match DseeSource::open(decoder, rate, 320) {
            Ok(source) => source,
            Err(OpenError::Rejected(_, message) | OpenError::Lost(message)) => {
                panic!("dsee stream failed: {message}")
            }
        };
        assert_eq!(source.sample_rate().get(), 176_400);
        let output: Vec<f32> = source.take(rate as usize).collect();
        assert!(
            output.iter().any(|sample| sample.abs() > 0.001),
            "expected audible samples from DSEE HX, got silence"
        );
    }

    #[test]
    #[ignore = "opens the default output device and plays a short tone"]
    fn installed_filter_streams_while_wasapi_is_playing() {
        if !filter_installed() {
            return;
        }
        use rodio::cpal::traits::HostTrait;
        let device = rodio::cpal::default_host()
            .default_output_device()
            .expect("default output");
        let sink = rodio::DeviceSinkBuilder::from_device(device)
            .expect("sink builder")
            .open_sink_or_fallback()
            .expect("output stream");
        let player = rodio::Player::connect_new(sink.mixer());
        let playing: Vec<f32> = (0..48_000 * 2)
            .flat_map(|frame| {
                let sample =
                    (2.0 * std::f32::consts::PI * 220.0 * frame as f32 / 48_000.0).sin() * 0.05;
                [sample, sample]
            })
            .collect();
        player.append(rodio::buffer::SamplesBuffer::new(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(48_000).unwrap(),
            playing,
        ));
        player.play();
        std::thread::sleep(Duration::from_millis(200));
        let rate = 48_000u32;
        let frames = rate / 5;
        let samples: Vec<f32> = (0..frames)
            .flat_map(|frame| {
                let sample =
                    (2.0 * std::f32::consts::PI * 440.0 * frame as f32 / rate as f32).sin() * 0.2;
                [sample, sample]
            })
            .collect();
        let decoder = rodio::buffer::SamplesBuffer::new(
            ChannelCount::new(2).unwrap(),
            SampleRate::new(rate).unwrap(),
            samples,
        );
        let source = match DseeSource::open(decoder, rate, 320) {
            Ok(source) => source,
            Err(OpenError::Rejected(_, message) | OpenError::Lost(message)) => {
                panic!("dsee stream during playback failed: {message}")
            }
        };
        let output: Vec<f32> = source.take(2_000).collect();
        assert!(output.iter().any(|sample| sample.abs() > 0.001));
        player.stop();
    }

    #[test]
    fn error_code_for_a_missing_filter_asks_for_music_center() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&7u32.to_le_bytes());
        payload.extend_from_slice(b"missing");
        assert_eq!(decode_error(&payload), INSTALL_NOTICE);
    }
}
