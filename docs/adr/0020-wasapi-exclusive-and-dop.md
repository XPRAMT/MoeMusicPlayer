# WASAPI exclusive output and DoP (plan)

Status: Proposed (planning only, nothing implemented)

Decision date: 2026-10-18, Asia/Taipei (UTC+08:00)

## Context

The user asked for two features: an **exclusive output mode** and **DoP** (DSD over PCM) for DSD files. ADR 0016 already reserved the DoP seam (`DopPacker`, `plan_dsd_route`, a second `AudioBackend`), and ADR 0017 and the Opus work did not touch the output path. This document turns those reservations into a concrete plan.

What exists now (read from the code, 2026-10-18):

- Output is **WASAPI shared mode through rodio 0.22.2 / cpal 0.17.3** (`RodioBackend`, `crates/player-audio-windows/src/rodio_backend.rs`). It opens the **default endpoint only**; there is no device picker. It can ask the shared-mode engine for the track's rate (`ResamplingMode::WindowsBuiltin`, `AUTOCONVERTPCM`) or run rubato at the mix rate (`HighQuality`). A DSEE HX DirectShow filter can sit in the chain below 48 kHz.
- The seam is `AudioBackend` (load, play, pause, stop, seek, set_volume, position, error) behind `Box<dyn AudioBackend>`; the worker thread owns one backend and recreates it through a `BackendFactory` on failure. Decoders are rodio `Source`s producing `f32` (rodio `Decoder`, `DsdPcmSource`, `OpusSource`).
- DSD is converted to 32-bit float PCM (176.4 or 352.8 kHz, gain +0…6 dB) and goes through the same shared path. `DsdStream` already yields **MSB-first planar bytes** and exact positions, which is what DoP needs.
- On this machine the active render endpoints are `Realtek Digital Output` (S/PDIF, exclusive allowed), two Bluetooth headsets, `CABLE Input` (virtual) and HDMI/NVIDIA outputs. **There is no USB DAC**, so DoP cannot be verified by ear here.

## Research findings

### WASAPI exclusive
- `IAudioClient::Initialize(EXCLUSIVE, EVENTCALLBACK, period, period, format)`: in event-driven exclusive mode the buffer duration must equal the period and, on HD Audio devices, be **aligned** (`AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED` → read `GetBufferSize`, recompute the duration, initialize again).
- The format is whatever the endpoint accepts: `IsFormatSupported` with `WAVEFORMATEXTENSIBLE` (16, 24-in-32, 24 packed, 32 bit; some devices only accept plain `WAVEFORMATEX`). There is no engine conversion and no mixer; nothing else can play on that endpoint while we hold it.
- Needs "Allow applications to take exclusive control of this device" (on by default; the registry flag is visible per endpoint). Errors to handle: `AUDCLNT_E_DEVICE_IN_USE`, `AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED`, `AUDCLNT_E_UNSUPPORTED_FORMAT`, `AUDCLNT_E_DEVICE_INVALIDATED` (unplug).
- The render thread must be time-critical (MMCSS "Pro Audio") and must never wait on decoding: decode on another thread into a ring buffer.
- Bluetooth endpoints cannot do useful exclusive output; Android has nothing comparable (Media3), so this is Windows only.

### Bit-perfect PCM through our `f32` pipeline
Symphonia and rodio convert N-bit integers to `f32` by dividing by 2^(N−1). That is exact for N ≤ 24 (the `f32` mantissa has 24 bits), and converting back with round-to-nearest returns the original integers. So 16-bit and 24-bit files stay bit-perfect through the existing decoders; 32-bit integer sources are not exact (rare; reported as not bit-perfect). No decoder rewrite is needed.

### DoP (DSD over PCM, standard 1.1)
- Each PCM sample is 24 bits: the **top byte is a marker alternating `0x05`, `0xFA` per frame** (both channels carry the same marker), the low 16 bits are **16 consecutive DSD bits of that channel, oldest bit in the most significant position**. In a 32-bit container the 24 bits are left-justified (bits 31..8).
- PCM rate = DSD rate / 16: DSD64 → 176.4 kHz, DSD128 → 352.8 kHz, DSD256 → 705.6 kHz (DSD512 would need 1.4112 MHz: not offered). Only the 44.1 kHz family is common; 48 kHz-family DSD stays on the PCM path.
- Every bit must reach the DAC untouched: exclusive mode only, **no gain, no volume, no dither, no resampling**. Silence is the DSD idle pattern `0x69` inside the same marker framing, never PCM zeros (zeros break the marker pattern and the DAC drops out of DSD with a click).
- The DAC needs a few frames of valid markers to lock. Entering and leaving DoP clicks on many DACs, so send idle DoP frames before the first audio and after the last, and keep them flowing during pause and seek.
- DSD has no filter state, so seeking is exact and instant (position in bytes, aligned to 2 bytes per channel).
- Data rate is small (DSD256 stereo ≈ 5.6 MB/s).

### Library
- **`wasapi` 0.26.0 (MIT)** wraps exactly what is needed (`ShareMode::Exclusive`, event handle, `is_supported_exclusive_with_quirks`, `calculate_aligned_period_near`, device enumeration and properties) and depends on **`windows` 0.62.2, the version this project already pins**, so no duplicate Windows bindings. cpal has no exclusive mode. Using the `windows` crate directly is the alternative if `wasapi` proves limiting.

## Decision (proposed)

