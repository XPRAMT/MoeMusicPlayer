//! The codec labels the shared reader writes into the library (`TrackMetadata::codec`). A label names the audio CODEC,
//! never the container: the quality sort looks at the encoding only (the table that ranks them is
//! `player_formats::codec`, which also knows the spellings older rows carry).
//!
//! Every uncompressed PCM file (WAV, AIFF, AIFC with a PCM format, MKA PCM, IEEE float too) is `WAV`.

use lofty::file::FileType;

pub const MP1: &str = "MP1";
pub const MP2: &str = "MP2";
pub const MP3: &str = "MP3";
pub const AAC: &str = "AAC";
/// An MP4/M4A file whose sample entry the reader does not recognise (it ranks like AAC).
pub const MP4_UNKNOWN: &str = "MP4";
pub const ALAC: &str = "ALAC";
pub const FLAC: &str = "FLAC";
pub const OPUS: &str = "OPUS";
pub const VORBIS: &str = "VORBIS";
/// Uncompressed PCM or IEEE float in any container.
pub const WAV: &str = "WAV";
/// AIFC with a compressed format (µ-law, A-law, ...).
pub const AIFC: &str = "AIFC";
/// A WAV file with ADPCM or another compressed format.
pub const WAV_COMPRESSED: &str = "ADPCM";
pub const APE: &str = "APE";
pub const MATROSKA_AUDIO: &str = "Matroska audio";
pub const WVLL: &str = "WVLL";
pub const WVL: &str = "WVL";
pub const WVDSD: &str = "WVDSD";

/// Every label that is not a DSD rate (`DSD64`, `DSD128`, ...) or an upper-cased container name without a finer one.
pub const FIXED: &[&str] = &[
    MP1,
    MP2,
    MP3,
    AAC,
    MP4_UNKNOWN,
    ALAC,
    FLAC,
    OPUS,
    VORBIS,
    WAV,
    AIFC,
    WAV_COMPRESSED,
    APE,
    MATROSKA_AUDIO,
    WVLL,
    WVL,
    WVDSD,
];

/// The label of a container that holds one codec: FLAC, Ogg Vorbis/Opus/Speex, ADTS AAC, Monkey's Audio, Musepack.
/// Containers that can hold several (MP4, WAV, AIFF, Matroska, WavPack) are labelled by their own readers.
pub fn container_label(file_type: FileType) -> String {
    match file_type {
        FileType::Flac => FLAC.to_owned(),
        FileType::Vorbis => VORBIS.to_owned(),
        FileType::Opus => OPUS.to_owned(),
        FileType::Aac => AAC.to_owned(),
        FileType::Ape => APE.to_owned(),
        other => format!("{other:?}").to_ascii_uppercase(),
    }
}

/// The label of a WAV file by its `fmt` format tag (1 PCM, 3 IEEE float, 0x50/0x55 MPEG audio, else compressed).
pub fn wav_label(format_tag: u16) -> &'static str {
    match format_tag {
        1 | 3 => WAV,
        0x0050 | 0x0055 => MP3,
        _ => WAV_COMPRESSED,
    }
}
