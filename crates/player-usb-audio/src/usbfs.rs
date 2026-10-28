//! The exclusive output over Linux usbfs (ADR 0046 P0, ADR 0047 P1). Android and Linux only.
//!
//! One thread ([`run`]) owns the device for one claim: it takes requests from the output (attach a ring, play, pause,
//! detach, release), feeds them and the time to the pure state machine (`engine::step`) and carries out its actions:
//! claim (detach the kernel driver from every audio interface, claim them), configure (alt 0, GET RANGE, the alt for
//! the format, SET CUR the rate, settle, GET CUR read-back: the order Linux `snd-usb-audio` uses for UAC2, verified in
//! ADR 0046), stream (feedback and data URBs in flight, reaped on this thread with poll + non-blocking reap; every
//! packet is filled from the ring, packed with the gain, and with silence where the ring has nothing), stop (discard and
//! reap every URB, rewind what was not played, alt 0) and hand back (alt 0, release, let the kernel driver bind
//! again). The hand-back also runs from `Drop`, so an early return or a panic gives the device back too.

use std::io;
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::descriptors::{parse_configuration, Configuration, StreamPlan};
use crate::diagnostics::{DataUrbStats, FeedbackUrbStats};
use crate::engine::{self, Action, Event, State, StreamFormat};
use crate::feedback::Feedback;
use crate::format::{self, NotBitPerfect, Source};
use crate::ioctl::{self, CtrlTransfer, IoctlRequest, IsoPacketDesc, SetInterface, Urb};
use crate::packets::PacketSizer;
use crate::pcm::{self, Container};
use crate::ring::{Cursor, Ring};
use crate::uac::{self, ControlRequest};
use crate::{AttachReply, EngineStatus};

/// Data URBs kept in flight and isochronous packets (microframes at high speed) per URB: 16 × 8 = 16 ms queued at
/// high speed, enough to ride out a scheduling hiccup of the thread (no underrun in ADR 0046's runs).
pub const DATA_URBS: usize = 16;
pub const PACKETS_PER_URB: usize = 8;
const FEEDBACK_URBS: usize = 2;
const CONTROL_TIMEOUT_MS: u32 = 1000;
/// How long one wait for completed URBs may last while streaming (requests are read between waits).
const STREAM_POLL_MS: c_int = 20;
/// How long the thread sleeps for a request while not streaming (the idle timers are checked between).
const IDLE_WAIT: Duration = Duration::from_millis(200);
/// How long the stop and the hand-back wait for discarded URBs to come back before leaving their memory to the process.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// After the clock is set, before it is read back and streaming starts.
const CLOCK_SETTLE: Duration = Duration::from_millis(50);
/// The status keeps this many usbfs steps.
const STEPS_KEPT: usize = 64;

const FEEDBACK_TAG: usize = 0x1000;

/// What the playback thread asks the device thread. Every request answers on its own channel once it was carried out.
pub enum Request {
    /// A new output plays `ring` at `rate`: choose the alt, claim or reconfigure as needed.
    Attach {
        ring: Arc<Ring>,
        rate: u32,
        source: Source,
        reply: SyncSender<Result<AttachReply, String>>,
    },
    Play {
        ring: usize,
        reply: SyncSender<Result<(), String>>,
    },
    Pause {
        ring: usize,
        reply: SyncSender<Result<(), String>>,
    },
    Detach {
        ring: usize,
        reply: SyncSender<Result<(), String>>,
    },
    /// Give the device back now (stop, the setting turned off, the service ending).
    Release {
        reason: String,
        reply: SyncSender<Result<(), String>>,
    },
}

/// A ring's identity in requests (the playback side holds the same `Arc`).
pub fn ring_id(ring: &Arc<Ring>) -> usize {
    Arc::as_ptr(ring) as usize
}

/// Shared between the device thread and the playback side.
#[derive(Default)]
pub struct Shared {
    /// The app volume as `f32` bits, applied before packing.
    gain_bits: AtomicU32,
    status: Mutex<EngineStatus>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            gain_bits: AtomicU32::new(1.0_f32.to_bits()),
            status: Mutex::new(EngineStatus {
                state: "unclaimed".into(),
                gain: 1.0,
                ..EngineStatus::default()
            }),
        }
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain_bits
            .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain_bits.load(Ordering::Relaxed))
    }

    pub fn status(&self) -> EngineStatus {
        let mut status = self
            .status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // The volume can change at any time: the bit-perfect answer follows it.
        let gain = self.gain();
        status.gain = gain;
        status
            .not_bit_perfect
            .retain(|reason| *reason != NotBitPerfect::Volume);
        if gain != 1.0 {
            status.not_bit_perfect.push(NotBitPerfect::Volume);
        }
        status.bit_perfect = status.state == "streaming" && status.not_bit_perfect.is_empty();
        status
    }

    fn update(&self, change: impl FnOnce(&mut EngineStatus)) {
        change(&mut self.status.lock().unwrap_or_else(PoisonError::into_inner));
    }
}

