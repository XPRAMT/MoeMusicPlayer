# DSD playback: one DSD stream layer, a PCM converter now, DoP later

Status: Proposed

Decision date: 2026-10-17, Asia/Taipei (UTC+08:00)

## Context

The library holds DSF files (99 files in the owner's collection: DSD64 ×91, DSD128 ×3, DSD256 ×5, all stereo). The Windows player decodes with rodio/Symphonia, which has no DSD support, so these files cannot be played. The output is WASAPI shared mode through cpal; the mixer, the high-quality resampler (rubato), volume and the playback accounting all assume PCM.

The project is source-available under PolyForm Noncommercial (README, `NOTICE`), which cannot be combined with GPL code. The mature Rust converter (`rdsd2pcm`) and its CLI are GPL-3.0, and FFmpeg builds with GPL parts are excluded too, so the converter is written here.

DSD can reach a DAC two ways: converted to PCM in software (works on any output, goes through the existing path), or packed as DoP (DSD over PCM) so a DSD-capable DAC receives the original bitstream. DoP needs WASAPI exclusive mode at the exact rate, no mixer, no volume and no resampling, so it is a different output path. It is out of scope now but must not require redoing the decoder.

## Decision

Split DSD handling into a container-neutral **DSD stream** and interchangeable **renderers**:

```
file ──► DsdStream (container, seek, normalised bit order) ──┬──► DsdPcmSource  (this ADR)  ──► rodio / rubato / mixer / WASAPI shared
                                                             └──► DopPacker     (future)    ──► exclusive WASAPI output
```

### Module layout (`crates/player-audio-windows/src/dsd/`)

- `stream.rs`: trait `DsdStream` and `DsdInfo` (channels, DSD sample rate, samples per channel, duration). Contract: `read(&mut self, out: &mut [Vec<u8>]) -> usize` returns planar bytes per channel with the **earliest bit in the most significant bit of each byte** (MSB-first), whatever the container stores, plus `seek_to_sample`, `position_samples`, and the idle pattern `0x69` for padding. Normalising once here means both renderers share one convention.
- `dsf.rs`: `DsfReader` implements `DsdStream` (written here; no dependency on `dsd-reader`). Parses the `DSD `, `fmt `, `data` chunks, de-interleaves 4096-byte per-channel blocks, reverses bit order when the file is LSB-first, clamps to the declared sample count and to the real file length, and exposes the trailing ID3v2 offset. Corrupt headers are `AudioError::Decode`; more than two channels is `AudioError::UnsupportedFormat` with a message (multichannel is not supported in this phase).
- `dff.rs` (later, same trait): uncompressed DSDIFF only; DST-compressed files stay unsupported.
- `filters.rs`: filter design (Kaiser windows) and the stage-A lookup table, built once per rate at first use.
- `pcm.rs`: `DsdPcmSource<S: DsdStream>`, a rodio `Source` (`f32`, channels from the file, fixed output rate, `total_duration`, `try_seek`).
- `dop.rs` (future): `DopPacker`, not created now.

### DSD→PCM conversion

Output rate default **176.4 kHz** (DSD64 ÷16, DSD128 ÷32, DSD256 ÷64, DSD512 ÷128; all integer ratios because DSD is the 44.1 kHz family). 352.8 kHz may be offered later; 192 kHz is not an integer ratio and is not offered by default. The existing rubato resampler then adapts to the device rate, so this stage only has to remove DSD's ultrasonic noise.

1. **Stage A, ÷8 by table lookup.** A linear-phase Kaiser FIR of 128 taps (16 groups of 8 taps) is folded into 16 tables of 256 `f32` entries, one per group, where each entry is the sum of the group's taps weighted ±1 by the byte's bits. One output sample is the sum of 16 lookups over the last 16 bytes. Bit `1` is `+1`, bit `0` is `−1`. Passband to 40 kHz within ±0.01 dB, stopband at least 110 dB from `R/8 − 40 kHz` upward. One byte in, one sample out, per channel.
2. **Stage B, halfband ÷2**, repeated until the target rate (DSD64: once, DSD128: twice, DSD256: three times). A 127-tap Kaiser halfband (64 non-zero taps), passband 40 kHz within ±0.01 dB, stopband at least 110 dB from `Fin/2 − 40 kHz`.
3. **Gain.** Samples are `f32` and are not clipped inside the source. A `gain_db` parameter exists from the start (default chosen after measuring the real files' peak headroom, recorded in the code and `AGENTS.md`); DSD's nominal 0 dB reference is 50% modulation, so an unscaled ±1 mapping puts it near −6 dBFS.
4. **Group delay.** Both stages are linear phase, so the total delay is constant. Output starts after that delay has been discarded; at end of stream the tail is flushed with idle pattern bytes (`0x69`, never zeros, so no DC step) and the output is cut to exactly `samples_per_channel / ratio` frames. Duration and position therefore match the file and the lyrics.
5. **Seek.** Target DSD sample = `round(t × R)`, aligned down to a whole byte and a multiple of the total ratio. The stream is positioned `PREROLL_BYTES` earlier (2048 bytes, about 5.8 ms at DSD64, far above the filters' settling time) and the pre-roll output is discarded; at file start the history is primed with idle pattern bytes. `try_seek` is bounded work (one block read plus pre-roll), so it fits the backend's seek deadline.
6. **Threading and I/O.** Decoding runs inline on the pull side like the other decoders. One 4096-byte block per channel (about 2048 output frames at DSD64) is converted per refill into preallocated buffers; `next()` does no allocation. The reader sits behind `DsdStream`, so a read-ahead wrapper can be added for slow media without touching the converter.

### Integration points

- `player-formats` gains a decode engine per format (`Rodio` or `Dsd`). `open_decoder` returns an `AudioDecoder` enum (rodio decoder or DSD source) chosen from the table; `PlaybackSource` gets a DSD variant. DSEE HX is never engaged for DSD.
- Windows library scanning adds `dsf` (and `dff` once supported) after the decoder exists. Track properties come from `DsdInfo` (rate, channels, duration, 1 bit); tags come from the trailing ID3v2 tag. Display labels are `DSD64/128/256/512`; every DSD file counts as Hi-Res.
- `ResamplingConversion` / `ResamplingInfo` gain a DSD description (for example "DSD64 → PCM 176.4 kHz") so the existing status display can show the path.
- No new user setting in the first version: target rate and gain are constants. A `dsdOutputMode` setting is introduced only when DoP exists, with a schema bump.

### Reserved for DoP (not built)

- **Same stream, second renderer.** `DopPacker` consumes `DsdStream` MSB-first bytes: 16 consecutive DSD bits per channel per frame, top byte alternating markers `0x05` / `0xFA`, at `R/16` (DSD64 176.4 kHz, DSD128 352.8 kHz, DSD256 705.6 kHz; DSD512 not offered).
- **Route planning.** A single function `plan_dsd_route(info, settings, device_caps) -> DsdRoute { Pcm { rate }, Dop { rate } }` decides per track. DoP is chosen only when the setting is on and the exclusive-mode endpoint accepts the exact format; otherwise it falls back to `Pcm` and reports why.
- **Output path.** `AudioBackend` is already the seam (the worker owns one implementation). DoP is a second backend (`WasapiExclusiveBackend`); a small routing backend would pick between it and `RodioBackend` per `load`. DoP ignores app volume (or the UI disables it), bypasses rubato and the mixer, and must keep the marker pattern continuous: while paused, seeking or between tracks it sends DSD idle pattern frames, never PCM silence.
- **Not designed now:** exclusive-mode device negotiation, hot-plug recovery for exclusive streams, and gapless DoP track changes.

### Verification

Synthetic DSD from a high-order sigma-delta modulator, written as DSF by the tests: 1 kHz tone amplitude and flatness (±0.02 dB to 20 kHz); alias rejection at `k·R/8 ± 1…20 kHz` of at least 100 dB; an impulse test for delay compensation; sequential versus seeked output equality over random positions; exact output frame count; DSD256 stereo conversion several times faster than real time. An `#[ignore]` smoke test reads a real file from `D:\Music\DSD`.

## Considered options

- **Use `rdsd2pcm` / `dsd2dxd`:** the most complete, but GPL-3.0, which forbids the noncommercial restriction. Used only as an offline reference for comparing frequency response, never linked or shipped.
- **Use FFmpeg's DSD decoder:** brings the whole FFmpeg distribution, licensing and build problem (see the earlier FFmpeg discussion) for a generic filter; not better than a purpose-built one.
- **Wrap `dsd-reader`:** reads containers only, converts nothing, very new. The DSF container is simple enough to own, and owning it gives the seek and normalisation contract above.
- **DoP first:** bit-perfect, but needs exclusive output, a DoP-capable DAC and a new backend; most listeners and the existing mixer path need PCM. Doing PCM first keeps every DAC working and leaves DoP as an addition.
- **Single large FIR instead of table plus halfbands:** simpler but much more work per sample at DSD256/512 and harder to retarget the output rate.

## Consequences

- DSD plays through the existing volume, resampling, DSEE-skip, statistics and system media paths with no changes outside the decoder layer.
- The first version fixes the target rate and gain; the filter quality targets above are testable requirements, not tuning preferences.
- Adding DoP later is additive: a renderer, a backend and a setting. The `DsdStream` contract (MSB-first planar bytes, idle pattern, exact sample positions) is what makes that possible and must not be bent for the PCM path.
- Multichannel DSD, DFF/DST and Android DSD are explicitly not covered. Android plays through Media3 and would need a separate software-decode path.
