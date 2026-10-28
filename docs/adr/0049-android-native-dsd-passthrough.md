# Native DSD passthrough on Android: a DSF's raw bits to the USB DAC

Status: Accepted (built and unit-tested; **not yet run on a device**)

## Context

ADR 0016 plays DSD on Android by converting it to PCM (176.4 kHz float). ADR 0047 drives a USB audio device over usbfs from Media3's output. Many USB DACs take DSD natively on a second alternate setting (UAC2 `RAW_DATA`, `bmFormats` bit 31): the Sony USB device 054C:0B8C (UAC2, high speed) has alt 7 = RAW_DATA, 4-byte subslots, 32 bits, 2 channels, the same isochronous OUT 0x01 (asynchronous) and feedback 0x81 as its PCM alts, and its clock source 0x29 lists 32, 44.1, 48, 88.2, 96, 176.4, 192, 352.8 and 384 kHz (fixture `crates/player-usb-audio/tests/fixtures/sony-054c-0b8c.txt`). Native passthrough sends the file's DSD bits to such an alt with no DSD-to-PCM conversion and no DoP (ADR 0020 P2 is the Windows way). The user chose this as the real feature, with no test UI.

## Decision

### The wire format

One 32-bit slot holds 32 consecutive DSD bits of one channel, which is four DSD bytes. A frame is the left slot, then the right slot (8 bytes). The device clock runs at the DSD bit rate ÷ 32: DSD64 (2 822 400) → 88 200 Hz, DSD128 → 176 400 Hz, DSD256 → 352 800 Hz. DSD512 would need 705.6 kHz, which no device lists: refused. Only stereo files of the 44.1 kHz family qualify (`player_dsd::raw::raw_transport_rate`). The bits are most-significant-first on the wire, which is what every `DsdStream` already hands out (DSF stores LSB-first; `DsfReader` reverses).

The order of the four bytes inside a slot is **not verified on any device**. Default **order A**: the oldest DSD byte first on the wire, bytes in time order (Linux `DSD_U32_BE` memory order). **Order B**: the word reversed (`DSD_U32_LE`). `SlotByteOrder` is a plain parameter of `pack_raw_words`. The hidden field `settings.json` `dsd.nativeByteOrder` (`"a"` default, `"b"`; no UI; `DsdNativeByteOrder` in Rust) chooses it for the next file opened. If the first listening test is noise or a harsh buzz, set it to `"b"`.

### Silence, lead-in, lead-out

DSD silence is the byte `0x69` (`01101001`, no DC). A zero word is a full-scale step to a DSD DAC, so **nothing in this path is ever zero**: every gap is `0x69`: the stream's underrun and pause fill, an empty or ended ring, the lead-in and the lead-out, the discarded start of a ring, the tail of the last slot when fewer than four bytes remain (`pack_raw_words`), and the invalid low bits of the last DSD byte when the sample count is not a multiple of 8 (`idle_padded_byte`).

- Lead-in: 150 ms of `0x69` after every start of streaming (after the alt and the clock were set, so also after a resume from pause), before the ring is read (`dsd::Gate`). A seek inside a running stream does not repeat it.
- Lead-out: before the alt goes back to 0 (pause, release, idle hand-back, the orphan grace ending), the stream keeps running idle fill for 150 ms plus the queued URBs (`Runtime::lead_out`); the ring is left alone, so unplayed music goes out again on resume. This blocks the device thread (and so the `pause` request) for about 170 ms. A device failure skips it.

### The sequence on the bus

The same as PCM: claim → alt 0 → GET RANGE → SET INTERFACE alt 7 → SET CUR (frame rate) → settle 50 ms → GET CUR read-back (a mismatch aborts and hands the device back) → stream. The engine state machine is unchanged: the alt (7, never a PCM alt) makes the format differ from every PCM format. `plan_raw_stream` finds the RAW_DATA alt (`StreamPlan.raw`), `format::choose_dsd` checks the rate against the RANGE and the alt, with `Unsupported::DsdRate` (「音響沒有 DSD 取樣率 88.2 kHz」) and `NoDsdAlt` as reasons. Packet sizing and feedback are the PCM code at the frame rate. There is no gain; the engine copies ring bytes unchanged.