fn ioctl_raw<T>(fd: c_int, request: u32, arg: *mut T) -> io::Result<c_int> {
    // SAFETY: every caller passes a request whose argument type is `T` (see `ioctl`), pointing at live memory.
    let result = unsafe { libc::ioctl(fd, request as _, arg) };
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}

fn errno(error: &io::Error) -> Option<i32> {
    error.raw_os_error()
}

#[repr(C)]
struct IsoUrb<const N: usize> {
    urb: Urb,
    packets: [IsoPacketDesc; N],
}

struct Slot<const N: usize> {
    urb: Box<IsoUrb<N>>,
    buffer: Vec<u8>,
    in_flight: bool,
    /// The ring whose bytes this URB carries, and how many input bytes each packet took from it.
    ring: Option<Arc<Ring>>,
    credits: [usize; N],
}

impl<const N: usize> Slot<N> {
    fn new(tag: usize, endpoint: u8, packet_bytes: usize) -> Self {
        Self {
            urb: Box::new(IsoUrb {
                urb: Urb {
                    kind: ioctl::URB_TYPE_ISO,
                    endpoint,
                    status: 0,
                    flags: ioctl::URB_ISO_ASAP,
                    buffer: std::ptr::null_mut(),
                    buffer_length: 0,
                    actual_length: 0,
                    start_frame: 0,
                    number_of_packets: N as c_int,
                    error_count: 0,
                    signal: 0,
                    user_context: tag as *mut c_void,
                },
                packets: [IsoPacketDesc::default(); N],
            }),
            buffer: vec![0; packet_bytes * N],
            in_flight: false,
            ring: None,
            credits: [0; N],
        }
    }

    fn pointer(&mut self) -> *mut Urb {
        &mut self.urb.urb
    }

    /// The bus is done with this URB: its ring bytes count as played. `only_sent`: credit only packets that went out
    /// (a discarded URB), else every packet (time passed for them, an errored packet's audio is lost).
    fn credit(&mut self, only_sent: bool) {
        if let Some(ring) = self.ring.take() {
            for (packet, credit) in self.urb.packets.iter().zip(self.credits) {
                if !only_sent || (packet.status == 0 && packet.actual_length > 0) {
                    Cursor::complete(&ring, credit);
                }
            }
        }
        self.credits = [0; N];
    }
}

/// The URBs and pacing of a running stream.
struct Stream {
    data: Vec<Slot<PACKETS_PER_URB>>,
    feedback: Vec<Slot<1>>,
    sizer: PacketSizer,
    parser: Feedback,
    device_frame_bytes: usize,
    container: Container,
    scratch: Vec<u8>,
    feedback_urbs: FeedbackUrbStats,
    data_urbs: DataUrbStats,
}

/// The ring the current output writes, with the device thread's cursor on it.
struct Attached {
    ring: Arc<Ring>,
    cursor: Cursor,
    source: Source,
    /// Some bytes of this ring went out: an empty ring after that is an underrun (until then it is the start).
    started: bool,
    starved: bool,
}

struct Device {
    fd: c_int,
    /// Audio interfaces the kernel driver was told to leave (each gets USBDEVFS_CONNECT back).
    detached: Vec<u32>,
    claimed: Vec<u32>,
    /// The streaming interface once it may be on another alt than 0.
    streaming: Option<u32>,
    handed_back: bool,
    steps: Vec<String>,
}

impl Device {
    fn step(&mut self, text: String) {
        if self.steps.len() >= STEPS_KEPT {
            self.steps.remove(0);
        }
        self.steps.push(text);
    }

    fn sub_ioctl(&self, interface: u32, code: u32) -> io::Result<c_int> {
        let mut request = IoctlRequest {
            interface: interface as c_int,
            ioctl_code: code as c_int,
            data: std::ptr::null_mut(),
        };
        ioctl_raw(self.fd, ioctl::IOCTL, &mut request)
    }

    fn take_over(&mut self, interfaces: &[u8]) -> Result<(), String> {
        for &interface in interfaces {
            let interface = u32::from(interface);
            if self.claimed.contains(&interface) {
                continue;
            }
            self.detached.push(interface);
            match self.sub_ioctl(interface, ioctl::DISCONNECT) {
                Ok(_) => self.step(format!("detach {interface}: ok")),
                // No driver was bound to it.
                Err(error) if errno(&error) == Some(libc::ENODATA) => {
                    self.step(format!("detach {interface}: no driver"))
                }
                Err(error) => {
                    self.step(format!("detach {interface}: {error}"));
                    return Err(format!("無法讓系統驅動程式放開介面 {interface}：{error}"));
                }
            }
            let mut number: c_uint = interface;
            match ioctl_raw(self.fd, ioctl::CLAIM_INTERFACE, &mut number) {
                Ok(_) => {
                    self.claimed.push(interface);
                    self.step(format!("claim {interface}: ok"));
                }
                Err(error) => {
                    self.step(format!("claim {interface}: {error}"));
                    return Err(format!("無法取得介面 {interface}：{error}"));
                }
            }
        }
        Ok(())
    }