### Architecture
1. **A second backend, `WasapiExclusiveBackend`**, implements `AudioBackend`; a small **`RoutingBackend`** (the object the worker owns) holds a `RodioBackend` and a `WasapiExclusiveBackend` and chooses per `load` from the settings and the track. Shared mode stays the default and untouched.
2. **The WASAPI layer sits behind a trait** (`RenderEndpoint`: formats supported, open at format, write, period, latency, invalidated) so that format negotiation, buffering, underrun handling, seek, track change and DoP framing are tested with a fake endpoint and no hardware.
3. **Threads:** a decode thread pulls the existing `Source`s (or the DSD stream) into a lock-free ring buffer (200 to 500 ms); an MMCSS render thread copies from the ring buffer into the endpoint buffer on the event; an underrun writes silence (PCM) or idle frames (DoP) and is counted.
4. **Frame types:** PCM path writes the negotiated integer container (16, 24-in-32, 24 packed, 32) from `f32` with round-to-nearest and no dither at 100% volume; DoP path writes 24-in-32 words produced by `DopPacker`.
5. **Format negotiation** per track: try the source's native rate and depth first (container order: native depth, then 24-in-32, 32); if the endpoint refuses the rate, resample with the existing rubato path to the nearest supported rate (prefer integer ratios) and mark "not bit-perfect" with the reason; if nothing works, fall back to shared mode for that track and say why.
6. **Route planning** (`plan_dsd_route(info, settings, endpoint_caps) -> Pcm{rate} | Dop{rate}`): DoP only when the setting is on, the output is exclusive and the endpoint accepts the exact DoP format at that rate; otherwise PCM with a notice (the DSD gain and rate settings keep applying to the PCM route only).

### Behaviour rules
- **Volume.** Proposed: in exclusive PCM the app volume is a software gain applied only below 100% (100% is a pure pass-through, so bit-perfect when left at 100%); in DoP the volume control is disabled with an explanation (volume is set on the DAC or amplifier). The DSD gain setting is PCM route only. DSEE HX and the shared-mode resampling choices are disabled while exclusive is active.
- **Pause, seek, stop, track change.** PCM: silence. DoP: idle frames with continuous markers; stop sends about 100 ms of idle before closing; a track with the same format keeps the stream open (no re-open click); a format change closes and re-opens after a short mute.
- **Release when idle.** An option closes the exclusive stream after the player has been paused or stopped for a chosen time so other apps can play, and re-opens on resume.
- **Device loss** (`DEVICE_INVALIDATED`, unplug): stop, fall back to the default shared output and report; the existing worker recovery re-creates the backend.
- **Errors are actionable:** "the device is in use by another app", "enable exclusive control for this device in Windows sound settings", "this device does not accept 24-bit 176.4 kHz".
- Status display shows the real path: "Exclusive · 24-bit 96 kHz · bit-perfect", "Exclusive · DoP · DSD64 → 176.4 kHz", or the fallback reason.

### Settings and UI (Settings → Output, Windows)
`settings.json` gets an `output` block: `deviceId` (endpoint id or default), `mode` (`shared` | `exclusive`), `dsdRoute` (`pcm` | `dop`), `releaseIdleSeconds`. The page gets: an **output device list** (name, state, a refresh button), **output mode**, **DSD output method** ("convert to PCM" or "DoP, needs exclusive mode"), a read-only line with what the current track negotiated, and the release-when-idle option. Strings go through `tr()` in all four languages.

### Phases
- **P0. Output device selection (shared mode).** Enumerate render endpoints, persist the choice, open the chosen device by id, handle hot-plug. A prerequisite for exclusive (a USB DAC is rarely the default device) and useful by itself. About one session.
- **P1. Exclusive PCM.** `RenderEndpoint` trait and fake, `WasapiExclusiveBackend` (decode thread, ring buffer, render thread, negotiation, error mapping), `RoutingBackend`, volume policy, status, UI. Acceptance without hardware: fake-endpoint tests for negotiation order, alignment retry, underrun, seek, same-format track change, 16/24-bit round trip equal to the file samples. Acceptance with hardware: the Realtek digital output opens exclusive; a loopback or analyzer check if available. About two to three sessions.
- **P2. DoP.** `DopPacker` (pure, heavily tested), `plan_dsd_route`, DoP mode in the exclusive backend, idle/lock/pop handling, UI. Acceptance without hardware: packing tests (marker alternation, bit order, idle pattern, rate map, round trip: unpack the words and compare the DSD bits with the DSF source over a whole file); fake-endpoint tests for pause/seek/stop framing. Acceptance with hardware needs a DoP-capable DAC. About one to two sessions.
- **Later, not planned:** gapless across format changes, DSD512, native DSD over ASIO/UAC2, an exclusive mode for the Android build.

## Open questions for the user
1. **Is there a DoP-capable DAC to test with** (model, USB or S/PDIF)? Without one, P2 can be built and unit-tested but not heard; the Realtek digital output could carry DSD64 DoP (176.4 kHz) to an S/PDIF DAC.
2. **Volume policy** in exclusive PCM: software gain below 100% (proposed) or no volume at all.
3. **Do P0 (device selection) first?** Proposed: yes, it is needed for P1 anyway.
4. Should **release when idle** default to on (for example 30 s) or off?
5. May **DSEE HX** be disabled while exclusive mode is on? (It cannot be bit-perfect.)

## Risks
- DACs differ: some accept only 24 packed, some need a particular buffer alignment, some never leave DoP lock cleanly. The negotiation order and idle handling are the mitigation, and they can only be tuned on real devices.
- Exclusive mode takes the endpoint away from every other app, including system sounds. The release-when-idle option and clear status text reduce surprises.
- Render-thread glitches would be audible and are hard to see in tests: the ring buffer size, the MMCSS class and underrun counters must be exposed in a diagnostics line from the start.
- The worker's recovery and the status snapshot assume one backend; the routing backend must report the active route and keep `position()` honest (frames written minus the stream latency).
