//! The exclusive stream over Linux usbfs (ADR 0046). Android and Linux only.
//!
//! Order (Linux `snd-usb-audio`'s UAC2 order, [`CLOCK_ORDER`]): plan from the descriptors → detach the kernel driver
//! from every audio interface and claim them → streaming interface to alt 0 → (UAC2) GET RANGE → alt for the format →
//! SET CUR rate, settle, GET CUR read-back, abort on a mismatch → feedback URBs and data URBs in flight, reaped on this
//! one thread (poll + non-blocking reap) → at any end: discard and reap every URB, alt 0, release, let the kernel driver
//! bind again. The hand-back runs from `Drop`, so an early return or a panic gives the device back too.

use std::io;
use std::os::raw::{c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::descriptors::{parse_configuration, plan_stream, PcmFormat, StreamPlan};
use crate::diagnostics::{DataUrbStats, FeedbackUrbStats, StatusCounts};
use crate::feedback::Feedback;
use crate::ioctl::{self, CtrlTransfer, IoctlRequest, IsoPacketDesc, SetInterface, Urb};
use crate::packets::PacketSizer;
use crate::pcm::{self, Pcm16};
use crate::uac::{self, ControlRequest};
use crate::{ClockOrder, Outcome, Report};

/// Data URBs kept in flight and isochronous packets (microframes at high speed) per URB: 16 × 8 = 16 ms queued at
/// high speed, enough to ride out a scheduling hiccup of the thread.
pub const DATA_URBS: usize = 16;
pub const PACKETS_PER_URB: usize = 8;
const FEEDBACK_URBS: usize = 2;
const CONTROL_TIMEOUT_MS: u32 = 1000;
const POLL_MS: c_int = 100;
/// How long the hand-back waits for discarded URBs to come back before leaving their memory to the process.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
/// After the clock is set, before it is read back and streaming starts.
const CLOCK_SETTLE: Duration = Duration::from_millis(50);
/// The rate is set after the streaming alt is selected, as Linux does for UAC2; the other order only for an A/B test.
const CLOCK_ORDER: ClockOrder = ClockOrder::AltThenClock;

const FEEDBACK_TAG: usize = 0x1000;

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
        }
    }

    fn pointer(&mut self) -> *mut Urb {
        &mut self.urb.urb
    }
}

struct Session {
    fd: c_int,
    /// Audio interfaces the kernel driver was told to leave (each gets USBDEVFS_CONNECT back).
    detached: Vec<u32>,
    claimed: Vec<u32>,
    streaming: Option<u32>,
    data: Vec<Slot<PACKETS_PER_URB>>,
    feedback: Vec<Slot<1>>,
    handed_back: bool,
    steps: Vec<String>,
    hand_back_errors: Vec<String>,
    hand_back_ok: bool,
}

