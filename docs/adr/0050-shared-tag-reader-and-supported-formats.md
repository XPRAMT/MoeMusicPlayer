# 0050 One tag reader, codec names, and the formats the library scans

Status: Accepted and implemented (unit-tested; nothing built into an APK or run on a device for this change).

## One reader

`player-track-metadata::read_track_metadata` is the only reader of a track's tags and properties. Windows scans, remote scans (ADR 0026), the refresh while a song plays (ADR 0040) and now the Android local scan (`mobile.rs`, through `PathSource`, or `NamedPathSource` for the private copy of a `content://` document, named after its MIME type) all call it. Symphonia and Android's `MediaMetadataRetriever` remain only as fallbacks for a file this reader cannot parse. The matrix test `crates/player-track-metadata/tests/matrix.rs` reads each container as a path, a memory stream (a remote file) and a renamed copy and requires the same answer.

Field rules (one per field, every container): a blank title is none; the track and disc number is the number before a `/`; the date is the tag's text as written when it can be read without lofty's timestamp conversion (ID3v2 `TDRC`, also for DSF), so `2024-2-9` keeps its day; every value of a multi-value artist or album-artist field is kept, blank and repeated ones dropped, joined with `" | "` (`MULTI_VALUE_JOINER`; the artist splitter of `player-db` and `splitArtists` split on `|`, and it is the separator least likely to be part of a name); a single value is stored untouched. A DSF scan walks the trailing ID3v2 frames and skips the `APIC` picture with a seek (`player_dsd::id3::read_dsf_id3_bytes(reader, false)`).

## The codec column names the codec

`codec` is the audio codec, never the container, because the quality sort looks at the encoding only. The table is `player_formats::codec::CODECS` (label, spellings older rows carry, rank, lossless, quick-index group); the reader writes its labels (`player-track-metadata/src/labels.rs`), `player-db` builds the sort `CASE`, the bit-depth list and `format_group_label` from it, and `tests/fixtures/codec-table.json` (written by the table's test) is what `tests/codec-table.test.mjs` compares the front end's lossless list with.

Order, high to low: DSD 7, WVDSD 6.5, WAV 6, WVLL 5.5, FLAC 5, ALAC 4.8, APE 4.7, WVL 4.5, Vorbis 4, Opus 3, AAC 2 (the MP4 group), MP3 1. Every uncompressed PCM file is one class labelled `WAV`: WAV, AIFF/AIFC with a PCM format, MKA PCM, IEEE float too; there is no AIFF rank or group. Resolution by container:

| Container | Codec comes from | Cost |
| --- | --- | --- |
| MP3 | MPEG layer | lofty, already parsed |
| MP4/M4A | the sample entry (`mp4a` AAC, `alac`, `fLaC`, MP3); anything else `MP4` (ranks as AAC) | lofty `Mp4File` properties |
| WAV | the `fmt ` format tag (1/3 PCM and float, 0x50/0x55 MP3, else `ADPCM`) | the `fmt ` chunk walk that already read the bit depth |
| AIFF | the COMM compression type (PCM family is `WAV`, otherwise `AIFC`) | lofty `AiffFile` properties |
| FLAC, Vorbis, Opus, ADTS AAC, APE | the container is the codec | none |
| Ogg | lofty's `FileType` (Vorbis, Opus); Ogg FLAC and Speex are not resolved | none |
| Matroska | Symphonia track codec (PCM family is `WAV`); unknown ones stay `Matroska audio` | already parsed |
| WavPack, DSF | `WVLL`/`WVL`/`WVDSD`, `DSD64`... | unchanged |

An ALAC file whose `stsd` is not the exact layout lofty expects reads as `MP4` with no bit depth; no heavy parse was added for it.

The source bit depth is reported for FLAC, APE, ALAC, WAV and AIFF PCM, WavPack and DSD only. Float32 (WAV tag 3, WavPack float, AIFF `fl32`) reports 32 like 32-bit integer: the column cannot tell them apart, so a USB exclusive stream (`bits_per_sample: track.bit_depth`) would treat float as integer. Not fixed (needs a column); listed for a later change.

## What the library scans

A format is scanned only when its tags can be read and it can be played. The evidence is the resolved feature set of the Windows build (`cargo tree -e features`: Symphonia has the RIFF `wav` feature only, `mp3` only without `mp1`/`mp2`, no APE, Musepack, Speex or CAF demuxer) and, for Android, Media3's built-in extractors and the converters of `player_formats::android_converter` (ADR 0043). Removed from the scan sets: `ape`, `spx`, `mpc`/`mp+`/`mpp` (no decoder on either platform), Android `dff` (DSDIFF is not implemented, ADR 0016), `wma`, `mid`/`midi` (no Media3 extractor); `caf` leaves the open dialog. Rows already scanned disappear on the next complete scan through the missing-file mechanism (ADR 0033).

Kept although not proven: `ac3` and `amr` on Android (Media3 has extractors, playback depends on the device's decoder); `aiff`/`aif`/`aifc`/`afc` and `mp1`/`mp2` on Windows are tag-readable but their Symphonia features are not enabled, so playback fails there until `symphonia-aiff`, `symphonia-mp1`, `symphonia-mp2` are added to the `rodio` features of `player-audio-windows` (decision for the owner); Android `alac` (raw `.alac`) and the MP4 variants `m4p`/`m4r`/`m4v`/`3gp`.

`tag_write` in the table now means lofty or our own code can write the container: Matroska, AC3, AMR, MIDI, WMA and CAF are read-only. The details dialog shows an MKA file read-only through the shared reader; its bit depth and date also come from the shared reader (the library and the dialog agree).

## Embedded lyrics

`player_tags::embedded::read_embedded_lyric_texts` (over `read_embedded_tags`, which converts a DSF's ID3v2 and an APE-family tag to lofty `Tag`s) is the only reader, for Windows and Android, local and remote. Lyrics the details panel writes into a DSF are now read at playback.

## Re-read

`TRACK_METADATA_VERSION` is 8: every track is re-read in the background once, which fixes old `Mp4` ALAC rows, AIFF rows, WAV bit depths of Android-scanned files and truncated dates.