### Getting the slots through Media3 (what was verified in 1.11.1)

The slots reach the output as the PCM Media3 writes. Reading the 1.11.1 bytecode found two hazards and one limit:

1. `DefaultAudioSink.configure` builds its pipeline as: if the encoding is 24/32-bit integer or float and float output is on (it is, for ADR 0016's converters), **its own** `ToFloatPcmAudioProcessor` is added and the app's `AudioProcessorChain` is not consulted at all. A 32-bit integer carrier would therefore be turned into float and destroyed, and nothing the app owns can prevent it (the class is final, the method private). **The carrier is therefore 16-bit PCM with 4 channels** at the frame rate: 8 bytes per frame, byte for byte the two slots. For 16-bit input the sink uses the app's chain, and `ToInt16PcmAudioProcessor` and the other defaults are inactive.
2. That chain (`FloatAwareAudioProcessorChain`) adds the app's `ToFloatPcmAudioProcessor` for 16-bit PCM. It takes a gate: `FloatAwareAudioProcessorChain { UsbExclusiveEngine.source.nativeDsd }` leaves the converter out while the loaded track is native DSD. The **flag**, not the format, decides: a genuine 88.2 kHz 16-bit 4-channel PCM would look the same. Media3 asks the chain for its processors at every `configure`, so the flag set at load applies to that track. Speed 1 and silence skipping off keep the remaining processors inactive (tested).
3. `DefaultAudioSink.maybeRampUpVolume` scales the **data itself** (`PcmAudioUtil.rampUpVolume`) for the first 20 ms of frames written after every flush or new output, for any linear PCM. It cannot be disabled. A new native-DSD ring therefore replaces the first 25 ms written (`dsd::discard_frames`: ceil(rate × 25 ms), whole frames, so never half a slot) with `0x69` (`Ring::new_dsd`); after a Media3 flush the output creates a new ring, so every ramp is covered.

Also: `setVolume` (the app volume and the audio-focus ducking arrive there) is ignored for native DSD (`DsdPassthroughRules.gainFor`); there is no gapless for DSD (one item at a time, a new output per track); seek works because the WAV is byte-addressable and a frame is 4 DSD bytes per channel (the position clock is frames ÷ frame rate, which is DSD seconds). The provider's `getFormatSupport` answers "supported directly" for the carrier (up to 352.8 kHz, beyond what the AudioTrack provider would be asked about).

### The raw WAV schemes

A DSF or a WavPack file that holds DSD (codec WVDSD), local or remote, is served by the same `DsdDataSource`/`DsdBridge` (`nativeOpenRaw`) as a WAV (`RawDsdWavStream<Box<dyn DsdStream>>`) of 16-bit PCM, 4 channels, rate = frame rate, whose data is the packed slots (header rate 88200/176400/352800 is a fake PCM rate; the data is not audio to anything but the USB output). Random access seeks the stream to frame × 4 bytes; there is no filter and no pre-roll. Local and remote files are not told apart anywhere in the decision or the carrier: both build a `DsdStream` (`DsfReader` or `WavPackDsdStream`, the readers the PCM converter already uses; WavPack-DSD is MSB-first, bit for bit the DSF's, so no bit-order step exists) and wrap it, over a file or over the remote reader (`RemoteStreamAdapter`).

| Load URI Rust sends | Raw form Kotlin loads (`DsdPassthroughRules.rawUriOf`) |
|---|---|
| `moedsd:///path/file.dsf` | `moedsdraw:///path/file.dsf` |
| `moewv:///path/file.wv` | `moewvraw:///path/file.wv` |
| `moeremote://<source>/<path>` (`.dsf` or `.wv`) | `moeremoteraw://<source>/<path>` |

The remote raw form is a scheme of its own, not a `?raw` query: `parse_remote_locator_uri` takes a `?` as part of the path. Rust rewrites `moeremoteraw://` to `moeremote://` before it opens the file (`remote_uri_of_raw`). All three are registered in Rust (`android_dsd`: `DSD_RAW_URI_SCHEME`, `WAVPACK_RAW_URI_SCHEME`, `REMOTE_RAW_URI_SCHEME`, `serves_converted_wav`, `raw_uri_of`) and in Kotlin (`LOCAL_CONVERTED_SCHEMES`, `isRustServedScheme`, `RAW_URI_SCHEMES`; a scheme missing on one side fails silently, see AGENTS.md). Rust never produces them. A remote open that fails because the server cannot be reached returns -2 from `nativeOpenRaw`, like `nativeOpen`.

### The decision at load time

Rust sends the usual load (`moedsd://`, `moewv://` or `moeremote://`) with `metadata.dsd`. The service (`DsdPassthrough.decide` → pure `DsdPassthroughRules.decide`) swaps the URI for its raw form when **all** of these hold, checked in this order (the file and the descriptors are read only when everything before them passed); the first failure is the reason for the card:

| Condition | Reason shown (after 「DSD 直通無法使用：」) |
|---|---|
| `dsd.nativePassthrough` is on and the track is DSD | (nothing is shown when off or not DSD) |
| a load scheme that can be carried: `moedsd`, `moewv` or `moeremote` (local and remote alike) | 只支援雙聲道 DSD64、DSD128、DSD256 的 DSF 與 WavPack-DSD |
| the exclusive mode is on | 不是獨佔模式 |
| the routing table plays it on a USB audio device | 沒有 USB 音訊裝置 |
| USB permission held | 未取得 USB 權限 |
| the file is a stereo DSD64/128/256 DSF or WavPack-DSD (`nativeRawPlan`) | 不是雙聲道／DSD512 超出 USB 音訊的取樣率上限／不是 DSF 或 WavPack-DSD 檔案／這不是 DSD 的 WavPack 檔案／無法讀取 DSD 檔案 |
| the device offers the frame rate and the RAW_DATA alt (`nativeSupportsDsd`) | 音響沒有 DSD 取樣率 88.2 kHz／音響沒有 DSD 直通格式 |

Otherwise the track is converted to PCM as before (ADR 0016) and the reason is kept in `UsbExclusiveEngine.dsdReason` until the next load.

**The file plan is read off the main thread.** `decide` runs on the main thread, and a remote header is a round trip to a server. `nativeRawPlan` is therefore called on the preflight thread (`PlaybackService.preflightLoad`, `moe-playback-preflight`), after `MediaPreflight.check` passed, and only when `DsdPassthroughRules.wantsFilePlan` holds (the setting, a DSD track, a passthrough scheme, the exclusive switch). The answer travels by command id (`precomputedRawPlans`) to `decide`, whose `filePlan` only parses it; there is one mechanism for local and remote files. The preflight already opened the file (and for a server paid for the connection) and has `PREFLIGHT_TIMEOUT_MS` and the server-unreachable handling, so a load whose server is down is rejected exactly as on the PCM path. A plan that cannot be read after a passed preflight is not a failed load: the reason is 「無法讀取 DSD 檔案」 and the track plays through the PCM converter. In Rust the plan reads the header through the provider's cached reader (`RemoteContent::open_cached`: the disk cache, no read-ahead; a DSF needs its first ~92 bytes, WavPack what the parser needs for the header), so the first 1 MiB block is cached and reused when playback opens the streaming reader. Each seek on a remote raw stream is a random read of frame × 4 bytes through the same reader as the PCM converter.

### Never an AudioTrack

The raw WAV must never reach an AudioTrack. When the track was loaded raw (any of the three schemes; the guard looks at the engine's `nativeDsd` flag and the carrier format, never at the scheme) and the provider cannot give a USB output (the device went away, the mode was switched to shared, the attach failed: a rate the RANGE refuses), `getAudioOutput` throws `AudioOutputProvider.InitializationException`; Media3 reports a player error; `PlaybackService.retryDsdThroughPcm` (first thing in `onPlayerError`) sets the track to PCM (`dsdPassthroughFailed`, reason kept for the card), reloads the item with its original URI (`moedsd://`, `moewv://` or `moeremote://`) at the current position, and the track goes on through the converter, on USB as PCM or through Android as the routing table says. The error is not reported and the pending load ack still completes. One such retry per load.

### Settings

`settings.json` `dsd`: `gainDb`, `outputRateHz`, `dop` and the new `nativePassthrough` (bool, default false) and hidden `nativeByteOrder` (`"a"` default / `"b"`). `settings_set_dsd` takes optional `nativePassthrough`/`nativeByteOrder`, old files and junk fall back field by field. The setting only matters at load: the service reads it over JNI (`nativeDsdPassthroughEnabled`) at each load; a change on the page updates the Rust copy at once and applies from the next track, so there is no separate push. The byte order is read from Rust when the file is opened.

### The page

Settings → playback → 「DSD 播放」 (Android only, the same `.resampling-option` markup as the Windows DoP checkbox): 「DSD 直通（獨佔模式）」. Its description depends on the output mode, which the page reads from `AndroidOutputDevicesSettings` (`onExclusiveChange`): exclusive: 「DSD 檔直接送到 USB 音訊裝置，不轉成 PCM；上面的增益與輸出取樣率、播放器音量都不再適用，音量請在音響或擴大機調整。只支援雙聲道 DSD64、DSD128、DSD256 的 DSF 與 WavPack-DSD；音響必須提供對應的取樣率，否則該曲自動改為轉換成 PCM。」; shared: 「DSD 直通只在獨佔模式下使用；請先在上方「輸出模式」選擇獨佔。」. No dialog, no test button. The playing card line (`describeAndroidExclusive`, from the engine status `dsdNative`): 「DSD64 → 88.2 kHz（DSD 直通）．2 聲道」 (DSD128 → 176.4, DSD256 → 352.8), no bit depth, no 「位元完美」, no volume reason (the engine status clears `notBitPerfect` and `bitPerfect` for native DSD). A DSD track that plays as PCM although the setting is on carries 「DSD 直通無法使用：…」 in the notice of its row (`exclusive.dsdFallbackReason`).

## Safety rationale

A DSD DAC fed PCM-style data, a wrong byte order or zeros makes loud noise that can hurt speakers and ears. The path therefore never invents a zero, never lets the app volume scale the slots, never plays the slots through an AudioTrack, rejects anything but the three stereo rates, and falls back to PCM on every doubt. The setting is off by default and needs the exclusive mode.

## Verification status

Unit-tested on the host (slot packing in both orders, tail padding, rate mapping, WAV header and random access, plan on the Sony fixture, RANGE check, ring discard, lead-in/out and fill invariants, the Kotlin decision, the sink gate, volume rules, settings, page texts) and compiled for aarch64. **Nothing was run on a device by the author.** The user reports on the tablet with the Sony USB DAC (054C:0B8C): a local DSF of DSD64 and DSD128 plays correctly (byte order A is right, track changes included), a remote DSF of DSD128 plays correctly (header read through the cached reader on the preflight thread); DSD256 is silent although the USB side is healthy (alt 7 selected, clock read back as 352.8 kHz, feedback 44.1 slots per microframe, no underruns or errors), so this device does not decode DSD256 natively. WavPack-DSD, remote seek and NX809J are not verified. To verify, with a stereo DSD64 DSF, the volume of the amplifier at minimum: (1) the track starts without noise (if noise, set `dsd.nativeByteOrder` to `"b"`), (2) the card shows 「DSD64 → 88.2 kHz（DSD 直通）」, (3) pause, resume, seek, next track and unplugging, (4) DSD128 and DSD256, (5) the fallbacks and their reasons, (6) clicks at the start and end. Open questions: the byte order of this device; whether Media3's WAV extractor and renderers accept 4-channel 16-bit at 352.8 kHz on the devices (the provider answers support itself); NX809J's SELinux for usbfs (ADR 0047).
