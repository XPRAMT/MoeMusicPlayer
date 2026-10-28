# USB exclusive output (P1): Media3 plays straight to a USB audio device

Status: Accepted (built and unit-tested; not yet run on a device)

Decision date: 2026-10-28, Asia/Taipei (UTC+08:00)

## Context

ADR 0046 (P0) proved on the tablet with the Sony USB speaker (054C:0B8C, UAC2, high speed) that the app can take a USB Audio Class device from the Android audio system over usbfs, stream PCM paced by explicit feedback, and give the device back so Android plays on it again. The required order is claim → alt 0 → GET RANGE → the streaming alt → SET CUR the rate → settle → GET CUR read-back → stream. P0 was a development button that decoded 30 s of a 16-bit FLAC in Rust. P1 makes it the real output path of the player: every track Media3 plays, with seek, pause, volume, track changes and fallbacks, so the lyrics and the progress clock stay right. The P0 button, its commands (`android_usb_exclusive_start` / `_stop` / `_status`, plugin `usbExclusiveStart`), the Kotlin `UsbExclusiveTest`, the page summary (`usb-exclusive-test.js`) and their strings are removed; the descriptor reader of ADR 0045 stays.

## Decision

### Where Media3 is hooked

Media3 1.11.1's `DefaultAudioSink` writes to an `AudioOutput` it gets from an `AudioOutputProvider` (`setAudioOutputProvider`). P1 adds one provider in front of the existing chain, and writes no sink of its own:

`UsbAudioOutputProvider(context, RouteObservingOutputProvider(AudioTrackAudioOutputProvider), preference)`

