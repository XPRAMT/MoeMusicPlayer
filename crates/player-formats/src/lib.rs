//! Shared "format capability" table.
//!
//! Every place that used to keep its own list of audio file extensions (library scanners, the open-file dialog,
//! tag writing, MIME mapping, spectrogram availability) asks this crate instead, so adding a format is one edit
//! here. The table describes *what the player does today*; each flag mirrors the behaviour that was previously
//! hard-coded, and the tests below pin the resulting sets.
//!
//! The crate has no dependencies so every platform crate, including the Android plugin, can use it.

use std::path::Path;

/// A container/codec family the player knows by name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FormatId {
    Mp3,
    Aac,
    Alac,
    Mp4,
    Flac,
    Wav,
    Aiff,
    Ogg,
    Opus,
    Speex,
    Matroska,
    Ape,
    WavPack,
    Mpc,
    Dsf,
    Dff,
    Ac3,
    Amr,
    Midi,
    Wma,
    Caf,
}

/// Which decoder plays a format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeEngine {
    /// rodio with Symphonia (every format that is not listed below; a format it cannot decode fails when opened).
    Rodio,
    /// The DSD→PCM converter (ADR 0016).
    Dsd,
    /// libopus over our Ogg reader (ADR 0017). `.ogg`/`.oga` files are routed here only when their first packet is
    /// `OpusHead` (the caller sniffs the content).
    Opus,
    /// libwavpack (ADR 0025).
    WavPack,
}

/// What the player can do with a format.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FormatInfo {
    pub id: FormatId,
    /// Short uppercase name shown in the UI.
    pub label: &'static str,
    /// Tags can be written back to the file (DSF/DFF are read-only, ADR 0013 D10).
    pub tag_write: bool,
    /// False only where the spectrogram decoder is known not to handle the format.
    pub spectrogram: bool,
    /// MIME types that map to this format (Android content resolver).
    pub mime_types: &'static [&'static str],
}

impl FormatInfo {
    pub fn decode_engine(&self) -> DecodeEngine {
        match self.id {
            FormatId::Dsf => DecodeEngine::Dsd,
            FormatId::Opus => DecodeEngine::Opus,
            FormatId::WavPack => DecodeEngine::WavPack,
            _ => DecodeEngine::Rodio,
        }
    }
}

const fn info(
    id: FormatId,
    label: &'static str,
    tag_write: bool,
    spectrogram: bool,
    mime_types: &'static [&'static str],
) -> FormatInfo {
    FormatInfo {
        id,
        label,
        tag_write,
        spectrogram,
        mime_types,
    }
}

const FORMATS: &[FormatInfo] = &[
    info(
        FormatId::Mp3,
        "MP3",
        true,
        true,
        &["audio/mpeg", "audio/mp3"],
    ),
    info(FormatId::Aac, "AAC", true, true, &[]),
    info(FormatId::Alac, "ALAC", true, true, &[]),
    info(
        FormatId::Mp4,
        "M4A",
        true,
        true,
        &[
            "audio/mp4",
            "audio/x-m4a",
            "audio/m4a",
            "audio/aac",
            "audio/mp4a-latm",
        ],
    ),
    info(
        FormatId::Flac,
        "FLAC",
        true,
        true,
        &["audio/flac", "audio/x-flac"],
    ),
    info(
        FormatId::Wav,
        "WAV",
        true,
        true,
        &["audio/wav", "audio/x-wav", "audio/vnd.wave", "audio/wave"],
    ),
    info(
        FormatId::Aiff,
        "AIFF",
        true,
        true,
        &["audio/aiff", "audio/x-aiff"],
    ),
    info(
        FormatId::Ogg,
        "OGG",
        true,
        true,
        &["audio/ogg", "application/ogg", "audio/vorbis"],
    ),
    info(FormatId::Opus, "OPUS", true, true, &["audio/opus"]),
    info(FormatId::Speex, "SPEEX", true, true, &[]),
    info(FormatId::Matroska, "MKA", true, true, &[]),
    info(FormatId::Ape, "APE", true, true, &[]),
    info(FormatId::WavPack, "WV", true, true, &[]),
    info(FormatId::Mpc, "MPC", true, true, &[]),
    info(
        FormatId::Dsf,
        "DSF",
        true,
        false,
        &["audio/x-dsf", "audio/dsf"],
    ),
    info(FormatId::Dff, "DFF", false, false, &[]),
    info(FormatId::Ac3, "AC3", true, true, &[]),
    info(FormatId::Amr, "AMR", true, true, &[]),
    info(FormatId::Midi, "MIDI", true, true, &[]),
    info(FormatId::Wma, "WMA", true, true, &[]),
    info(FormatId::Caf, "CAF", true, true, &[]),
];

