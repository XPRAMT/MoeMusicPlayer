# USB exclusive output test (P0, development only)

Status: Accepted (built and unit-tested; not yet run on a device)

Decision date: 2026-10-28, Asia/Taipei (UTC+08:00)

## Context

ADR 0044 sends every Android output through AudioFlinger, which may resample and mix, and ADR 0045 reads what a USB DAC really offers. The next question is whether the app can drive a USB Audio Class DAC itself: take the device away from the Android audio system (audio policy, AudioFlinger, the USB audio HAL and the kernel's `snd-usb-audio`), set its clock, stream PCM over isochronous transfers paced by the DAC's own feedback, and give the device back afterwards so Android can use it again. Before any product feature is designed, P0 proves the path once: a 44.1 kHz / 16-bit stereo FLAC played bit-perfectly for 30 s, triggered by a development-only button.

Target device (verified on Windows): Sony "Sony Audio" VID 054C PID 0B8C, USB 2.0 high speed, UAC2. Interface 0 is Audio Control without endpoints: clock source 0x29 (internal programmable, frequency read/write), clock selector 0x28 (one input, 0x29), USB streaming input terminal 0x01 (2 channels, clock 0x28), feature unit 0x08 without volume or mute, output terminal 0x04. Interface 1 is Audio Streaming: alt 0 zero bandwidth; alts 1–6 PCM (4/32, 4/24, 4/16, 3/24, 3/16, 2/16 subslot bytes/bits); alt 7 raw data (DSD) 4/32. Each alt has an isochronous asynchronous OUT 0x01 (wMaxPacketSize 1024, bInterval 1: every microframe) and an explicit feedback IN 0x81 (4 bytes, bInterval 4: every 8 microframes). The kernel lists 32–384 kHz. The raw descriptors are the test fixture `crates/player-usb-audio/tests/fixtures/sony-054c-0b8c.txt`.

## Decision

### Where the code lives

- `crates/player-usb-audio` (pure Rust, no libusb). Pure, tested on every host: `descriptors` (the UAC1/UAC2 descriptors needed to plan a stream), `uac` (sampling-frequency control requests), `feedback`, `packets` (packet sizer), `pcm` (16-bit decode and packing over `player-flac`), `ioctl` (usbfs structs and ioctl numbers, checked against the 64-bit kernel values). System calls only in `usbfs` (Android and Linux), through `libc`.
- `src-tauri/src/android_usb_exclusive.rs`: guards, the shared status, the JNI export `UsbExclusiveBridge.nativeRun(fd, rawDescriptors, path)`.
- Kotlin `UsbExclusiveTest.kt`: USB permission (the dialog flow of ADR 0045, `UsbAudioReader.askInTurn`), `openDevice`, `getRawDescriptors`, the run on its own thread (`moe-usb-exclusive`, `THREAD_PRIORITY_URGENT_AUDIO`), and `close()` only after Rust returned. Plugin command `usbExclusiveStart { path }`.
- Tauri commands `android_usb_exclusive_start` / `_stop` / `_status` (null on other platforms); page `AndroidOutputDevicesSettings.svelte`, summary text `usb-exclusive-test.js`.

### Plan from the descriptors

The first configuration of `getRawDescriptors()` is walked by bLength (a truncated or unknown descriptor is skipped). UAC version: interface protocol 0x20 → UAC2, else the AC header's bcdADC. For the wanted format (subslot bytes, bits, channels) the first Audio Streaming alt with an isochronous OUT data endpoint and a matching Type I PCM format is chosen (UAC2: bFormatType 1, bmFormats bit 0 set and bit 31 clear, bSubslotSize, bBitResolution, AS general bNrChannels; UAC1: wFormatTag PCM, bSubframeSize, and the alt must list the rate). The clock (UAC2) is found from the alt's terminal link → the input terminal's bCSourceID → a clock selector is followed to its first input, a multiplier to its source. An asynchronous data endpoint needs an explicit feedback endpoint (an isochronous IN with usage "feedback", or UAC1's bSynchAddress); implicit feedback is refused. The kernel driver is detached from, and later reattached to, every audio interface of the configuration. On the Sony DAC: AC 0, streaming 1, alt 6, clock 0x29, data 0x01, feedback 0x81.