    fn set_interface(&mut self, interface: u32, alt: u32) -> io::Result<()> {
        let mut request = SetInterface {
            interface,
            alt_setting: alt,
        };
        let result = ioctl_raw(self.fd, ioctl::SET_INTERFACE, &mut request).map(|_| ());
        self.step(format!(
            "set interface {interface} alt {alt}: {}",
            result
                .as_ref()
                .map_or_else(ToString::to_string, |()| "ok".to_owned())
        ));
        result
    }

    fn control(&mut self, request: &mut ControlRequest) -> io::Result<usize> {
        let mut transfer = CtrlTransfer {
            request_type: request.request_type,
            request: request.request,
            value: request.value,
            index: request.index,
            length: request.data.len() as u16,
            timeout: CONTROL_TIMEOUT_MS,
            data: request.data.as_mut_ptr().cast(),
        };
        let result =
            ioctl_raw(self.fd, ioctl::CONTROL, &mut transfer).map(|count| count.max(0) as usize);
        self.step(format!(
            "control {:02X} {:02X} {:04X} {:04X} [{}]: {}",
            request.request_type,
            request.request,
            request.value,
            request.index,
            request.data.len(),
            match &result {
                Ok(count) => format!("{count} byte"),
                Err(error) => error.to_string(),
            }
        ));
        result
    }

    /// Wait until a completed URB can be reaped (or `timeout_ms`); an error when the device is gone.
    fn wait(&self, timeout_ms: c_int) -> io::Result<()> {
        let mut poll = libc::pollfd {
            fd: self.fd,
            events: libc::POLLOUT | libc::POLLWRNORM,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        let result = unsafe { libc::poll(&mut poll, 1, timeout_ms) };
        if result < 0 {
            let error = io::Error::last_os_error();
            if errno(&error) == Some(libc::EINTR) {
                return Ok(());
            }
            return Err(error);
        }
        if poll.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
            return Err(io::Error::from_raw_os_error(libc::ENODEV));
        }
        Ok(())
    }