impl Session {
    fn step(&mut self, text: String) {
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

    /// Wait until a completed URB can be reaped (or `timeout_ms`); false when the device is gone.
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
    fn reap(&mut self) -> io::Result<Option<usize>> {
        let mut pointer: *mut Urb = std::ptr::null_mut();
        match ioctl_raw(self.fd, ioctl::REAP_URB_NDELAY, &mut pointer) {
            Ok(_) if !pointer.is_null() => {
                // SAFETY: the kernel hands back a pointer we submitted; the slot that owns it is still alive.
                let tag = unsafe { (*pointer).user_context } as usize;
                if let Some(slot) = self.slot_in_flight(tag) {
                    *slot = false;
                }
                Ok(Some(tag))
            }
            Ok(_) => Ok(None),
            Err(error) if errno(&error) == Some(libc::EAGAIN) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn slot_in_flight(&mut self, tag: usize) -> Option<&mut bool> {
        if tag >= FEEDBACK_TAG {
            self.feedback
                .get_mut(tag - FEEDBACK_TAG)
                .map(|slot| &mut slot.in_flight)
        } else {
            self.data.get_mut(tag).map(|slot| &mut slot.in_flight)
        }
    }

    fn data_in_flight(&self) -> usize {
        self.data.iter().filter(|slot| slot.in_flight).count()
    }

    fn any_in_flight(&self) -> bool {
        self.data.iter().any(|slot| slot.in_flight)
            || self.feedback.iter().any(|slot| slot.in_flight)
    }

    /// Give the DAC back: discard and reap every URB, alt 0, release, let the kernel driver bind again. Runs once.
    fn hand_back(&mut self) {
        if self.handed_back {
            return;
        }
        self.handed_back = true;
        let mut errors = Vec::new();
        let fd = self.fd;
        for slot in &mut self.data {
            if slot.in_flight {
                let _ = ioctl_raw(fd, ioctl::DISCARD_URB, slot.pointer());
            }
        }
        for slot in &mut self.feedback {
            if slot.in_flight {
                let _ = ioctl_raw(fd, ioctl::DISCARD_URB, slot.pointer());
            }
        }
        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while self.any_in_flight() && Instant::now() < deadline {
            match self.reap() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if self.wait(20).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if self.any_in_flight() {
            // The kernel may still hold these: their memory is never freed (the process keeps it).
            for slot in self.data.drain(..).filter(|slot| slot.in_flight) {
                std::mem::forget(slot);
            }
            for slot in self.feedback.drain(..).filter(|slot| slot.in_flight) {
                std::mem::forget(slot);
            }
            errors.push("URB 未全部收回".to_owned());
        }
        if let Some(interface) = self.streaming.take() {
            if let Err(error) = self.set_interface(interface, 0) {
                errors.push(format!("介面 {interface} 回到 alt 0 失敗：{error}"));
            }
        }
        let mut released_all = true;
        for interface in std::mem::take(&mut self.claimed) {
            let mut number: c_uint = interface;
            let result = ioctl_raw(fd, ioctl::RELEASE_INTERFACE, &mut number);
            self.step(format!(
                "release {interface}: {}",
                result
                    .as_ref()
                    .map_or_else(ToString::to_string, |_| "ok".to_owned())
            ));
            if let Err(error) = result {
                released_all = false;
                errors.push(format!("釋放介面 {interface} 失敗：{error}"));
            }
        }
        let mut reattached = true;
        for interface in std::mem::take(&mut self.detached) {
            let result = self.sub_ioctl(interface, ioctl::CONNECT);
            match result {
                Ok(_) => self.step(format!("reattach {interface}: ok")),
                // The driver bound it already (it claims every interface of the function from the control one).
                Err(error) if errno(&error) == Some(libc::EBUSY) => {
                    self.step(format!("reattach {interface}: already bound"))
                }
                Err(error) => {
                    self.step(format!("reattach {interface}: {error}"));
                    reattached = false;
                    errors.push(format!("介面 {interface} 交還系統驅動程式失敗：{error}"));
                }
            }
        }
        self.hand_back_ok = released_all && reattached;
        self.hand_back_errors = errors;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.hand_back();
    }
}

/// USB speed from the kernel, or a guess from bcdUSB when the ioctl is refused.
fn high_speed(fd: c_int, bcd_usb: u16) -> bool {
    match ioctl_raw::<c_void>(fd, ioctl::GET_SPEED, std::ptr::null_mut()) {
        Ok(speed) => speed >= ioctl::SPEED_HIGH,
        Err(_) => bcd_usb >= 0x0200,
    }
}

/// UAC2: which rates the clock takes. A refused RANGE request is not fatal (the read-back still guards).
fn check_rate_range(session: &mut Session, plan: &StreamPlan, rate: u32) -> Result<(), String> {
    let Some(clock) = plan.clock_id else {
        return Ok(());
    };
    let mut head = uac::uac2_get_rate_range(clock, plan.ac_interface, 2);
    if session.control(&mut head).is_ok() {
        if let Some(length) = uac::uac2_range_length(&head.data) {
            let mut full = uac::uac2_get_rate_range(clock, plan.ac_interface, length);
            if let Ok(count) = session.control(&mut full) {
                let ranges = uac::parse_uac2_rate_ranges(&full.data[..count.min(full.data.len())]);
                if !ranges.is_empty() && !uac::rate_in_ranges(&ranges, rate) {
                    return Err(format!("USB 音訊裝置不支援 {rate} Hz"));
                }
            }
        }
    }
    Ok(())
}

/// Set the clock, let it settle and read it back; a read-back that is not the rate is an error.
fn set_clock(
    session: &mut Session,
    plan: &StreamPlan,
    rate: u32,
    report: &mut Report,
) -> Result<(), String> {
    if let Some(clock) = plan.clock_id {
        let mut set = uac::uac2_set_rate(clock, plan.ac_interface, rate);
        session
            .control(&mut set)
            .map_err(|error| format!("設定時脈 {rate} Hz 失敗：{error}"))?;
        report.rate_set = Some(rate);
        std::thread::sleep(CLOCK_SETTLE);
        let mut get = uac::uac2_get_rate(clock, plan.ac_interface);
        let count = session
            .control(&mut get)
            .map_err(|error| format!("讀回時脈失敗：{error}"))?;
        report.rate_read_back = uac::parse_rate(&get.data[..count.min(4)]);
    } else {
        let endpoint = plan.data_endpoint.address;
        let mut set = uac::uac1_set_rate(endpoint, rate);
        session
            .control(&mut set)
            .map_err(|error| format!("設定取樣率 {rate} Hz 失敗：{error}"))?;
        report.rate_set = Some(rate);
        std::thread::sleep(CLOCK_SETTLE);
        let mut get = uac::uac1_get_rate(endpoint);
        // Many UAC1 devices cannot report the rate: a refused read-back is taken as the rate set.
        report.rate_read_back = match session.control(&mut get) {
            Ok(count) => uac::parse_rate(&get.data[..count.min(3)]),
            Err(_) => Some(rate),
        };
    }
    if report.rate_read_back != Some(rate) {
        return Err(format!(
            "時脈讀回 {} Hz，不是 {rate} Hz",
            report
                .rate_read_back
                .map_or_else(|| "?".to_owned(), |r| r.to_string())
        ));
    }
    Ok(())
}

/// Play `pcm` exclusively on the USB audio device behind `fd` until it ends, `stop` is set or `limit` passes, then
/// give the device back to the kernel driver. Never panics across the caller (the session's `Drop` hands back).
pub fn run_exclusive(
    fd: c_int,
    raw_descriptors: &[u8],
    pcm: &Pcm16,
    stop: &AtomicBool,
    limit: Duration,
) -> Report {
    let mut report = Report {
        file_rate: Some(pcm.rate),
        frames_total: pcm.frames(),
        data_urbs: DATA_URBS,
        packets_per_urb: PACKETS_PER_URB,
        ..Report::default()
    };
    let config = parse_configuration(raw_descriptors);
    let format = PcmFormat {
        subslot_bytes: 2,
        bits: 16,
        channels: pcm.channels as u8,
        rate: pcm.rate,
    };
    let plan = match plan_stream(&config, format) {
        Ok(plan) => plan,
        Err(error) => {
            report.error = Some(error.to_string());
            return report;
        }
    };
    report.interface = Some(plan.streaming_interface);
    report.alt = Some(plan.alt);
    let high = high_speed(fd, config.bcd_usb);
    report.high_speed = Some(high);
    report.clock_order = CLOCK_ORDER;
    let levels = pcm::levels(pcm);
    let pcm_step = format!(
        "pcm: peak first {} s {} ({}), peak {} ({}), first samples {:?}",
        levels.first_seconds,
        levels.peak_first,
        dbfs_text(levels.peak_first_dbfs),
        levels.peak,
        dbfs_text(levels.peak_dbfs),
        levels.first_samples
    );
    report.pcm = Some(levels);

    let mut session = Session {
        fd,
        detached: Vec::new(),
        claimed: Vec::new(),
        streaming: None,
        data: Vec::new(),
        feedback: Vec::new(),
        handed_back: false,
        steps: Vec::new(),
        hand_back_errors: Vec::new(),
        hand_back_ok: false,
    };
    session.step(pcm_step);
    let result = stream(&mut session, &plan, high, pcm, stop, limit, &mut report);
    session.hand_back();
    report.handed_back = Some(session.hand_back_ok);
    report.hand_back_errors = std::mem::take(&mut session.hand_back_errors);
    report.steps = std::mem::take(&mut session.steps);
    match result {
        Ok(outcome) => report.outcome = outcome,
        Err(error) => {
            report.outcome = Outcome::Failed;
            report.error = Some(error);
        }
    }
    report
}

/// `["0×120", "-18 EXDEV×3"]`, for the log.
fn status_text(counts: &StatusCounts) -> Vec<String> {
    counts
        .0
        .iter()
        .map(|entry| match entry.name {
            Some(name) => format!("{} {name}×{}", entry.status, entry.count),
            None => format!("{}×{}", entry.status, entry.count),
        })
        .collect()
}

fn dbfs_text(dbfs: Option<f64>) -> String {
    dbfs.map_or_else(|| "silent".to_owned(), |dbfs| format!("{dbfs:.1} dBFS"))
}

/// Cuts the PCM into the packets of data URBs.
struct Feeder<'a> {
    pcm: &'a [u8],
    queued: usize,
    bytes_per_frame: usize,
    sizer: PacketSizer,
}

impl Feeder<'_> {
    fn more(&self) -> bool {
        self.queued < self.pcm.len()
    }

    /// Fill a data URB; false when nothing is left to send.
    fn fill(&mut self, slot: &mut Slot<PACKETS_PER_URB>, feedback: Option<u32>) -> bool {
        if !self.more() {
            return false;
        }
        let mut at = 0;
        for packet in 0..PACKETS_PER_URB {
            let frames = self.sizer.next_frames(feedback);
            let bytes = (frames * self.bytes_per_frame).min(self.pcm.len() - self.queued);
            slot.buffer[at..at + bytes]
                .copy_from_slice(&self.pcm[self.queued..self.queued + bytes]);
            slot.urb.packets[packet].length = bytes as c_uint;
            self.queued += bytes;
            at += bytes;
        }
        true
    }
}

fn stream(
    session: &mut Session,
    plan: &StreamPlan,
    high: bool,
    pcm: &Pcm16,
    stop: &AtomicBool,
    limit: Duration,
    report: &mut Report,
) -> Result<Outcome, String> {
    session.take_over(&plan.audio_interfaces)?;
    let streaming = u32::from(plan.streaming_interface);
    session
        .set_interface(streaming, 0)
        .map_err(|error| format!("介面 {streaming} 無法回到 alt 0：{error}"))?;
    check_rate_range(session, plan, pcm.rate)?;
    // From here the streaming interface may be on another alt: the hand-back puts it to alt 0.
    session.streaming = Some(streaming);
    let alt = plan.alt;
    let set_alt = |session: &mut Session| {
        session
            .set_interface(streaming, u32::from(alt))
            .map_err(|error| format!("無法切到 alt {alt}：{error}"))
    };
    match CLOCK_ORDER {
        ClockOrder::AltThenClock => {
            set_alt(session)?;
            set_clock(session, plan, pcm.rate, report)?;
        }
        ClockOrder::ClockThenAlt => {
            set_clock(session, plan, pcm.rate, report)?;
            set_alt(session)?;
        }
    }

    let bytes_per_frame = plan.bytes_per_frame.max(1);
    let max_packet = plan.data_endpoint.max_bytes_per_interval();
    let max_frames = max_packet / bytes_per_frame;
    if max_frames == 0 {
        return Err("端點封包太小".to_owned());
    }
    let bus_rate = if high { 8000 } else { 1000 };
    let mut feeder = Feeder {
        pcm: &pcm.bytes,
        queued: 0,
        bytes_per_frame,
        sizer: PacketSizer::new(pcm.rate, bus_rate, plan.data_endpoint.interval, max_frames),
    };
    let mut feedback = Feedback::new(pcm.rate, bus_rate);
    let mut feedback_urbs = FeedbackUrbStats::default();
    let mut data_packets = DataUrbStats::default();
    let data_endpoint = plan.data_endpoint.address;
    session.data = (0..DATA_URBS)
        .map(|index| Slot::new(index, data_endpoint, max_packet))
        .collect();
    if let Some(endpoint) = plan.feedback_endpoint {
        let bytes = endpoint.max_bytes_per_interval().max(4);
        session.feedback = (0..FEEDBACK_URBS)
            .map(|index| {
                let mut slot = Slot::<1>::new(FEEDBACK_TAG + index, endpoint.address, bytes);
                slot.urb.packets[0].length = bytes as c_uint;
                slot
            })
            .collect();
    }

    let fd = session.fd;
    for index in 0..session.feedback.len() {
        if let Err(error) = Session::submit(fd, &mut session.feedback[index]) {
            report.urb_errors += 1;
            session.step(format!("submit feedback: {error}"));
        }
    }
    let started = Instant::now();
    let result = 'run: {
        for index in 0..DATA_URBS {
            if !feeder.fill(&mut session.data[index], feedback.current_q16()) {
                break;
            }
            if let Err(error) = Session::submit(fd, &mut session.data[index]) {
                report.urb_errors += 1;
                break 'run Err(format!("無法送出等時資料：{error}"));
            }
        }
        session.step(format!(
            "streaming: {} data URBs queued",
            session.data_in_flight()
        ));
        loop {
            if stop.load(Ordering::Relaxed) {
                break 'run Ok(Outcome::Stopped);
            }
            if started.elapsed() >= limit || (session.data_in_flight() == 0 && !feeder.more()) {
                break 'run Ok(Outcome::Finished);
            }
            if let Err(error) = session.wait(POLL_MS) {
                break 'run Err(format!("USB 裝置中斷：{error}"));
            }
            loop {
                let tag = match session.reap() {
                    Ok(Some(tag)) => tag,
                    Ok(None) => break,
                    Err(error) => break 'run Err(format!("USB 裝置中斷：{error}")),
                };
                if tag >= FEEDBACK_TAG {
                    let Some(slot) = session.feedback.get_mut(tag - FEEDBACK_TAG) else {
                        continue;
                    };
                    let packet = slot.urb.packets[0];
                    let urb_status = slot.urb.urb.status;
                    if urb_status != 0 {
                        report.urb_errors += 1;
                    }
                    let length = (packet.actual_length as usize).min(slot.buffer.len());
                    if let Some(bytes) = feedback_urbs.record(
                        urb_status,
                        packet.status as i32,
                        &slot.buffer[..length],
                    ) {
                        feedback.push(bytes);
                    }
                    if !stop.load(Ordering::Relaxed) && Session::submit(fd, slot).is_err() {
                        report.urb_errors += 1;
                    }
                    continue;
                }
                let Some(slot) = session.data.get(tag) else {
                    continue;
                };
                let (status, packets) = (slot.urb.urb.status, slot.urb.packets);
                if status != 0 {
                    report.urb_errors += 1;
                }
                data_packets.record_urb(status);
                for packet in packets {
                    data_packets.record_packet(
                        packet.status as i32,
                        packet.length,
                        packet.actual_length,
                    );
                    if packet.status != 0 {
                        report.packet_errors += 1;
                        continue;
                    }
                    let sent = if packet.actual_length > 0 {
                        packet.actual_length
                    } else {
                        packet.length
                    };
                    report.frames_sent += u64::from(sent) / bytes_per_frame as u64;
                }
                if !feeder.more() || stop.load(Ordering::Relaxed) {
                    continue;
                }
                if session.data_in_flight() == 0 {
                    report.underruns += 1;
                }
                if feeder.fill(&mut session.data[tag], feedback.current_q16()) {
                    if let Err(error) = Session::submit(fd, &mut session.data[tag]) {
                        report.urb_errors += 1;
                        session.step(format!("submit data: {error}"));
                        if session.data_in_flight() == 0 {
                            break 'run Err(format!("無法送出等時資料：{error}"));
                        }
                    }
                }
            }
        }
    };
    report.elapsed_ms = started.elapsed().as_millis() as u64;
    report.feedback = feedback.stats();
    session.step(format!(
        "feedback urbs: reaped {}, with data {}, zero length {}, packet status {:?}, urb status {:?}, first {:?}",
        feedback_urbs.reaped,
        feedback_urbs.with_data,
        feedback_urbs.zero_length,
        status_text(&feedback_urbs.packet_status),
        status_text(&feedback_urbs.urb_status),
        feedback_urbs.first_packets
    ));
    session.step(format!(
        "feedback parser: accepted {}, rejected {}, shift {:?}, rescales {}, first raw {:?}, avg {:?}",
        report.feedback.accepted,
        report.feedback.rejected,
        report.feedback.shift,
        report.feedback.rescales,
        report.feedback.first_raw,
        report.feedback.avg
    ));
    session.step(format!(
        "data urbs: reaped {}, urb status {:?}, packets {}, packet status {:?}, length mismatch {}, bytes {}/{}",
        data_packets.reaped,
        status_text(&data_packets.urb_status),
        data_packets.packets,
        status_text(&data_packets.packet_status),
        data_packets.length_mismatch,
        data_packets.actual_bytes,
        data_packets.requested_bytes
    ));
    report.feedback_urbs = feedback_urbs;
    report.data_packets = data_packets;
    result
}