### usbfs flow

1. Decode first (whole, at most 30 s, before the device is touched: at 44.1 kHz that is 5.3 MB), so decoding can never starve the bus.
2. `USBDEVFS_GET_SPEED` (high speed: 8000 bus intervals per second; full speed: 1000; if refused, bcdUSB ≥ 2.0 counts as high speed).
3. For each audio interface: `USBDEVFS_IOCTL(USBDEVFS_DISCONNECT)` (ENODATA = no driver bound, fine), then `USBDEVFS_CLAIMINTERFACE`. `DISCONNECT_CLAIM` is not used: the two-step form works on every kernel Android ships.
4. `USBDEVFS_SETINTERFACE` streaming interface alt 0.
5. UAC2: GET RANGE of the clock (2 bytes for wNumSubRanges, then 2 + 12 × n); when it answers and the rate is on none of the sub-ranges (min ≤ rate ≤ max on the resolution grid) the test stops with 「DAC 不支援 … Hz」; a refused RANGE request is not fatal. SET CUR (bmRequestType 0x21, bRequest CUR 0x01, wValue 0x0100 = CS_SAM_FREQ_CONTROL << 8 with channel 0, wIndex = clock id << 8 | AC interface, 4 bytes LE), wait 50 ms, GET CUR (0xA1): a read-back that is not the rate stops the test. UAC1: SET_CUR / GET_CUR to the data endpoint (0x22 / 0xA2, 3 bytes); a refused read-back is taken as the rate set.
6. `USBDEVFS_SETINTERFACE` to the chosen alt.
7. Stream (below) until the PCM ends, 「停止」, the 30 s limit, or an error.
8. Hand back, always (below).

Every step and its result is kept in the report's `steps`, logged with the whole report as `USB exclusive: i/N: {json}` (tag `MoeMediaIndex`, in parts).

### Streaming, feedback, packet sizes