    /// A completed URB's tag, `None` when nothing is ready.
    fn reap(&self) -> io::Result<Option<usize>> {
        let mut pointer: *mut Urb = std::ptr::null_mut();
        match ioctl_raw(self.fd, ioctl::REAP_URB_NDELAY, &mut pointer) {
            // SAFETY: the kernel hands back a pointer we submitted; the slot that owns it is still alive.
            Ok(_) if !pointer.is_null() => Ok(Some(unsafe { (*pointer).user_context } as usize)),
            Ok(_) => Ok(None),
            Err(error) if errno(&error) == Some(libc::EAGAIN) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Give the device back: alt 0, release, let the kernel driver bind again. Runs once; the URBs must be gone.
    fn hand_back(&mut self) -> Vec<String> {
        if self.handed_back {
            return Vec::new();
        }
        self.handed_back = true;
        let mut errors = Vec::new();
        if let Some(interface) = self.streaming.take() {
            if let Err(error) = self.set_interface(interface, 0) {
                errors.push(format!("介面 {interface} 回到 alt 0 失敗：{error}"));
            }
        }
        for interface in std::mem::take(&mut self.claimed) {
            let mut number: c_uint = interface;
            let result = ioctl_raw(self.fd, ioctl::RELEASE_INTERFACE, &mut number);
            self.step(format!(
                "release {interface}: {}",
                result
                    .as_ref()
                    .map_or_else(ToString::to_string, |_| "ok".to_owned())
            ));
            if let Err(error) = result {
                errors.push(format!("釋放介面 {interface} 失敗：{error}"));
            }
        }
        for interface in std::mem::take(&mut self.detached) {
            match self.sub_ioctl(interface, ioctl::CONNECT) {
                Ok(_) => self.step(format!("reattach {interface}: ok")),
                // The driver bound it already (it claims every interface of the function from the control one).
                Err(error) if errno(&error) == Some(libc::EBUSY) => {
                    self.step(format!("reattach {interface}: already bound"))
                }
                Err(error) => {
                    self.step(format!("reattach {interface}: {error}"));
                    errors.push(format!("介面 {interface} 交還系統驅動程式失敗：{error}"));
                }
            }
        }
        errors
    }
}

fn submit<const N: usize>(fd: c_int, slot: &mut Slot<N>) -> io::Result<()> {
    slot.urb.urb.buffer = slot.buffer.as_mut_ptr().cast();
    slot.urb.urb.status = 0;
    slot.urb.urb.actual_length = 0;
    slot.urb.urb.error_count = 0;
    slot.urb.urb.start_frame = 0;
    slot.urb.urb.flags = ioctl::URB_ISO_ASAP;
    slot.urb.urb.number_of_packets = N as c_int;
    slot.urb.urb.buffer_length = slot.urb.packets.iter().map(|p| p.length as c_int).sum();
    for packet in &mut slot.urb.packets {
        packet.actual_length = 0;
        packet.status = 0;
    }
    let pointer = slot.pointer();
    ioctl_raw(fd, ioctl::SUBMIT_URB, pointer)?;
    slot.in_flight = true;
    Ok(())
}

/// USB speed from the kernel, or a guess from bcdUSB when the ioctl is refused.
fn high_speed(fd: c_int, bcd_usb: u16) -> bool {
    match ioctl_raw::<c_void>(fd, ioctl::GET_SPEED, std::ptr::null_mut()) {
        Ok(speed) => speed >= ioctl::SPEED_HIGH,
        Err(_) => bcd_usb >= 0x0200,
    }
}

struct Runtime<'a> {
    device: Device,
    config: Configuration,
    shared: &'a Shared,
    high_speed: bool,
    started_at: Instant,
    state: State,
    /// The plan of the format the device is configured (or being configured) for.
    plan: Option<StreamPlan>,
    attached: Option<Attached>,
    stream: Option<Stream>,
    rate_read_back: Option<u32>,
    underruns: u64,
    urb_errors: u64,
    packet_errors: u64,
    error: Option<String>,
    hand_back_errors: Vec<String>,
}

impl Runtime<'_> {
    fn now_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    fn publish(&mut self) {
        let format = engine::format_of(self.state);
        let attached = self.attached.as_ref();
        let source = attached.map(|a| a.source).unwrap_or_default();
        let not_bit_perfect = match (attached, self.stream.as_ref()) {
            (Some(attached), Some(stream)) => format::bit_perfect(
                source,
                attached.ring.encoding(),
                stream.container,
                self.shared.gain(),
            ),
            _ => Vec::new(),
        };
        let feedback_avg = self
            .stream
            .as_ref()
            .and_then(|stream| stream.parser.stats().avg);
        let state = engine::state_name(self.state).to_owned();
        let steps = self.device.steps.clone();
        let handed_back =
            (self.state == State::HandedBack).then_some(self.hand_back_errors.is_empty());
        let (underruns, urb_errors, packet_errors) =
            (self.underruns, self.urb_errors, self.packet_errors);
        let (vendor_id, product_id, high) = (
            self.config.vendor_id,
            self.config.product_id,
            self.high_speed,
        );
        let error = self.error.clone();
        let hand_back_errors = self.hand_back_errors.clone();
        let rate_read_back = self.rate_read_back;
        self.shared.update(|status| {
            *status = EngineStatus {
                state,
                vendor_id: Some(vendor_id),
                product_id: Some(product_id),
                high_speed: Some(high),
                rate: format.map(|f| f.rate),
                rate_read_back,
                channels: format.map(|f| f.channels),
                alt: format.map(|f| f.alt),
                subslot_bytes: format.map(|f| f.subslot_bytes),
                bits: format.map(|f| f.bits),
                source_bits: source.bits,
                dsd: source.dsd,
                not_bit_perfect,
                bit_perfect: false,
                gain: status.gain,
                underruns,
                feedback_avg,
                urb_errors,
                packet_errors,
                error,
                handed_back,
                hand_back_errors,
                steps,
            };
        });
    }

    /// Feed `event` to the state machine and carry out its actions; a failed action gives the device back.
    fn handle(&mut self, event: Event) -> Result<(), String> {
        let (next, actions) = engine::step(self.state, event, self.now_ms());
        self.state = next;
        for action in actions {
            if let Err(error) = self.execute(action) {
                self.fail(error.clone());
                return Err(error);
            }
        }
        Ok(())
    }

    /// A step failed or the device went away: stop, give the device back, and stall the attached output (it falls back).
    fn fail(&mut self, error: String) {
        self.device.step(format!("error: {error}"));
        self.error = Some(error);
        self.state = State::HandedBack;
        self.stop_streaming();
        self.hand_back_errors = self.device.hand_back();
        if let Some(attached) = &self.attached {
            attached.ring.fail();
        }
    }

    fn execute(&mut self, action: Action) -> Result<(), String> {
        if action == Action::HandBack {
            self.stop_streaming();
            self.hand_back_errors = self.device.hand_back();
            return Ok(());
        }
        let plan = self.plan.clone().ok_or("沒有串流設定")?;
        match action {
            Action::Claim => self.device.take_over(&plan.audio_interfaces),
            Action::Configure(format) => self.configure(&plan, format.rate),
            Action::StartStreaming => self.start_streaming(&plan),
            Action::StopStreaming => {
                self.stop_streaming();
                let interface = u32::from(plan.streaming_interface);
                self.device
                    .set_interface(interface, 0)
                    .map_err(|error| format!("介面 {interface} 無法回到 alt 0：{error}"))
            }
            Action::HandBack => Ok(()),
        }
    }

