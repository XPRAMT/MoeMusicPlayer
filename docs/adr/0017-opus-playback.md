# Opus playback: our own Ogg demuxer over libopus

Status: Accepted

Decision date: 2026-10-17, Asia/Taipei (UTC+08:00)

## Context

Opus files (`.opus`, and `.ogg`/`.oga` files that hold an Opus stream) are scanned into the library and their tags can be read, but the player cannot decode them: rodio's Symphonia has no Opus decoder and Symphonia 0.6.1 still has none. Android already plays Ogg Opus through Media3, so this is a Windows decoder. The project is source-available under PolyForm Noncommercial (no GPL code); libopus is BSD-3-Clause, and the Rust `opus` crate (MIT OR Apache-2.0, built on `opusic-sys`) compiles libopus from source and links it statically on this machine without extra tools (no cmake, no DLL to ship).

## Decision

- New crate `crates/player-opus` (Windows playback only; not part of the Android build): an Ogg page/packet reader, an `OpusHead` parser, and `OpusReader`, which decodes with libopus to interleaved `f32` at 48 kHz. `player-audio-windows` wraps it in a rodio `Source` and adds an `AudioDecoder::Opus` variant, selected like DSD through the format table (`DecodeEngine::Opus`).
- **Own Ogg demuxer, not the `ogg` crate for reading.** At open, one pass over the page headers (27 bytes plus the segment table each, bodies skipped) builds an index of `(file offset, granule position, continued flag)` for the first logical stream. That index gives the exact duration (last granule minus pre-skip) and makes seeking exact: a seek target is mapped to the last page that is not a continuation and whose start position (the previous page's granule) is at least 200 ms before the target (RFC 7845 suggests 80 ms, but a pure tone still differed by about 3% from sequential decoding after 80 ms), decoding resumes there, and output before the target is dropped. The `ogg` crate stays a dev-dependency to write test files.
- **Mono and stereo only** (channel mapping family 0). Multichannel (family 1/255) is refused with a reason, like multichannel DSD.
- **Gain and trimming follow RFC 7845**: the header output gain is applied; the first `pre-skip` frames are dropped; output stops at the final granule position, so the length is exact. `R128_*` tags are not applied.
- **Ogg sniffing.** `.opus` goes to this decoder. `.ogg`/`.oga` files are opened with rodio unless their first packet is `OpusHead`, in which case they use this decoder too.
- **Tags.** Reading, scanning and writing go through lofty, which already supports Ogg Opus (Vorbis comments, embedded pictures). Opus is therefore editable in the details panel with the existing flow; a test writes a synthetic Opus file and edits it.
- Spectrogram: `player-tags` decodes Opus (and Ogg files whose first packet is `OpusHead`) with this reader instead of Symphonia; the rest of the spectrogram pipeline is shared.

## Considered options

- **`ogg` crate for reading:** its `seek_absgp` lands on the first page at or after a granule, and where a packet starts inside that page is not exposed, which makes exact positions awkward. Our index is smaller than the work to work around it.
- **Pure-Rust Opus decoders (`opus-pure`, `moosicbox_opus_native`):** no C build, but very young (weeks old, few downloads) and not conformance-tested here; libopus is the reference implementation.
- **FFmpeg:** same licensing, size and build objections as for DSD (ADR 0016 discussion).

## Consequences

- The Windows build compiles libopus (a one-time cost) and links it statically; the executable grows by the size of libopus. `opus` and libopus are added to the credits.
- Seeking needs the page index, built once when a file is opened (about a thousand page headers for a five-minute track).
- Chained Ogg streams play only their first link.
