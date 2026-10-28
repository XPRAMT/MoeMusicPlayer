//! The codec labels the shared reader writes into the library (`TrackMetadata::codec`), in one place. The database sorts
//! and groups by them case-insensitively (`audio_format_order_by`, `format_group_label` in `player-db`), the lists show
//! them upper-cased, and `track-columns.js` knows which are lossless; a new label must be added to all three.
//!
//! Container labels keep the capitalisation they have always had (lofty's `FileType` name), so rows already in a library
//! stay the same strings.

use lofty::file::FileType;

pub const MP1: &str = "MP1";
pub const MP2: &str = "MP2";
pub const MP3: &str = "MP3";
/// An MP4/M4A file whose audio is not ALAC (normally AAC); the name has always been the container's.
pub const MP4: &str = "Mp4";
/// Apple Lossless inside MP4/M4A: a lossless codec, told apart from AAC so the quality sort can rank it.
pub const ALAC: &str = "ALAC";
/// Matroska/WebM audio whose codec is none of the known ones.
pub const MATROSKA_AUDIO: &str = "Matroska audio";
pub const WVLL: &str = "WVLL";
pub const WVL: &str = "WVL";
pub const WVDSD: &str = "WVDSD";

/// Every label that is not derived from a container name or a DSD rate (`DSD64`, `DSD128`, ...).
pub const FIXED: &[&str] = &[MP1, MP2, MP3, MP4, ALAC, MATROSKA_AUDIO, WVLL, WVL, WVDSD];

/// The label of a container that has no finer one: lofty's name for it (`Flac`, `Wav`, `Aiff`, `Vorbis`, `Opus`, `Ape`,
/// `Aac`, `Speex`, `Mpc`, `Mp4`, `WavPack`).
pub fn container_label(file_type: FileType) -> String {
    format!("{file_type:?}")
}