    /// Alt 0, GET RANGE (UAC2), the alt, SET CUR, settle, GET CUR (ADR 0046 order).
    fn configure(&mut self, plan: &StreamPlan, rate: u32) -> Result<(), String> {
        let interface = u32::from(plan.streaming_interface);
        self.device
            .set_interface(interface, 0)
            .map_err(|error| format!("介面 {interface} 無法回到 alt 0：{error}"))?;
        self.check_rate_range(plan, rate)?;
        // From here the streaming interface may be on another alt: the hand-back puts it to alt 0.
        self.device.streaming = Some(interface);
        let alt = plan.alt;
        self.device
            .set_interface(interface, u32::from(alt))
            .map_err(|error| format!("無法切到 alt {alt}：{error}"))?;
        self.set_clock(plan, rate)
    }

    /// UAC2: which rates the clock takes (remembered for this device). A refused RANGE request is not fatal (the
    /// read-back still guards).
    fn check_rate_range(&mut self, plan: &StreamPlan, rate: u32) -> Result<(), String> {
        let Some(clock) = plan.clock_id else {
            return Ok(());
        };
        let (vendor, product) = (self.config.vendor_id, self.config.product_id);
        let ranges = match format::known_ranges(vendor, product) {
            Some(ranges) => ranges,
            None => {
                let mut head = uac::uac2_get_rate_range(clock, plan.ac_interface, 2);
                let mut ranges = Vec::new();
                if self.device.control(&mut head).is_ok() {
                    if let Some(length) = uac::uac2_range_length(&head.data) {
                        let mut full = uac::uac2_get_rate_range(clock, plan.ac_interface, length);
                        if let Ok(count) = self.device.control(&mut full) {
                            ranges = uac::parse_uac2_rate_ranges(
                                &full.data[..count.min(full.data.len())],
                            );
                        }
                    }
                }
                if !ranges.is_empty() {
                    format::remember_ranges(vendor, product, ranges.clone());
                }
                ranges
            }
        };
        if !ranges.is_empty() && !uac::rate_in_ranges(&ranges, rate) {
            return Err(format!("USB 音訊裝置不支援 {rate} Hz"));
        }
        Ok(())
    }

    /// Set the clock, let it settle and read it back; a read-back that is not the rate is an error.
    fn set_clock(&mut self, plan: &StreamPlan, rate: u32) -> Result<(), String> {
        let read_back = if let Some(clock) = plan.clock_id {
            let mut set = uac::uac2_set_rate(clock, plan.ac_interface, rate);
            self.device
                .control(&mut set)
                .map_err(|error| format!("設定時脈 {rate} Hz 失敗：{error}"))?;
            std::thread::sleep(CLOCK_SETTLE);
            let mut get = uac::uac2_get_rate(clock, plan.ac_interface);
            let count = self
                .device
                .control(&mut get)
                .map_err(|error| format!("讀回時脈失敗：{error}"))?;
            uac::parse_rate(&get.data[..count.min(4)])
        } else {
            let endpoint = plan.data_endpoint.address;
            let mut set = uac::uac1_set_rate(endpoint, rate);
            self.device
                .control(&mut set)
                .map_err(|error| format!("設定取樣率 {rate} Hz 失敗：{error}"))?;
            std::thread::sleep(CLOCK_SETTLE);
            let mut get = uac::uac1_get_rate(endpoint);
            // Many UAC1 devices cannot report the rate: a refused read-back is taken as the rate set.
            match self.device.control(&mut get) {
                Ok(count) => uac::parse_rate(&get.data[..count.min(3)]),
                Err(_) => Some(rate),
            }
        };
        self.rate_read_back = read_back;
        if read_back != Some(rate) {
            return Err(format!(
                "時脈讀回 {} Hz，不是 {rate} Hz",
                read_back.map_or_else(|| "?".to_owned(), |r| r.to_string())
            ));
        }
        Ok(())
    }