/// One file extension and where it is accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExtensionEntry {
    extension: &'static str,
    format: FormatId,
    /// Windows library scanner and file association.
    windows_library: bool,
    /// Android library scanner.
    android_library: bool,
    /// Windows "open audio file" dialog filter.
    open_dialog: bool,
    /// Video container: never scanned into the library. `mp4` is also an Android content-provider candidate.
    video: bool,
}

const fn ext(
    extension: &'static str,
    format: FormatId,
    windows_library: bool,
    android_library: bool,
    open_dialog: bool,
) -> ExtensionEntry {
    ExtensionEntry {
        extension,
        format,
        windows_library,
        android_library,
        open_dialog,
        video: false,
    }
}

const fn video(extension: &'static str, format: FormatId, open_dialog: bool) -> ExtensionEntry {
    ExtensionEntry {
        extension,
        format,
        windows_library: false,
        android_library: false,
        open_dialog,
        video: true,
    }
}

use FormatId::*;

//                 extension  format    windows android dialog
const EXTENSIONS: &[ExtensionEntry] = &[
    ext("mp3", Mp3, true, true, true),
    ext("mp2", Mp3, true, false, false),
    ext("mp1", Mp3, true, false, false),
    ext("aac", Aac, true, true, true),
    ext("alac", Alac, false, true, true),
    ext("m4a", Mp4, true, true, true),
    ext("m4b", Mp4, true, true, false),
    ext("m4p", Mp4, true, false, false),
    ext("m4r", Mp4, true, false, false),
    ext("m4v", Mp4, true, false, false),
    ext("3gp", Mp4, true, false, false),
    video("mp4", Mp4, true),
    ext("flac", Flac, true, true, true),
    ext("wav", Wav, true, true, true),
    ext("wave", Wav, false, true, true),
    ext("aiff", Aiff, true, true, true),
    ext("aif", Aiff, true, true, true),
    ext("afc", Aiff, true, false, false),
    ext("aifc", Aiff, true, false, false),
    ext("ogg", Ogg, true, true, true),
    ext("oga", Ogg, true, true, true),
    ext("opus", Opus, true, true, false),
    ext("spx", Speex, true, true, false),
    ext("mka", Matroska, true, true, true),
    video("mkv", Matroska, true),
    video("webm", Matroska, true),
    ext("ape", Ape, true, true, false),
    ext("wv", WavPack, true, true, true),
    ext("mpc", Mpc, true, false, false),
    ext("mp+", Mpc, true, false, false),
    ext("mpp", Mpc, true, false, false),
    ext("dsf", Dsf, true, true, true),
    ext("dff", Dff, false, true, false),
    ext("ac3", Ac3, false, true, false),
    ext("amr", Amr, false, true, false),
    ext("mid", Midi, false, true, false),
    ext("midi", Midi, false, true, false),
    ext("wma", Wma, false, true, false),
    ext("caf", Caf, false, false, true),
];

fn extension_of(path: &Path) -> Option<&str> {
    path.extension().and_then(|extension| extension.to_str())
}

fn entry_for(extension: &str) -> Option<&'static ExtensionEntry> {
    EXTENSIONS
        .iter()
        .find(|entry| entry.extension.eq_ignore_ascii_case(extension))
}

/// Capabilities for a format.
pub fn format_info(id: FormatId) -> &'static FormatInfo {
    FORMATS
        .iter()
        .find(|format| format.id == id)
        .expect("every FormatId has a FORMATS entry")
}

/// Format of a file extension (without the dot), case-insensitive.
pub fn format_of_extension(extension: &str) -> Option<&'static FormatInfo> {
    entry_for(extension).map(|entry| format_info(entry.format))
}

