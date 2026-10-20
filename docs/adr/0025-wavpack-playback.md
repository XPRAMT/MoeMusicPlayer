# WavPack playback: a vendored libwavpack decoder

Status: Accepted

Decision date: 2026-10-20, Asia/Taipei (UTC+08:00)

## Context

`.wv` files were already scanned into the library and their tags were readable (lofty), but nothing could play them: Symphonia has no WavPack codec, and the only Symphonia-style Rust decoder on crates.io (`symphonia-codec-wavpack`) is GPL-3.0, which this project must not use (PolyForm Noncommercial cannot carry a GPL dependency). `wavpack-sys` (BSD-3-Clause) needs cmake and bindgen/libclang, and the Android build has no cmake (the same reason libopus is not built there).

## Decision

- New crate `crates/player-wavpack` vendors the **decoder half of libwavpack 5.6.0** (BSD-3-Clause, `vendor/COPYING`, credited in `licenses/credits.json`): 13 C files, no encoder, no assembly, no legacy (< 4.0) support. `build.rs` compiles them with the `cc` crate, so Windows and Android build the same way and no tool beyond a C compiler is needed.
- A small hand-written FFI plus a safe `WavPackReader`: it opens any `Read + Seek` through the library's stream-reader callbacks (so Windows paths of any kind work), and a `.wvc` correction file next to a hybrid `.wv` is opened too (the result is then lossless). Output is interleaved `f32` in ±1.0: integers are divided by `2^(bits-1)` (exact up to 24 bits), floating point files are bit-exact (including NaN and infinity), DSD-in-WavPack files are decimated by the library to 24-bit PCM (rate ÷ 8). Seeking is sample exact; a seek to the last frame is answered by the wrapper because the library refuses it.
- `player-formats`: `DecodeEngine::WavPack` for `.wv`; the open-file dialog now offers `.wv` (Windows scanning and association already did). `player-audio-windows` adds `AudioDecoder::WavPack` over a rodio `Source`. The spectrogram decodes WavPack with the same reader.
- **Tags and covers.** lofty reads and writes the APEv2 tag of a `.wv`, but its generic `Tag` neither carries the `Cover Art (...)` items nor writes pictures back, so for WavPack and Monkey's Audio the details panel works on the file's own `ApeTag` (`player-tags/src/ape_cover.rs`, `write_ape_edit`): existing cover items survive an edit of another field, a replaced or removed cover drops the old items, and library artwork (`player-platform-windows/src/artwork.rs`) reads the cover from the same place. This also fixes covers of `.ape` files.

## Verification

Besides the reference encoder's small fixtures, 15 whole songs were encoded with the official `wavpack` 5.9.0 (lossless 16/24/32-bit integer and 32-bit float in several compression modes, hybrid lossy at 96–768 kbps, and lossy `.wv` + `.wvc` pairs) and decoded with the official `wvunpack`; the decoder equals `wvunpack` bit for bit in every case (and the original WAV for lossless files), seeks are exact, tags, covers of 0.6–1.1 MB, tag edits (audio unchanged, `.wvc` pairing intact), the library scan columns and shared/exclusive WASAPI playback all pass. `crates/player-wavpack/tests/make_official_test_files.py` rebuilds the set. ffmpeg's WavPack encoder stores 24-bit input in a 32-bit container, so it is not used for bit-depth checks.

## Consequences

- The decoder is the reference implementation, so every WavPack feature that matters for playback (lossless, hybrid with and without `.wvc`, float, 8–32 bit, multichannel, DSD) is covered; it is verified against the reference encoder's own files (`tests/reference`) and against local music round-tripped through ffmpeg at several compression levels (`tests/real_music.rs`, run by hand).
- Android plays WavPack through the stream bridge DSD uses (ADR 0016): the track is loaded as `moewv:///path`, the Kotlin data source reads it through JNI as a float WAV (`WavPackWavStream`, random access by byte position, the file's own sample rate, `.wvc` honoured) and Media3's WAV extractor and float output do the rest; library scanning lists `.wv`, bit depth comes from the decoder and covers from the APEv2 tag. Verified on a phone with 16/24-bit, float32 and hybrid files (playback, exact seeks, track change, 0 underruns). The notification cover of a `.wv` track is missing because the platform retriever cannot read APEv2 covers.
- DSD-in-WavPack is played through libwavpack's decimation, not our DSD converter (ADR 0016); this is rare and acceptable.