    fn start_streaming(&mut self, plan: &StreamPlan) -> Result<(), String> {
        self.stop_streaming();
        let device_frame_bytes = plan.bytes_per_frame.max(1);
        let max_packet = plan.data_endpoint.max_bytes_per_interval();
        let max_frames = max_packet / device_frame_bytes;
        if max_frames == 0 {
            return Err("端點封包太小".to_owned());
        }
        let format = engine::format_of(self.state).ok_or("沒有串流格式")?;
        let bus_rate = if self.high_speed { 8000 } else { 1000 };
        let container = Container {
            bytes: format.subslot_bytes,
            bits: format.bits,
        };
        let mut stream = Stream {
            data: (0..DATA_URBS)
                .map(|index| Slot::new(index, plan.data_endpoint.address, max_packet))
                .collect(),
            feedback: Vec::new(),
            sizer: PacketSizer::new(
                format.rate,
                bus_rate,
                plan.data_endpoint.interval,
                max_frames,
            ),
            parser: Feedback::new(format.rate, bus_rate),
            device_frame_bytes,
            container,
            scratch: vec![0; max_frames * 4 * usize::from(format.channels.max(1))],
            feedback_urbs: FeedbackUrbStats::default(),
            data_urbs: DataUrbStats::default(),
        };
        if let Some(endpoint) = plan.feedback_endpoint {
            let bytes = endpoint.max_bytes_per_interval().max(4);
            stream.feedback = (0..FEEDBACK_URBS)
                .map(|index| {
                    let mut slot = Slot::<1>::new(FEEDBACK_TAG + index, endpoint.address, bytes);
                    slot.urb.packets[0].length = bytes as c_uint;
                    slot
                })
                .collect();
        }
        let fd = self.device.fd;
        for slot in &mut stream.feedback {
            if let Err(error) = submit(fd, slot) {
                self.urb_errors += 1;
                self.device.step(format!("submit feedback: {error}"));
            }
        }
        let gain = self.shared.gain();
        for index in 0..DATA_URBS {
            fill(
                &mut stream,
                index,
                self.attached.as_mut(),
                gain,
                &mut self.underruns,
            );
            if let Err(error) = submit(fd, &mut stream.data[index]) {
                self.urb_errors += 1;
                stream.data[index].credits = [0; PACKETS_PER_URB];
                stream.data[index].ring = None;
                self.stream = Some(stream);
                return Err(format!("無法送出等時資料：{error}"));
            }
        }
        self.device.step(format!(
            "streaming: {DATA_URBS} data URBs, {} kHz",
            format.rate as f64 / 1000.0
        ));
        self.stream = Some(stream);
        Ok(())
    }