- `getFormatSupport`, listeners, `setClock`, `hasPendingReleases` are the delegate's: Media3 decodes and processes exactly as before (float PCM out of `FloatAwareAudioProcessorChain` or the converters of ADR 0016 / 0025 / 0043).
- `getOutputConfig`: the pure rule `UsbExclusiveRules.choose` decides. USB only when the switch `androidUsbExclusive` is on, the chosen output device (ADR 0044) is of kind `usb`, a matching USB audio device is attached (`UsbExclusiveRules.pickDevice`: the USB product name equal to the chosen device's name, else the only USB audio device), USB permission is held, the encoding is linear PCM (16, 24 packed, 32, float), this track has not already fallen back, and the device's descriptors offer the rate, channel count and a container for the source bit depth (`nativeSupports` → `format::choose`, with the clock RANGE remembered from an earlier claim of the same vendor/product). Then the provider builds its own `OutputConfig` (the PCM encoding, rate, channel mask, a 250 ms ring as `bufferSize`, no offload, tunneling or playback parameters); otherwise the delegate's config, and a reason when the user expected USB.
- `getAudioOutput`: the same rule decides again, from the config's own encoding, rate and channel count (`Integer.bitCount(channelMask)`), for any config that is plain PCM (`takesConfig`: no offload, tunneling or output playback parameters), so an output Media3 re-creates for the config it already has (a seek, a stalled output, a change of the switch or the device, below) follows the switch and the device as they are now. USB opens a `UsbAudioOutput`; when it cannot start (the device does not open, the attach fails: rate not in RANGE, clock read-back wrong) the reason becomes the track's fallback and the same config is given to the delegate, which opens an AudioTrack. Media3 does not notice. Every AudioTrack output is handed to Media3 wrapped in `RetirableAudioOutput` (all calls forwarded); the provider keeps the output it handed out last (`live`) until Media3 releases it.
- `release`: the device is handed back, then the delegate is released.

`AudioOutput` and `AudioOutputProvider` are `@UnstableApi`: a Media3 update can change them (see Risks).

### The output (`UsbAudioOutput`)

| Media3 call | What happens |
|---|---|
| `getAudioOutput` | a Rust `Ring` (250 ms of the output's encoding); `nativeAttach` → device thread `Attach(format)` |
| `write(buffer, …)` | whole frames copied into the ring from the direct buffer (`GetDirectBufferAddress`; a heap buffer is copied to a direct one first); never blocks; answers whether the buffer was taken whole; nothing is taken once the ring is failed |
| `play` | `Play`; when the claim ended meanwhile (10 s paused), a new claim is started and the same ring attached again, then `Play` |
| `pause` | `Pause` |
| `flush` | the ring is detached and replaced by an empty one (attached with the same format, so the stream keeps running with silence); Media3 1.11.1 never calls it (its `DefaultAudioSink.flush` releases the output instead) |
| `stop` | the ring is marked ended: after it drains, silence follows and is not an underrun |
| `release` | `Detach`, the ring is freed, `onReleased` |
| `setVolume` | the gain (applied in Rust before packing) |
| `getPositionUs` | frames played on the bus (below) |
| `isStalled` | true once the ring is failed; Media3 then releases the output and asks the provider again, which gives an AudioTrack (the track's fallback is set) |
| `getBufferSizeInFrames`, `getSampleRate`, `getAudioSessionId` | the ring size, the rate, the config's session id |
| offload, aux effects, preferred device, playback parameters | no-ops / `PlaybackParameters.DEFAULT` (speed is Media3's processors' job) |

Listeners get `onUnderrun` when the ring's underrun count grows (checked on each write) and `onPositionAdvancing` once the position first moves while playing.

### Position and latency

The ring has three growing byte counters: `written` (Media3 copied in), `sent` (packed into URBs; the device thread's own cursor) and `played` (came back from the bus in completed URBs). Each data URB remembers which ring its packets came from and how many input bytes each packet took; when the URB is reaped, those bytes are added to `played` (every packet: an errored packet's time has passed too). `getPositionUs = played frames × 10⁶ / rate` (`frames_to_us`, rounded down). So the up to 16 queued URBs (16 ms at high speed) are never counted as played, and nothing is hidden: the position is what the device has been sent, at the device's own clock (the feedback-paced sizer of ADR 0046), with 1 ms granularity from URB completions. Media3 caps it at the frames written, so `hasPendingData` ends a track exactly when the last frame came back. What the device buffers internally after the bus is not known and not added.

The producer may fill only up to `played + capacity`, so the bytes of a URB still in flight are never overwritten. When streaming stops (pause, format change), every URB is discarded and reaped; packets that went out (status 0 with bytes) count as played, the rest is rewound (`sent = played`) and goes out again on resume, so a pause loses no audio and the position stays continuous. A new output starts a new ring at position 0, as a new AudioTrack does.

### State machine (`engine.rs`, pure)

States: `Unclaimed` → `Claimed` (interfaces claimed, alt and clock set, no URBs) → `Streaming` → `Paused` (URBs stopped, alt 0, still claimed) → `HandedBack` (the end of one claim). Actions: `Claim`, `Configure(format)` (alt 0, RANGE, alt, SET CUR, settle, GET CUR, ADR 0046 order), `StartStreaming`, `StopStreaming`, `HandBack`.

- `Attach(F)` (a new output): Unclaimed → Claim, Configure(F) → Claimed. Claimed with another format → Configure(F). Paused → stays Paused with F (configured on play). Streaming with the same F → keeps streaming, the new ring takes over at the next packet (a seek or the next track of the same format: no alt or clock change). Streaming with another format → StopStreaming, Configure(F) → Claimed, waiting for `Play`.
- `Play`: Claimed → StartStreaming; Paused → Configure(F), StartStreaming (no new claim).
- `Pause`: Streaming → StopStreaming → Paused.
- `Detach` (the output released; Media3 releases it on every seek and track change): Streaming goes on with silence (orphaned); after `ORPHAN_GRACE_MS` (1 s) without a new output → StopStreaming → Paused.
- `Tick`: Claimed or Paused for `IDLE_RELEASE_MS` (10 s, as Windows exclusive mode's `IDLE_RELEASE`) → HandBack.
- `Release` (the service's `stop`, the switch turned off, the playback service ending, the provider released) and `DeviceError` (poll error, a reap error, a submit failure with nothing in flight) → HandBack from any state. A release or an error also fails the attached ring, so the output stalls and Media3 moves to an AudioTrack for the rest of the track.

The playback service sends the switch and the track's source with every load (`newTrack`: the track's fallback is cleared, so each track tries USB again); the service's `stop` and `onDestroy` release.

### A change applies to the track that is playing

The choice was first made only when Media3 configured an output, so on the tablet turning the switch on during a track did nothing until the next track. Now a change of the switch (`applyUsbExclusive`) or of the chosen output device (`applyOutputPreference`) goes through `PlaybackService.changeOutput` on the main thread: the pure rule `UsbExclusiveRules.outputChange(before, after, hasOutput)` compares the two `Wish`es (switch, chosen device; `usb` = switch on and a USB device chosen). No change, or no USB on either side (speaker, Bluetooth: the router re-pins the AudioTrack as before) → nothing. Otherwise: the track's fallback is cleared (the choice is made again for this track), the new state is stored first, then `newOutput` (an output is open) retires the output Media3 plays through (`UsbAudioOutputProvider.retireCurrent`), and `handBack` (USB was wanted before) gives a held device back at once (`UsbExclusiveEngine.release`).

The mechanism is Media3's own path for a dead output: a retired output (`UsbAudioOutput.retire`, `RetirableAudioOutput.retire`) takes no more data and reports `isStalled`; on its next `handleBuffer` (within one render loop while playing; at the next render while paused, at most about a second, or at play) `DefaultAudioSink` logs "Resetting stalled audio output", releases it (`flush`) and builds a new one with the config it has through `getAudioOutput`, which decides again (above). Position and play/pause state are the player's and do not change; the sink restarts its media time at the next buffer, so the position stays monotonic. A retired output, or the switch being off, never records a fallback reason, and a retired paused USB output does not claim the device again at play. Turning the switch off is the release path of before (the ring fails, the output stalls, an AudioTrack follows), now without a stale fallback.

What is lost: the audio already written to the old output and not yet played, and what the sink's processors held, is dropped, so the music jumps ahead by about the old output's buffer (Media3's AudioTrack buffer for PCM, at least 250 ms; the USB ring, at most 250 ms), followed by the new output's start (a claim and the clock settle, tens of milliseconds, when it is USB) and Media3's 20 ms ramp. Rejected: a seek to the current position would re-decode without dropping anything, but ExoPlayer ignores a seek to the millisecond it is already at (`ExoPlayerImplInternal.seekToInternal`, BUFFERING or READY), which is always the case while paused and can be while playing, and it reports a seek and a buffering state to Rust; `setAudioSessionId` / `setAudioAttributes` reach `DefaultAudioSink.reconfigureAndFlush` (which asks `getOutputConfig` again) but drop the same buffered audio and change the session or the attributes; `AudioOutputProvider.Listener.onFormatSupportChanged` only makes the renderer re-select tracks, which keeps the same track and output.

### Decision log

Every decision is logged (tag `MoeMediaIndex`, prefix `USB exclusive:`), once per output config or output, never per write: `UsbExclusiveRules.decisionLog` gives where (`getOutputConfig`, `getAudioOutput`), the result with its reason (`explain`, in the order of `choose`) and all facts, e.g. `USB exclusive: getAudioOutput → USB: every check passed; enabled=true chosenKind=usb deviceAttached=true permitted=true encoding=4 rate=48000 channels=2 sourceBits=16 dsd=false trackFallback=null unsupported=null` or `USB exclusive: getOutputConfig → AudioTrack: switch off; enabled=false …`. A config that is not plain PCM logs its offload, tunneling and playback-parameter flags. `openUsb` logs the device it opened (`openUsb → USB: /dev/bus/usb/002/002 rate=… channels=… encoding=… sourceBits=… dsd=…`) or why it could not (`openUsb → AudioTrack (無法開啟 USB 音訊裝置)`, the attach's reason). A change logs `USB exclusive: switch changed (usb wanted false → true, chosen usb:… → usb:…); re-creating the output now` (or `chosen output device changed`, `no output to re-create`, `, device handed back`).

### Runtime and threads

One device thread per claim (`moe-usb-audio`, `THREAD_PRIORITY_URGENT_AUDIO`, started by Kotlin `UsbExclusiveEngine`, running `usbfs::run` through `nativeLoop`) owns the file descriptor and all URBs. Requests from Media3's playback thread (attach, play, pause, detach, release) go over a channel and are answered once carried out (3 s timeout, so a device that stops answering cannot hang Media3). While streaming, the thread waits for completions with `poll` (20 ms) and reads requests between waits; otherwise it sleeps on the channel (200 ms) and ticks the timers. Every reaped data URB is refilled at once: per packet the sizer's frame count (feedback-paced, ADR 0046), ring bytes packed with the gain, the rest silence. An underrun is counted once per dry spell while a ring that already delivered audio is attached and not ended. The `UsbDeviceConnection` is closed by Kotlin only after the loop returned (the device given back); `UsbExclusiveEngine.held` is the output device as Android listed it before it disappeared.

### Formats and bits

- No resampling and no channel mixing: an unsupported rate or channel count falls back with the reason (`USB 音訊裝置不支援 … Hz`, `… 沒有 N 聲道的 PCM 格式`) and is tried again on the next track.
- The alt follows the source's bit depth, which Rust sends with the load (`metadata.bitsPerSample` from the library's `bit_depth`; `dsd` from the codec name), because the output receives float whatever the file was: 16 → 2-byte/16 (else 3/24, 4/24, 4/32, 3/16, 4/16); 24 or unknown → 3/24 (else 4/24, 4/32); 32 → 4/32 (else 4/24, 3/24); DSD (converted to PCM as before) → as 24. On the Sony device: 16-bit → alt 6, 24-bit → alt 4, 32-bit → alt 1.
- Packing (`pcm::pack`): float × 2^(valid bits − 1), rounded and clamped; integers are shifted (exact when widening). Samples sit left-justified in the subslot, little-endian (4 bytes with 24 valid bits has a zero low byte). A float sample of an n ≤ 24-bit integer is that integer / 2^(n−1) exactly (the converters divide a left-aligned i32 by 2^31; f32 has a 24-bit significand), so the packed sample is the file's sample: tested for every 16-bit and every 24-bit value and by the FLAC MD5 of a 16-bit and a 24-bit fixture.

### Volume and bit-perfect

The app volume (`setVolume`, including Media3's audio-focus ducking) is a gain applied in Rust before packing; the system volume keys do not reach the device. Bit-perfect (`format::bit_perfect`) needs: a known integer source depth that fits the container, not DSD, not a 32-bit source that went through float, and gain exactly 1.0. The status lists the reasons (`volume`, `float32`, `unknownBits`, `dsd`, `truncated`); the page shows 「44.1 kHz → 44.1 kHz（獨佔）．2 聲道．16bit．位元完美」 through the shared Windows formatter (`describeAndroidExclusive` → `describeExclusive`), the reasons in the hint. Media3's `DefaultAudioSink` ramps the first 20 ms after every new output (start, seek) up from silence (`maybeRampUpVolume`); those frames are not the file's and the label does not mention it.

### Router while the device is held

Detaching the kernel driver makes Android remove the USB output. `AudioOutputRouter` asks `UsbExclusiveRules.keepsPin`: while the chosen device is the held one, a device change does not re-pin (no move to the system default), and the status lists the held device as it was (`withHeld`), so the chosen row stays attached and shows the exclusive line, never 「目前由 … 播放」. When the claim ends, `UsbExclusiveEngine.onHeldChanged` makes the router reconcile; the device comes back under a new id and is pinned again.

### Setting, page and permission

`settings.json` `androidUsbExclusive` (default false, Rust `AppSettings::default`, `#[serde(default)]`, JS `DEFAULT_ANDROID_USB_EXCLUSIVE`), commands `settings_get/set_android_usb_exclusive`; Rust keeps a copy (`android_output::set_usb_exclusive`) that the service reads at create and before each load (`nativeUsbExclusiveEnabled`: -1 unknown, 0, 1), and a change is pushed (`setUsbExclusive`). On the USB card: the switch 「USB 獨佔（App 直接驅動，不經過系統）」. Turning it on opens the explanation panel first (ADR 0039): while exclusive, other apps and notification sounds play from the phone or tablet speaker, the system volume has no effect, bit-perfect needs the app volume at 100 %, and the device may have no hardware volume. Only its 「前往授權」 asks for USB permission (`android_request_usb_exclusive_permission` → `requestUsbExclusivePermission`, the system dialog of ADR 0045); only a granted permission saves the switch on, and it also chooses that card. Turning it on or off applies to the track that is playing (see "A change applies to the track that is playing"). `android_audio_output_status` carries `exclusive` (`enabled`, `active`, `state`, `fallbackReason`, `engine`: rate, read-back, channels, alt, subslot bytes, bits, source bits, DSD, reasons, gain, underruns, feedback average, URB and packet errors, error, hand-back result). While the device is back with Android after 10 s paused the row says 「USB 獨佔：裝置已暫時交還系統，播放時再取得」.

### Licensing

Written from the Media3 1.11.1 sources (Apache-2.0, read for the interface contracts only), the USB Audio Class specifications and the Linux usbfs header; nothing copied from other players. No GPL/AGPL code was read.

## Consequences

- Unit tests: `cargo test -p player-usb-audio` (ring: whole frames, room only after played, rewind, wrap-around; packing: every 16-bit and 24-bit sample through float in each container, clamping, integers, gain, FLAC MD5 of 16- and 24-bit fixtures; alt choice on the Sony fixture, channels and remembered ranges; bit-perfect rule; state machine: attach/play/pause, resume within 10 s, 10 s hand-back, orphan grace, format change, release and errors; position math), src-tauri `android_usb_output::tests`, `android_output::tests`, `settings::tests`, `android_playback::tests`; Kotlin `UsbExclusiveRulesTest` (provider choice, plain-PCM configs, decision log, when a change re-creates the output or hands the device back, device matching, router keep-pin and held listing, ring and position math); JS `tests/android-output-devices.test.mjs`, `tests/output-format.test.mjs`.
- Media3's playback thread blocks on a claim (opening the device, the clock settle) when an output is created: tens of milliseconds, like creating an AudioTrack.
- `getAudioOutput` decides from the config alone, so an AudioTrack config of a track that started on Android can be played by the USB output after a change; the USB output ignores the config's buffer size (its ring is 250 ms).

## Risks and what to watch on the device

- `AudioOutput` / `AudioOutputProvider` are `@UnstableApi`; a Media3 update must be checked against this ADR (write semantics, `flush` releasing the output, the 20 ms ramp, `canReuseAudioOutput`).
- SELinux on the NX809J (and other phones) is not verified; only the tablet allowed usbfs DISCONNECT/CONNECT and isochronous URBs (ADR 0046).
- When Android removes the USB output, the AudioTrack provider reports changed capabilities; Media3 forwards it to the renderer. If that ever re-selects tracks or reconfigures the sink, a glitch right after the claim would show it.
- Watch: the position (lyrics and the progress bar against the audio), pause/resume within and after 10 s, seek and track changes of the same and of another format (silence between, no clicks), every listed rate and 16/24/32-bit files, the volume below 100 %, unplugging while playing (fallback to an AudioTrack and the reason), turning the switch on and off while playing and while paused (the jump ahead, the time until sound; whether the AudioTrack plays from the speaker until Android lists the USB output again after the hand-back), changing the chosen device while exclusive, stop and app exit (normal playback afterwards), notification sounds while exclusive, underruns under load, and the fallback when the rate is not in RANGE.
