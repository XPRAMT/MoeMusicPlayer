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

## Codec labels in the list

The list column would be too wide with `WavPack`, so the codec written by the scanner is `WVLL` (lossless, also a hybrid `.wv` with its `.wvc`), `WVL` (lossy hybrid) or `WVDSD` (DSD inside WavPack, lossless only; the sample rate is the 1-bit rate and the depth 1, like DSF). The decoder decides (`player_wavpack::summarize`); quality sorting (the owner's order, best first) is DSD, WVDSD, WAV, WVLL, FLAC, (the other formats as before), WVL, MP3. DSD-in-WavPack is read as its native 1-bit stream (`OPEN_DSD_NATIVE`, `WavPackDsdStream`, a `DsdStream` in the same MSB-first convention as DSF) and goes through the project's own DSD→PCM converter (ADR 0016) with the same gain and output-rate settings as a DSF file, on Windows and on Android (176.4 kHz). libwavpack's own 24-bit decimation (rate ÷ 8) is not used for playback. DoP (Windows exclusive mode, ADR 0020 P2) works for it too: the DoP source takes any `DsdStream`, and the DoP words equal the DSF's bit for bit (checked over a whole DSD256 song).

## Verification

Besides the reference encoder's small fixtures, 15 whole songs were encoded with the official `wavpack` 5.9.0 (lossless 16/24/32-bit integer and 32-bit float in several compression modes, hybrid lossy at 96–768 kbps, and lossy `.wv` + `.wvc` pairs) and decoded with the official `wvunpack`; the decoder equals `wvunpack` bit for bit in every case (and the original WAV for lossless files), seeks are exact, tags, covers of 0.6–1.1 MB, tag edits (audio unchanged, `.wvc` pairing intact), the library scan columns and shared/exclusive WASAPI playback all pass. `crates/player-wavpack/tests/make_official_test_files.py` rebuilds the set. ffmpeg's WavPack encoder stores 24-bit input in a 32-bit container, so it is not used for bit-depth checks.

## Consequences

- The decoder is the reference implementation, so every WavPack feature that matters for playback (lossless, hybrid with and without `.wvc`, float, 8–32 bit, multichannel, DSD) is covered; it is verified against the reference encoder's own files (`tests/reference`) and against local music round-tripped through ffmpeg at several compression levels (`tests/real_music.rs`, run by hand).
- Android plays WavPack through the stream bridge DSD uses (ADR 0016): the track is loaded as `moewv:///path`, the Kotlin data source reads it through JNI as a float WAV (`WavPackWavStream`, random access by byte position, the file's own sample rate, `.wvc` honoured) and Media3's WAV extractor and float output do the rest; library scanning lists `.wv`, bit depth comes from the decoder and covers from the APEv2 tag. Verified on a phone with 16/24-bit, float32 and hybrid files (playback, exact seeks, track change, 0 underruns). Android's `MediaMetadataRetriever` only understands ID3, Vorbis and MP4 covers, so the cover of a `.wv` (an APEv2 item) is read by Rust and handed to the notification code with the load command (`artworkDataBase64`).
- DSD-in-WavPack is played through libwavpack's decimation, not our DSD converter (ADR 0016); this is rare and acceptable.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

WavPack（ADR 0025）：`crates/player-wavpack` 包 libwavpack 5.6.0 解碼端（BSD-3-Clause，`vendor/`，`build.rs` 用 `cc`），安全的 `WavPackReader`（自動使用 `.wvc`，輸出 ±1.0 f32，seek 精確）；Windows `player-audio-windows/src/wavpack.rs`、頻譜圖共用。編解碼短名（使用者定案）：`WVLL`＝無損（含 `.wv`＋`.wvc`）、`WVL`＝有損 hybrid、`WVDSD`＝DSD-in-WavPack（列表顯示 MHz、不顯示位元深度、一律 Hi-Res），由 `player_wavpack::summarize` 決定（Windows `read_wavpack_summary`、Android `mobile.rs`）；詳情「格式」欄顯示 `WavPack DSD`／`WavPack Lossless`／`WavPack Loss`（`wavpack_format_name`），WVDSD 屬性以 1-bit 呈現（`fill_wavpack_dsd_properties`）。品質排序（`audio_format_order_by`，由高到低）：DSD、WVDSD、WAV、WVLL、FLAC、WVL、Vorbis、Opus、MP4、MP3；前端 `LOSSLESS_CODECS` 含 `WVLL`／`WVDSD`、`isDsdCodec` 含 `WVDSD`；舊名 `WV`／`WAVPACK` 仍被辨識。WVDSD 走專案的 DSD 轉換器（`player-wavpack/src/dsd.rs` 的 `WavPackDsdStream` → `DsdPcmReader`；Windows `dsd::open_wavpack_dsd`、Android `Served::DsdWavPack` 固定 176.4 kHz），也支援 DoP。WavPack／Monkey's Audio 的標籤與封面直接編輯檔案的 `ApeTag`（`player-tags/src/ape_cover.rs`、`write_ape_edit`），專輯封面也從 APEv2 讀（`artwork.rs`、Android `find_android_artwork`），內嵌歌詞讀 APEv2 `Lyrics`；Android 通知封面由 Rust `ape_cover_base64` 以 `artworkDataBase64` 交給 `PlaybackService.loadArtwork`（≤ 2 MiB，也適用 `.ape`）。驗證：`tests/reference_files.rs`（參考編碼器檔逐位元比對）、`make_official_test_files.py` 產生的官方編碼檔 ignored 測試（`MOE_WV_MANIFEST`、`MOE_WV_DSD`／`MOE_WV_DSF`、`MOE_WV_DOP=1`）；位元深度一律以官方編碼器檔驗證（ffmpeg 會把 24-bit 存成 32-bit 容器）；`wvtag` 寫 UTF-8 檔時要加 `--no-utf8-convert`。Android 手機上的 WavPack 歌詞與 Release GUI 實機播放待確認。