    /// Discard and reap every URB; what was packed but not played goes out again on the next start.
    fn stop_streaming(&mut self) {
        let Some(mut stream) = self.stream.take() else {
            return;
        };
        let fd = self.device.fd;
        for slot in stream.data.iter_mut().filter(|slot| slot.in_flight) {
            let _ = ioctl_raw(fd, ioctl::DISCARD_URB, slot.pointer());
        }
        for slot in stream.feedback.iter_mut().filter(|slot| slot.in_flight) {
            let _ = ioctl_raw(fd, ioctl::DISCARD_URB, slot.pointer());
        }
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        let in_flight = |stream: &Stream| {
            stream.data.iter().any(|slot| slot.in_flight)
                || stream.feedback.iter().any(|slot| slot.in_flight)
        };
        while in_flight(&stream) && Instant::now() < deadline {
            match self.device.reap() {
                Ok(Some(tag)) if tag >= FEEDBACK_TAG => {
                    if let Some(slot) = stream.feedback.get_mut(tag - FEEDBACK_TAG) {
                        slot.in_flight = false;
                    }
                }
                Ok(Some(tag)) => {
                    if let Some(slot) = stream.data.get_mut(tag) {
                        slot.in_flight = false;
                        slot.credit(true);
                    }
                }
                Ok(None) => {
                    if self.device.wait(20).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if in_flight(&stream) {
            // The kernel may still hold these: their memory is never freed (the process keeps it).
            for slot in stream.data.drain(..).filter(|slot| slot.in_flight) {
                std::mem::forget(slot);
            }
            for slot in stream.feedback.drain(..).filter(|slot| slot.in_flight) {
                std::mem::forget(slot);
            }
            self.device.step("stop: URBs not all reaped".to_owned());
        }
        if let Some(attached) = self.attached.as_mut() {
            attached.cursor.rewind(&attached.ring);
            attached.starved = false;
        }
        let (feedback, data) = (&stream.feedback_urbs, &stream.data_urbs);
        let stats = stream.parser.stats();
        self.device.step(format!(
            "stop: feedback urbs {} (with data {}), accepted {}, avg {:?}; data urbs {}, packets {}, length mismatch {}",
            feedback.reaped,
            feedback.with_data,
            stats.accepted,
            stats.avg,
            data.reaped,
            data.packets,
            data.length_mismatch
        ));
    }

    /// Reap every completed URB and resubmit it; an error when the device is gone.
    fn service(&mut self) -> Result<(), String> {
        let fd = self.device.fd;
        self.device
            .wait(STREAM_POLL_MS)
            .map_err(|error| format!("USB 裝置中斷：{error}"))?;
        let gain = self.shared.gain();
        loop {
            let Some(tag) = self
                .device
                .reap()
                .map_err(|error| format!("USB 裝置中斷：{error}"))?
            else {
                return Ok(());
            };
            let Some(stream) = self.stream.as_mut() else {
                continue;
            };
            if tag >= FEEDBACK_TAG {
                let Some(slot) = stream.feedback.get_mut(tag - FEEDBACK_TAG) else {
                    continue;
                };
                slot.in_flight = false;
                let packet = slot.urb.packets[0];
                let urb_status = slot.urb.urb.status;
                if urb_status != 0 {
                    self.urb_errors += 1;
                }
                let length = (packet.actual_length as usize).min(slot.buffer.len());
                if let Some(bytes) = stream.feedback_urbs.record(
                    urb_status,
                    packet.status as i32,
                    &slot.buffer[..length],
                ) {
                    stream.parser.push(bytes);
                }
                if submit(fd, slot).is_err() {
                    self.urb_errors += 1;
                }
                continue;
            }
            let Some(slot) = stream.data.get_mut(tag) else {
                continue;
            };
            slot.in_flight = false;
            let status = slot.urb.urb.status;
            if status != 0 {
                self.urb_errors += 1;
            }
            stream.data_urbs.record_urb(status);
            for packet in slot.urb.packets {
                stream.data_urbs.record_packet(
                    packet.status as i32,
                    packet.length,
                    packet.actual_length,
                );
                if packet.status != 0 {
                    self.packet_errors += 1;
                }
            }
            slot.credit(false);
            fill(
                stream,
                tag,
                self.attached.as_mut(),
                gain,
                &mut self.underruns,
            );
            if let Err(error) = submit(fd, &mut stream.data[tag]) {
                self.urb_errors += 1;
                stream.data[tag].credits = [0; PACKETS_PER_URB];
                stream.data[tag].ring = None;
                if !stream.data.iter().any(|slot| slot.in_flight) {
                    return Err(format!("無法送出等時資料：{error}"));
                }
            }
        }
    }

    fn reply_attach(
        &mut self,
        ring: Arc<Ring>,
        rate: u32,
        source: Source,
    ) -> Result<AttachReply, String> {
        let channels = ring.channels();
        let ranges = format::known_ranges(self.config.vendor_id, self.config.product_id);
        let chosen = format::choose(&self.config, rate, channels, source, ranges.as_deref());
        let (plan, container) = match chosen {
            Ok(chosen) => chosen,
            Err(reason) => {
                // The track plays through Android: the device must be Android's again for that.
                let reason = reason.to_string();
                if self.state != State::Unclaimed {
                    self.fail(reason.clone());
                } else {
                    self.error = Some(reason.clone());
                    self.state = State::HandedBack;
                }
                ring.fail();
                return Err(reason);
            }
        };
        let format = StreamFormat {
            rate,
            channels,
            alt: plan.alt,
            subslot_bytes: container.bytes,
            bits: container.bits,
        };
        let previous = engine::format_of(self.state);
        let streaming_same =
            matches!(self.state, State::Streaming { .. }) && previous == Some(format);
        if !streaming_same {
            // The old stream (if any) stops with the old plan before the new one is configured.
            if matches!(self.state, State::Streaming { .. }) {
                self.stop_streaming();
            }
        }
        self.plan = Some(plan);
        self.attached = Some(Attached {
            cursor: Cursor::at(&ring),
            ring: Arc::clone(&ring),
            source,
            started: false,
            starved: false,
        });
        self.error = None;
        self.handle(Event::Attach(format))?;
        Ok(AttachReply {
            rate,
            channels,
            alt: plan_alt(self.plan.as_ref()),
            container,
        })
    }

    fn matches(&self, ring: usize) -> bool {
        self.attached
            .as_ref()
            .is_some_and(|attached| ring_id(&attached.ring) == ring)
    }

    /// Carry out one request; false when the claim ended.
    fn request(&mut self, request: Request) -> bool {
        match request {
            Request::Attach {
                ring,
                rate,
                source,
                reply,
            } => {
                let answer = self.reply_attach(ring, rate, source);
                let _ = reply.send(answer);
            }
            Request::Play { ring, reply } => {
                let answer = if self.matches(ring) {
                    self.handle(Event::Play)
                } else {
                    Err("這個輸出已不是目前的輸出".to_owned())
                };
                let _ = reply.send(answer);
            }
            Request::Pause { ring, reply } => {
                let answer = if self.matches(ring) {
                    self.handle(Event::Pause)
                } else {
                    Ok(())
                };
                let _ = reply.send(answer);
            }
            Request::Detach { ring, reply } => {
                let answer = if self.matches(ring) {
                    self.attached = None;
                    self.handle(Event::Detach)
                } else {
                    Ok(())
                };
                let _ = reply.send(answer);
            }
            Request::Release { reason, reply } => {
                self.device.step(format!("release: {reason}"));
                let answer = self.handle(Event::Release);
                // The output that played stalls: Media3 replaces it with one through Android.
                if let Some(attached) = &self.attached {
                    attached.ring.fail();
                }
                let _ = reply.send(answer);
            }
        }
        self.publish();
        self.state != State::HandedBack
    }
}

fn plan_alt(plan: Option<&StreamPlan>) -> u8 {
    plan.map_or(0, |plan| plan.alt)
}

/// Fill data URB `index` for the next packets: ring bytes packed with `gain`, silence for what the ring does not have.
fn fill(
    stream: &mut Stream,
    index: usize,
    mut attached: Option<&mut Attached>,
    gain: f32,
    underruns: &mut u64,
) {
    let frame = stream.device_frame_bytes;
    let container = stream.container;
    let feedback = stream.parser.current_q16();
    let slot = &mut stream.data[index];
    slot.ring = attached.as_ref().map(|attached| Arc::clone(&attached.ring));
    let mut at = 0;
    let mut short = false;
    for packet in 0..PACKETS_PER_URB {
        let frames = stream.sizer.next_frames(feedback);
        let bytes = frames * frame;
        let mut got_frames = 0;
        if let Some(attached) = attached.as_deref_mut() {
            let in_frame = attached.ring.frame_bytes();
            let wanted = (frames * in_frame).min(stream.scratch.len());
            let got = attached
                .cursor
                .take(&attached.ring, wanted, &mut stream.scratch[..wanted]);
            got_frames = got / in_frame;
            pcm::pack(
                attached.ring.encoding(),
                &stream.scratch[..got],
                container,
                gain,
                &mut slot.buffer[at..at + got_frames * frame],
            );
            slot.credits[packet] = got;
            if got > 0 {
                attached.started = true;
            }
            short |= got < wanted;
        } else {
            slot.credits[packet] = 0;
        }
        slot.buffer[at + got_frames * frame..at + bytes].fill(0);
        slot.urb.packets[packet].length = bytes as c_uint;
        at += bytes;
    }
    if let Some(attached) = attached {
        // An empty ring before any byte went out is the start, and after `stop()` it is the end.
        if short && attached.started && !attached.ring.is_ended() {
            if !attached.starved {
                attached.starved = true;
                attached.ring.count_underrun();
                *underruns += 1;
            }
        } else if !short {
            attached.starved = false;
        }
    }
}

/// Own the device behind `fd` for one claim: carry out `requests` until the device is handed back (idle, a release,
/// an error, or every sender gone). Never panics across the caller (the hand-back also runs from the device's `Drop`).
pub fn run(fd: c_int, raw_descriptors: &[u8], requests: &Receiver<Request>, shared: &Shared) {
    let config = parse_configuration(raw_descriptors);
    let high = high_speed(fd, config.bcd_usb);
    let mut runtime = Runtime {
        device: Device {
            fd,
            detached: Vec::new(),
            claimed: Vec::new(),
            streaming: None,
            handed_back: false,
            steps: Vec::new(),
        },
        config,
        shared,
        high_speed: high,
        started_at: Instant::now(),
        state: State::Unclaimed,
        plan: None,
        attached: None,
        stream: None,
        rate_read_back: None,
        underruns: 0,
        urb_errors: 0,
        packet_errors: 0,
        error: None,
        hand_back_errors: Vec::new(),
    };
    runtime.publish();
    let mut last_publish = Instant::now();
    loop {
        let next = if runtime.stream.is_some() {
            match requests.try_recv() {
                Ok(request) => Some(request),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => break,
            }
        } else {
            match requests.recv_timeout(IDLE_WAIT) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        };
        if let Some(request) = next {
            if !runtime.request(request) {
                break;
            }
            continue;
        }
        let _ = runtime.handle(Event::Tick);
        if runtime.state == State::HandedBack {
            break;
        }
        if runtime.stream.is_some() {
            if let Err(error) = runtime.service() {
                runtime.fail(error);
                break;
            }
        }
        if last_publish.elapsed() >= Duration::from_millis(250) {
            runtime.publish();
            last_publish = Instant::now();
        }
    }
    if runtime.state != State::HandedBack {
        let _ = runtime.handle(Event::Release);
    }
    if !runtime.device.handed_back {
        runtime.stop_streaming();
        runtime.hand_back_errors = runtime.device.hand_back();
    }
    runtime.state = State::HandedBack;
    runtime.publish();
    // Requests that arrive now find the channel closed: the playback side starts a new claim.
    while let Ok(request) = requests.try_recv() {
        reject(request);
    }
}

/// Answer a request the ended claim cannot carry out.
fn reject(request: Request) {
    fn gone<T>() -> Result<T, String> {
        Err("USB 獨佔輸出已交還".to_owned())
    }
    match request {
        Request::Attach { reply, ring, .. } => {
            ring.fail();
            let _ = reply.send(gone());
        }
        Request::Play { reply, .. }
        | Request::Pause { reply, .. }
        | Request::Detach { reply, .. }
        | Request::Release { reply, .. } => {
            let _ = reply.send(gone());
        }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = self.hand_back();
    }
}