/// Format of a path by its extension.
pub fn format_of_path(path: &Path) -> Option<&'static FormatInfo> {
    extension_of(path).and_then(format_of_extension)
}

/// Accepted by the Windows library scanner (video containers are excluded).
pub fn is_windows_library_audio(path: &Path) -> bool {
    extension_of(path)
        .and_then(entry_for)
        .is_some_and(|entry| entry.windows_library)
}

/// Accepted by the Android library scanner.
pub fn is_android_library_audio(path: &Path) -> bool {
    extension_of(path)
        .and_then(entry_for)
        .is_some_and(|entry| entry.android_library)
}

/// `.mp4` files are treated as video and kept out of the library.
pub fn is_video_mp4(path: &Path) -> bool {
    extension_of(path).is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
}

/// Extensions the Windows library accepts, with a leading dot (for file association keys).
pub fn windows_library_extensions() -> Vec<String> {
    EXTENSIONS
        .iter()
        .filter(|entry| entry.windows_library)
        .map(|entry| format!(".{}", entry.extension))
        .collect()
}

/// Extensions of the Android library scanner (no dot).
pub fn android_library_extensions() -> Vec<&'static str> {
    EXTENSIONS
        .iter()
        .filter(|entry| entry.android_library)
        .map(|entry| entry.extension)
        .collect()
}

/// Extensions Android's content-provider check treats as audio: the library set plus `mp4`.
pub fn android_candidate_extensions() -> Vec<&'static str> {
    EXTENSIONS
        .iter()
        .filter(|entry| entry.android_library || (entry.video && entry.extension == "mp4"))
        .map(|entry| entry.extension)
        .collect()
}

/// Extensions offered by the Windows "open audio file" dialog (no dot).
pub fn open_dialog_extensions() -> Vec<&'static str> {
    EXTENSIONS
        .iter()
        .filter(|entry| entry.open_dialog)
        .map(|entry| entry.extension)
        .collect()
}

/// Whether tags can be written back to this file. Unknown extensions are treated as writable (the tag library
/// decides), matching the previous behaviour where only DSF/DFF were refused.
pub fn tag_write_supported(path: &Path) -> bool {
    format_of_path(path).is_none_or(|format| format.tag_write)
}

/// Whether the spectrogram decoder is known to handle this file. Unknown extensions are tried anyway.
pub fn spectrogram_supported(path: &Path) -> bool {
    format_of_path(path).is_none_or(|format| format.spectrogram)
}

/// The decoder that plays this file. Unknown extensions go to rodio, which sniffs the content.
pub fn decode_engine(path: &Path) -> DecodeEngine {
    format_of_path(path).map_or(DecodeEngine::Rodio, FormatInfo::decode_engine)
}

