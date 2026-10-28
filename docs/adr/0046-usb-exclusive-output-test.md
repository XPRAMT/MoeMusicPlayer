# USB exclusive output test (P0, development only)

Status: Accepted (P0 verified on the tablet with the Sony USB speaker: plays at 44.1 kHz with feedback after the order change; other devices not run)

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
5. UAC2: GET RANGE of the clock (2 bytes for wNumSubRanges, then 2 + 12 × n); when it answers and the rate is on none of the sub-ranges (min ≤ rate ≤ max on the resolution grid) the test stops with 「USB 音訊裝置不支援 … Hz」; a refused RANGE request is not fatal.
6. `USBDEVFS_SETINTERFACE` to the chosen alt.
7. Set the rate: UAC2 SET CUR (bmRequestType 0x21, bRequest CUR 0x01, wValue 0x0100 = CS_SAM_FREQ_CONTROL << 8 with channel 0, wIndex = clock id << 8 | AC interface, 4 bytes LE), wait 50 ms, GET CUR (0xA1): a read-back that is not the rate stops the test (and hands back). UAC1: SET_CUR / GET_CUR to the data endpoint (0x22 / 0xA2, 3 bytes); a refused read-back is taken as the rate set.
8. Stream (below) until the PCM ends, 「停止」, the 30 s limit, or an error.
9. Hand back, always (below).

Order of steps 6 and 7 (`ClockOrder`, report `clockOrder`): the alt is selected first and the rate set on the running alt, the order Linux `snd-usb-audio` uses for UAC2 (set the interface, then initialise the sample rate, then start the endpoints). The first device run used the other order (rate set and read back on alt 0, then the alt) and the device was silent; some devices reset or ignore a clock set while the streaming interface is at zero bandwidth. The old order stays as the internal constant `ClockOrder::ClockThenAlt` in `usbfs.rs` for an A/B run; nothing in the UI selects it.

Every step and its result is kept in the report's `steps` (with summary lines for the PCM, the feedback URBs, the feedback parser and the data URBs), logged with the whole report as `USB exclusive: i/N: {json}` (tag `MoeMediaIndex`, in parts).

### Streaming, feedback, packet sizes