- **Thread model**: one thread does everything: it submits, polls the usbfs fd (`poll` POLLOUT, 100 ms) and reaps with `USBDEVFS_REAPURBNDELAY` until nothing is ready. No other thread touches the URBs, so their memory needs no locking. Stop is an atomic flag read on every loop.
- **URBs**: 16 data URBs of 8 isochronous packets (one packet per service interval; at high speed with bInterval 1 a URB is 1 ms, so 16 ms are queued), `USBDEVFS_URB_ISO_ASAP`, each with its own buffer of 8 × wMaxPacketSize; 2 feedback URBs of one packet each (the feedback endpoint's size). Each URB is a boxed `#[repr(C)]` header followed by its packet descriptors, as the kernel's flexible array expects. A reaped data URB is refilled and resubmitted at once; an underrun is counted when every data URB had completed before the next was queued.
- **Feedback**: high speed 4 bytes Q16.16 frames per microframe; full speed 3 bytes Q10.14 per frame (shifted to Q16.16). The first non-zero value fixes a power-of-two shift that brings it between 0.75 and 1.5 of nominal (devices that send another scaling, the idea the Linux driver uses); after that a value is accepted only within ±2 % of nominal (`FEEDBACK_TOLERANCE`: real clocks are within a few hundred ppm, a wrong scaling is a factor of 2), and 8 rejected values in a row work the shift out again. Until the first accepted value the nominal rate is used. No smoothing: the latest accepted value is used.
- **Packet sizer**: an exact integer accumulator in units of 1 / (packets per second × 65 536) frame. The nominal rate adds rate × 65 536 per packet and a Q16.16 feedback value adds value × bus intervals per packet × packets per second, both whole numbers, so nothing is lost to rounding: at the nominal rate the frames sent after any whole number of seconds are exactly rate × seconds (5 and 6 frames alternating at 44.1 kHz), and with feedback they are within one frame of the feedback rate × time. A packet never exceeds wMaxPacketSize; what did not fit is carried, at most one packet's worth.
- **PCM**: `player-flac` decodes to `f32` as Symphonia's left-aligned `i32` / 2^31; for 16-bit samples that is exact, and × 32 768 gives the original sample back. Every value is checked to be a whole 16-bit sample and packed little-endian interleaved: the bytes are the file's samples (the unit test compares their MD5 with the FLAC STREAMINFO signature). No volume, no resampling, no dither.

### Hand-back rules

At any end (finished, stopped, a failed step, a device error, a panic: the session's `Drop` runs it and `catch_unwind` keeps a panic from crossing JNI): `USBDEVFS_DISCARDURB` every URB in flight, reap them (up to 2 s; a URB the kernel did not return has its memory left to the process instead of freed), streaming interface to alt 0, `USBDEVFS_RELEASEINTERFACE` each claimed interface, then `USBDEVFS_IOCTL(USBDEVFS_CONNECT)` each detached interface in order, AC first (EBUSY = the driver already bound it, which `snd-usb-audio` does for the streaming interface when it probes the control one). `handedBack` is true when every release and reattach succeeded; the errors are listed. Kotlin closes the `UsbDeviceConnection` only after `nativeRun` returned.

### Guards (P0)

Refused with a reason: the player is playing or buffering (Media3 must not use the DAC); no current track; the current track has no local file; it is not FLAC, or not 16-bit stereo; a test already runs; no USB audio device (the first attached one is used); no USB permission; the device cannot be opened. The rate check is the DAC's own (GET RANGE, then the read-back). Pressing play during the test is not blocked: the DAC is not an Android output then, so Media3 plays elsewhere.

### Page and safety

Inside the USB card's 「讀取 USB 規格」 result, once a device was read: 「USB 獨佔試播 30 秒（開發用）」 and 「停止」. The start button opens a confirm panel first (the app's `update-dialog floating-surface` dialog): the output is bit-perfect, so with a DAC without hardware volume it is full level, and the DAC or headphone volume must be lowered first. While it runs the page reads the status every 500 ms; afterwards one line, e.g. 「時鐘 44100（讀回 44100）．alt 6．回授平均 5.5126．欠載 0．錯誤 0．已交還」, 「已停止．…」 or 「失敗：{reason}．…」.

### Licensing

Written from the USB 2.0 and USB Audio Class 1.0/2.0 specifications and the Linux usbfs header. No code was copied from other projects; decent-player's MIT `libs/decent-usb-audio-driver` was allowed as an ordering reference but not used, and its AGPL `driver/Felicity/`, libusbdac, Tryptify, EchoTide-Music and other GPL/AGPL code were not read (AGENTS.md: no GPL/AGPL). `md-5` (MIT/Apache) is a test-only dependency.

## Consequences

- P0 changes no playback path; the button is development only and not a product feature.
- Unit tests: `cargo test -p player-usb-audio` (Sony fixture plan incl. every PCM alt and alt 7 not PCM, truncation at every byte and random bytes, UAC1 adaptive with rates, async without feedback refused, high-bandwidth sizes; control request encoding, RANGE parsing and the grid check; feedback HS/FS/rescaled/rejected; packet sizer exact over ten minutes and every listed rate, feedback within one frame, packet cap; MD5 bit-exactness of the 16-bit path; ioctl numbers); src-tauri `android_usb_exclusive::tests` (guards, one test at a time); JS `tests/usb-exclusive-test.test.mjs`.
- Not verified, to watch in the device test: whether Android's SELinux policy lets an app issue `USBDEVFS_IOCTL` DISCONNECT/CONNECT and isochronous `SUBMITURB` on the fd from `UsbManager.openDevice`; that `snd-usb-audio` really lets go of both interfaces and probes again on CONNECT (the device should disappear from and come back to the output list, with a new `AudioDeviceInfo` id); what AudioFlinger and the playback service do while the DAC is gone; that the Sony DAC accepts SET CUR 44100 and reads it back; the feedback values it sends (format and scale) and that 16 × 1 ms URBs never underrun on the phone and the tablet; clicks at start and end; that the hand-back works after a stop in the middle and after unplugging during the test; that bit-perfect really reaches the DAC (needs a DAC that shows the incoming format or a digital loopback).
- Future work beyond P0: a real output path (other bit depths and rates, 24-bit packing, volume, gapless, seek, DSD/DoP over alt 7), choosing among several USB devices, and keeping Media3 away from the DAC while it is taken.