/// File extension for a MIME type (no dot), when the format is known.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    let mime = mime.to_ascii_lowercase();
    let format = FORMATS
        .iter()
        .find(|format| format.mime_types.iter().any(|known| *known == mime))?;
    // The canonical extension is the first table entry of the format.
    EXTENSIONS
        .iter()
        .find(|entry| entry.format == format.id)
        .map(|entry| entry.extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(
        values: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> std::collections::BTreeSet<String> {
        values
            .into_iter()
            .map(|value| value.as_ref().to_owned())
            .collect()
    }

    #[test]
    fn every_format_has_info_and_every_extension_a_format() {
        for entry in EXTENSIONS {
            assert_eq!(format_info(entry.format).id, entry.format);
        }
        for format in FORMATS {
            assert!(
                EXTENSIONS.iter().any(|entry| entry.format == format.id),
                "{:?}",
                format.id
            );
        }
    }

    #[test]
    fn extensions_are_unique_and_lowercase() {
        let mut seen = std::collections::BTreeSet::new();
        for entry in EXTENSIONS {
            assert_eq!(entry.extension, entry.extension.to_ascii_lowercase());
            assert!(seen.insert(entry.extension), "{}", entry.extension);
        }
    }

    /// Previously lofty's `EXTENSIONS` plus `mka`/`oga`, minus `mp4`; `dsf` since the DSD converter (ADR 0016).
    #[test]
    fn windows_library_set_is_unchanged() {
        let expected = [
            "aac", "ape", "aiff", "aif", "afc", "aifc", "mp3", "mp2", "mp1", "wav", "wv", "opus",
            "flac", "ogg", "m4a", "m4b", "m4p", "m4r", "m4v", "3gp", "mpc", "mp+", "mpp", "spx",
            "mka", "oga", "dsf",
        ]
        .map(|extension| format!(".{extension}"));
        assert_eq!(set(windows_library_extensions()), set(expected));
    }

    /// Previously `fs_scan::AUDIO_EXTENSIONS`, plus `wv` since WavPack playback on Android (ADR 0025).
    #[test]
    fn android_library_set_is_unchanged() {
        let expected = [
            "aac", "ac3", "aif", "aiff", "alac", "amr", "ape", "dff", "dsf", "flac", "m4a", "m4b",
            "mid", "midi", "mka", "mp3", "oga", "ogg", "opus", "spx", "wav", "wave", "wma", "wv",
        ];
        assert_eq!(set(android_library_extensions()), set(expected));
    }

    /// Previously the Kotlin `AUDIO_EXTENSIONS` (library set plus `mp4`).
    #[test]
    fn android_candidate_set_is_unchanged() {
        let mut expected = set(android_library_extensions());
        expected.insert("mp4".to_owned());
        assert_eq!(set(android_candidate_extensions()), expected);
    }

    /// Previously the literal list in the open-file dialog.
    #[test]
    fn open_dialog_set_is_unchanged() {
        let expected = [
            "mp3", "flac", "m4a", "mp4", "aac", "alac", "ogg", "oga", "wav", "wave", "aif", "aiff",
            "mka", "mkv", "webm", "caf", "dsf", "wv",
        ];
        assert_eq!(set(open_dialog_extensions()), set(expected));
    }

    #[test]
    fn mp4_is_video_and_not_a_windows_library_file() {
        assert!(is_video_mp4(Path::new("clip.MP4")));
        assert!(!is_windows_library_audio(Path::new("clip.mp4")));
        assert!(is_windows_library_audio(Path::new("song.M4A")));
        assert!(!is_windows_library_audio(Path::new("movie.mkv")));
    }

    #[test]
    fn dsf_is_writable_dff_is_read_only_and_neither_has_a_spectrogram() {
        assert!(tag_write_supported(Path::new("a.dsf")));
        assert!(!tag_write_supported(Path::new("a.DFF")));
        for name in ["a.dsf", "a.DFF"] {
            assert!(!spectrogram_supported(Path::new(name)), "{name}");
        }
        assert!(tag_write_supported(Path::new("a.flac")));
        assert!(tag_write_supported(Path::new("a.unknown")));
        assert!(spectrogram_supported(Path::new("a.opus")));
        assert!(spectrogram_supported(Path::new("a.mp3")));
    }

    #[test]
    fn only_dsf_and_opus_use_their_own_decoders() {
        assert_eq!(decode_engine(Path::new("a.DSF")), DecodeEngine::Dsd);
        assert_eq!(decode_engine(Path::new("a.dff")), DecodeEngine::Rodio);
        assert_eq!(decode_engine(Path::new("a.OPUS")), DecodeEngine::Opus);
        assert_eq!(decode_engine(Path::new("a.ogg")), DecodeEngine::Rodio);
        assert_eq!(decode_engine(Path::new("a.flac")), DecodeEngine::Rodio);
        assert_eq!(decode_engine(Path::new("a.unknown")), DecodeEngine::Rodio);
    }

    #[test]
    fn mime_maps_to_the_canonical_extension() {
        assert_eq!(extension_for_mime("audio/mpeg"), Some("mp3"));
        assert_eq!(extension_for_mime("Audio/X-M4A"), Some("m4a"));
        assert_eq!(extension_for_mime("audio/x-wav"), Some("wav"));
        assert_eq!(extension_for_mime("audio/x-dsf"), Some("dsf"));
        assert_eq!(extension_for_mime("audio/opus"), Some("opus"));
        assert_eq!(extension_for_mime("audio/x-unknown"), None);
    }
}