- **Thread model**: one thread does everything: it submits, polls the usbfs fd (`poll` POLLOUT, 100 ms) and reaps with `USBDEVFS_REAPURBNDELAY` until nothing is ready. No other thread touches the URBs, so their memory needs no locking. Stop is an atomic flag read on every loop.
- **URBs**: 16 data URBs of 8 isochronous packets (one packet per service interval; at high speed with bInterval 1 a URB is 1 ms, so 16 ms are queued), `USBDEVFS_URB_ISO_ASAP`, each with its own buffer of 8 × wMaxPacketSize; 2 feedback URBs of one packet each (the feedback endpoint's size). Each URB is a boxed `#[repr(C)]` header followed by its packet descriptors, as the kernel's flexible array expects. A reaped data URB is refilled and resubmitted at once; an underrun is counted when every data URB had completed before the next was queued.
- **Feedback**: high speed 4 bytes Q16.16 frames per microframe; full speed 3 bytes Q10.14 per frame (shifted to Q16.16). The first non-zero value fixes a power-of-two shift that brings it between 0.75 and 1.5 of nominal (devices that send another scaling, the idea the Linux driver uses); after that a value is accepted only within ±2 % of nominal (`FEEDBACK_TOLERANCE`: real clocks are within a few hundred ppm, a wrong scaling is a factor of 2), and 8 rejected values in a row work the shift out again. Until the first accepted value the nominal rate is used. No smoothing: the latest accepted value is used.
- **Packet sizer**: an exact integer accumulator in units of 1 / (packets per second × 65 536) frame. The nominal rate adds rate × 65 536 per packet and a Q16.16 feedback value adds value × bus intervals per packet × packets per second, both whole numbers, so nothing is lost to rounding: at the nominal rate the frames sent after any whole number of seconds are exactly rate × seconds (5 and 6 frames alternating at 44.1 kHz), and with feedback they are within one frame of the feedback rate × time. A packet never exceeds wMaxPacketSize; what did not fit is carried, at most one packet's worth.
- **Diagnostics** (`diagnostics.rs`, pure; statuses are negative Linux errno, named from a fixed table): `feedbackUrbs` counts every reaped feedback URB in exactly one bucket: `withData` (URB and packet status 0, at least one byte: given to the parser), `zeroLength`, `packetStatus` and `urbStatus` (distinct non-zero values with counts and errno names), plus the first 8 packets with bytes as hex (`firstPackets`). The parser adds `shift` (the power-of-two shift chosen), `rescales` and `firstRaw` (the first packet read as Q16.16 before any shift) to `feedback`. `dataPackets` counts every data URB and packet status (0 included), packets whose `actual_length` differs from the `length` asked for (`lengthMismatch`), and requested / actual bytes. `pcm` gives the peak absolute sample over the first 3 s and over the whole buffer, raw and in dBFS (20 log10(peak / 32768), null for silence), and the first 16 interleaved samples, to rule out a silent buffer.
- **PCM**: `player-flac` decodes to `f32` as Symphonia's left-aligned `i32` / 2^31; for 16-bit samples that is exact, and × 32 768 gives the original sample back. Every value is checked to be a whole 16-bit sample and packed little-endian interleaved: the bytes are the file's samples (the unit test compares their MD5 with the FLAC STREAMINFO signature). No volume, no resampling, no dither.

### Hand-back rules

At any end (finished, stopped, a failed step, a device error, a panic: the session's `Drop` runs it and `catch_unwind` keeps a panic from crossing JNI): `USBDEVFS_DISCARDURB` every URB in flight, reap them (up to 2 s; a URB the kernel did not return has its memory left to the process instead of freed), streaming interface to alt 0, `USBDEVFS_RELEASEINTERFACE` each claimed interface, then `USBDEVFS_IOCTL(USBDEVFS_CONNECT)` each detached interface in order, AC first (EBUSY = the driver already bound it, which `snd-usb-audio` does for the streaming interface when it probes the control one). `handedBack` is true when every release and reattach succeeded; the errors are listed. Kotlin closes the `UsbDeviceConnection` only after `nativeRun` returned.

### Guards (P0)

Refused with a reason: the player is playing or buffering (Media3 must not use the DAC); no current track; the current track has no local file; it is not FLAC, or not 16-bit stereo; a test already runs; no USB audio device (the first attached one is used); no USB permission; the device cannot be opened. The rate check is the DAC's own (GET RANGE, then the read-back). Pressing play during the test is not blocked: the DAC is not an Android output then, so Media3 plays elsewhere.

### Page and safety

Inside the USB card's 「讀取 USB 規格」 result, once a device was read: 「USB 獨佔試播 30 秒（開發用）」 and 「停止」. The start button opens a confirm panel first (the app's `update-dialog floating-surface` dialog): the output is bit-perfect, so with a DAC without hardware volume it is full level, and the DAC or headphone volume must be lowered first. While it runs the page reads the status every 500 ms; afterwards one line, e.g. 「時鐘 44100（讀回 44100）．alt 6．回授 7480／7500．平均 5.5125．峰值 -3.2 dBFS．欠載 0．錯誤 0．已交還」 (feedback values accepted ／ feedback URBs reaped; the average only when one was accepted; 「PCM 全為靜音」 for a silent buffer), 「已停止．…」 or 「失敗：{reason}．…」. The panel names a USB audio device, not a DAC: the device may be a speaker. A remote current track is refused with 「這首是遠端曲目，試播只支援本機 FLAC」.

### Licensing

Written from the USB 2.0 and USB Audio Class 1.0/2.0 specifications and the Linux usbfs header. No code was copied from other projects; decent-player's MIT `libs/decent-usb-audio-driver` was allowed as an ordering reference but not used, and its AGPL `driver/Felicity/`, libusbdac, Tryptify, EchoTide-Music and other GPL/AGPL code were not read (AGENTS.md: no GPL/AGPL). `md-5` (MIT/Apache) is a test-only dependency.

## First device run

Tablet, the Sony device (a USB speaker), old order (clock before alt): outcome finished, rate set and read back 44100, alt 6, 1 322 955 of 1 323 000 frames sent in 30 000 ms, no underruns, no URB or packet errors, handed back (normal playback worked again afterwards). RANGE answered (2, then 110 bytes), SET CUR and GET CUR answered 4 bytes, 16 data URBs queued. But nothing was heard and the feedback parser saw no value at all (accepted 0, rejected 0): feedback URBs were submitted without error, yet none came back with status 0 and bytes, and such packets were not counted then. Hence the Linux order and the diagnostics above; the next run should tell whether feedback packets come back empty, with a status, or not at all, and whether the PCM buffer itself is silent.

## Second device run (Linux order)

Same tablet, device and 44.1 kHz / 16-bit FLAC, order alt then clock: the user heard the track play normally. Rate set and read back 44100, alt 6, 1 322 950 of 1 323 000 frames sent in 30 000 ms, no underruns, no URB or packet errors (30 000 data URBs and 240 000 packets all status 0, requested and actual bytes equal), handed back. Feedback came back: 29 976 feedback URBs reaped (about one per millisecond, matching bInterval 4 at high speed), 29 852 with data and 124 zero length, no error status; raw `0F 83 05 00` = Q16.16 5.5120 frames per microframe, shift 0, no rescale, all accepted, range 5.5119–5.5130, average 5.5123 (nominal 5.5125, so the device clock runs about 40 ppm slow and the sizer follows it). PCM peak −0.3 dBFS. Conclusion: this device only starts streaming (and sends feedback) when the sampling frequency is set after the streaming alt is selected; P0's goal (app-owned USB output at 44.1 kHz, bypassing the Android audio system, and handing the device back) is met on this tablet. Bit-perfect delivery itself is still unproven at the device end (no input-format display, no loopback).

## Consequences

- P0 changes no playback path; the button is development only and not a product feature.
- Unit tests: `cargo test -p player-usb-audio` (Sony fixture plan incl. every PCM alt and alt 7 not PCM, truncation at every byte and random bytes, UAC1 adaptive with rates, async without feedback refused, high-bandwidth sizes; control request encoding, RANGE parsing and the grid check; feedback HS/FS/rescaled/rejected, shift / rescales / first raw; status counts with errno names, every feedback URB in one bucket, first raw packets capped, data packet statuses and length mismatch; PCM peaks in dBFS and the first samples; packet sizer exact over ten minutes and every listed rate, feedback within one frame, packet cap; MD5 bit-exactness of the 16-bit path; ioctl numbers); src-tauri `android_usb_exclusive::tests` (guards, a remote track's reason, one test at a time); JS `tests/usb-exclusive-test.test.mjs`.
- Verified on the tablet (Android 15, the Sony USB speaker): SELinux lets the app issue `USBDEVFS_IOCTL` DISCONNECT/CONNECT and isochronous `SUBMITURB` on the `openDevice` fd; `snd-usb-audio` lets go and binds again (normal playback works afterwards); Q16.16 feedback at high speed; 16 × 1 ms URBs without underrun.
- Not verified: the phones (NX809J, others); what AudioFlinger and the playback service do while the DAC is gone; clicks at start and end; that the hand-back works after a stop in the middle and after unplugging during the test; that bit-perfect really reaches the DAC (needs a DAC that shows the incoming format or a digital loopback).
- Future work beyond P0: a real output path (other bit depths and rates, 24-bit packing, volume, gapless, seek, DSD/DoP over alt 7), choosing among several USB devices, and keeping Media3 away from the DAC while it is taken.
