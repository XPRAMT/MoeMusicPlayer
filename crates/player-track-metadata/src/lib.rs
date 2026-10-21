//! Reading a track's tags and audio properties from any byte source (ADR 0026). The Windows scanner reads local files
//! through [`PathSource`]; remote sources hand in their own [`ByteSource`]; both get exactly the same parsing.

use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use lofty::{
    config::ParseOptions,
    file::{FileType, TaggedFile},
    mpeg::{Layer, MpegFile},
    prelude::{Accessor, AudioFile, TaggedFileExt},
    probe::Probe,
    tag::ItemKey,
};
use player_core::{TrackMetadata, TrackMetadataError};
use symphonia::{
    core::{formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint},
    default::get_probe,
};

pub use player_wavpack::ReadSeek;

/// Where a track's bytes come from. Every call to [`open`](Self::open) returns a fresh reader at the start, so the
/// parser can read the file more than once (a tag frame, then the audio properties).
pub trait ByteSource {
    /// The file name, only for its extension.
    fn file_name(&self) -> String;

    fn open(&self) -> io::Result<Box<dyn ReadSeek>>;

    /// The `.wvc` correction file of a WavPack file, when there is one.
    fn open_correction(&self) -> Option<Box<dyn ReadSeek>> {
        None
    }
}

/// A file on a local disk.
pub struct PathSource<'a>(pub &'a Path);

impl ByteSource for PathSource<'_> {
    fn file_name(&self) -> String {
        self.0
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        Ok(Box::new(std::fs::File::open(self.0)?))
    }

    fn open_correction(&self) -> Option<Box<dyn ReadSeek>> {
        let mut name = self.0.as_os_str().to_owned();
        name.push("c");
        std::fs::File::open(std::path::PathBuf::from(name))
            .ok()
            .map(|file| Box::new(BufReader::new(file)) as Box<dyn ReadSeek>)
    }
}

/// What a library scan parses: everything but the embedded pictures.
fn scan_parse_options() -> ParseOptions {
    ParseOptions::new().read_cover_art(false)
}

/// Tags and audio properties of one track.
pub fn read_track_metadata(source: &dyn ByteSource) -> Result<TrackMetadata, TrackMetadataError> {
    let name = source.file_name();
    let name_path = Path::new(&name);
    if name_path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mka"))
    {
        return read_matroska_audio_metadata(source);
    }
    if player_formats::decode_engine(name_path) == player_formats::DecodeEngine::Dsd {
        let reader = source.open().map_err(|error| TrackMetadataError {
            message: format!("could not open DSF file: {error}"),
        })?;
        return read_dsf_audio_metadata_from(reader);
    }

    let reader = source.open().map_err(|error| TrackMetadataError {
        message: format!("could not open audio metadata: {error}"),
    })?;
    // `Probe::open` starts from the extension's file type; keep that behaviour for a stream.
    let mut probe = Probe::new(BufReader::new(reader));
    if let Some(file_type) = FileType::from_path(name_path) {
        probe = probe.set_file_type(file_type);
    }
    // A scan needs the tags and the audio properties, never the pictures: a cover is megabytes, and over a network it is
    // megabytes that cross the link for nothing (the artwork is read on its own when it is shown).
    let probe = probe
        .options(scan_parse_options())
        .guess_file_type()
        .map_err(|error| TrackMetadataError {
            message: format!("could not identify audio file type: {error}"),
        })?;
    let file_type = probe.file_type().ok_or_else(|| TrackMetadataError {
        message: "could not identify audio file type".to_owned(),
    })?;
    let raw_id3_recording_date = if file_type == FileType::Mpeg {
        source
            .open()
            .ok()
            .and_then(|reader| read_id3v2_text_frame(reader, b"TDRC").ok().flatten())
    } else {
        None
    };
    let (tagged_file, codec) = if file_type == FileType::Mpeg {
        let mut reader = probe.into_inner();
        reader
            .seek(std::io::SeekFrom::Start(0))
            .map_err(|error| TrackMetadataError {
                message: format!("could not rewind MPEG audio file: {error}"),
            })?;
        let mpeg_file = MpegFile::read_from(&mut reader, scan_parse_options()).map_err(|error| {
            TrackMetadataError {
                message: format!("could not parse MPEG audio properties: {error}"),
            }
        })?;
        let codec = mpeg_codec_name(*mpeg_file.properties().layer()).to_owned();
        (TaggedFile::from(mpeg_file), codec)
    } else {
        let tagged_file = probe.read().map_err(|error| TrackMetadataError {
            message: format!("could not parse audio metadata: {error}"),
        })?;
        (tagged_file, format!("{file_type:?}"))
    };
    let wavpack = if file_type == FileType::WavPack {
        source
            .open()
            .ok()
            .and_then(|main| player_wavpack::summarize_from(main, source.open_correction()))
    } else {
        None
    };
    let codec = wavpack.map_or(codec, |summary| summary.codec.to_owned());
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());
    let properties = tagged_file.properties();
    let text = |value: Option<std::borrow::Cow<'_, str>>| value.map(|value| value.into_owned());
    let release_date = match raw_id3_recording_date.as_deref() {
        Some(value) => player_core::parse_recording_date(value),
        None => tag
            .and_then(|tag| tag.get_string(ItemKey::RecordingDate))
            .and_then(player_core::parse_recording_date),
    };
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year);

    Ok(TrackMetadata {
        title: tag
            .and_then(|tag| text(tag.title()))
            .filter(|value| !value.trim().is_empty()),
        artist: tag.and_then(|tag| text(tag.artist())),
        album: tag.and_then(|tag| text(tag.album())),
        album_artist: tag.and_then(|tag| tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)),
        track_number: tag.and_then(|tag| tag.track()),
        disc_number: tag.and_then(|tag| tag.disk()),
        duration_ms: Some(properties.duration().as_millis().min(u64::MAX as u128) as u64),
        codec: Some(codec),
        bitrate_bps: properties
            .audio_bitrate()
            .map(|kbps| kbps.saturating_mul(1000)),
        sample_rate_hz: wavpack
            .map(|summary| summary.sample_rate_hz)
            .or_else(|| properties.sample_rate()),
        release_date,
        year,
        embedded_lyrics: embedded_lyrics_text(tagged_file.tags()),
        bit_depth: if file_type == FileType::Wav {
            source
                .open()
                .ok()
                .and_then(read_lossless_wav_bit_depth_from)
        } else if file_type == FileType::WavPack {
            wavpack.and_then(|summary| summary.bit_depth)
        } else {
            reliable_source_bit_depth(file_type, properties.bit_depth())
        },
    })
}

/// Read an ID3v2 text frame without Lofty's semantic timestamp conversion. Lofty's timestamp
/// parser treats an unpadded segment such as `2024-2-9` as `2024-02`, losing the day before the
/// shared date normalizer can inspect it. Unsupported or malformed tag layouts fall back to Lofty.
fn read_id3v2_text_frame(
    mut file: impl Read + Seek,
    target_id: &[u8; 4],
) -> Result<Option<String>, ()> {
    let mut header = [0; 10];
    file.read_exact(&mut header).map_err(|_| ())?;
    if &header[..3] != b"ID3" || !matches!(header[3], 3 | 4) {
        return Ok(None);
    }
    // Extended headers have version-specific layouts. Leave those to Lofty rather than guessing
    // their offset; ordinary ID3v2.3/v2.4 frames can be scanned without loading APIC data.
    if header[5] & 0x40 != 0 {
        return Err(());
    }
    let tag_size = decode_synchsafe_size(&header[6..10]).ok_or(())? as u64;
    let tag_end = 10_u64.checked_add(tag_size).ok_or(())?;
    let mut offset = 10_u64;
    while offset.checked_add(10).is_some_and(|end| end <= tag_end) {
        file.seek(SeekFrom::Start(offset)).map_err(|_| ())?;
        let mut frame_header = [0; 10];
        file.read_exact(&mut frame_header).map_err(|_| ())?;
        if frame_header[..4].iter().all(|byte| *byte == 0) {
            return Ok(None);
        }
        let frame_size = if header[3] == 4 {
            decode_synchsafe_size(&frame_header[4..8]).ok_or(())? as u64
        } else {
            u32::from_be_bytes(frame_header[4..8].try_into().map_err(|_| ())?) as u64
        };
        let content_start = offset.checked_add(10).ok_or(())?;
        let content_end = content_start.checked_add(frame_size).ok_or(())?;
        if content_end > tag_end {
            return Err(());
        }
        if &frame_header[..4] == target_id {
            if frame_size > 4096 {
                return Err(());
            }
            let format_flags = frame_header[9];
            let unsupported_flags = if header[3] == 4 {
                format_flags & 0x0c != 0 // compressed or encrypted
            } else {
                format_flags & 0xc0 != 0 // compressed or encrypted
            };
            if unsupported_flags {
                return Err(());
            }
            let mut frame = vec![0; usize::try_from(frame_size).map_err(|_| ())?];
            file.read_exact(&mut frame).map_err(|_| ())?;
            let prefix = if header[3] == 4 {
                usize::from(format_flags & 0x40 != 0) + if format_flags & 0x01 != 0 { 4 } else { 0 }
            } else {
                usize::from(format_flags & 0x20 != 0)
            };
            return decode_id3_text(frame.get(prefix..).ok_or(())?).map(Some);
        }
        offset = content_end;
    }
    Ok(None)
}

fn decode_synchsafe_size(bytes: &[u8]) -> Option<u32> {
    if bytes.len() != 4 || bytes.iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    Some(
        (u32::from(bytes[0]) << 21)
            | (u32::from(bytes[1]) << 14)
            | (u32::from(bytes[2]) << 7)
            | u32::from(bytes[3]),
    )
}

fn decode_id3_text(frame: &[u8]) -> Result<String, ()> {
    let (&encoding, content) = frame.split_first().ok_or(())?;
    match encoding {
        0 => Ok(content
            .iter()
            .copied()
            .take_while(|byte| *byte != 0)
            .map(char::from)
            .collect()),
        1 | 2 => {
            if content.len() % 2 != 0 {
                return Err(());
            }
            let little_endian = if encoding == 1 {
                match content.get(..2) {
                    Some([0xff, 0xfe, ..]) => true,
                    Some([0xfe, 0xff, ..]) => false,
                    _ => return Err(()),
                }
            } else {
                false
            };
            let content = if encoding == 1 {
                &content[2..]
            } else {
                content
            };
            let units = content
                .chunks_exact(2)
                .map(|pair| {
                    if little_endian {
                        u16::from_le_bytes([pair[0], pair[1]])
                    } else {
                        u16::from_be_bytes([pair[0], pair[1]])
                    }
                })
                .take_while(|unit| *unit != 0)
                .collect::<Vec<_>>();
            String::from_utf16(&units).map_err(|_| ())
        }
        3 => {
            let content = content.split(|byte| *byte == 0).next().ok_or(())?;
            std::str::from_utf8(content)
                .map(str::to_owned)
                .map_err(|_| ())
        }
        _ => Err(()),
    }
}

/// The lyrics written in a file's tags (the first non-empty, not larger than the lyric limit), as the tag holds them.
fn embedded_lyrics_text(tags: &[lofty::tag::Tag]) -> Option<String> {
    for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
        for raw in tags.iter().filter_map(|tag| tag.get_string(key)) {
            if !raw.trim().is_empty() && raw.len() <= player_core::MAX_LYRIC_PAYLOAD_BYTES {
                return Some(raw.to_owned());
            }
        }
    }
    None
}

pub fn mpeg_codec_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Layer1 => "MP1",
        Layer::Layer2 => "MP2",
        Layer::Layer3 => "MP3",
    }
}

/// Lofty exposes the encoded source bit depth, but some containers can carry lossy codecs.
/// Restrict reporting to formats which are intrinsically lossless; ambiguous containers remain
/// unset even if they expose a bits-per-sample property.
pub fn reliable_source_bit_depth(file_type: FileType, bit_depth: Option<u8>) -> Option<u8> {
    if !matches!(file_type, FileType::Flac | FileType::Ape) {
        return None;
    }
    bit_depth.filter(|depth| (8..=32).contains(depth))
}

/// WavPack: lofty reports neither a bit depth nor lossless or DSD, so the decoder is asked (ADR 0025).
pub fn read_wavpack_summary(path: &Path) -> Option<player_wavpack::WavPackSummary> {
    player_wavpack::summarize(path)
}

/// WAV is a container and may carry lossy codecs such as ADPCM or companded audio. Verify its
/// format chunk and report a depth only for integer PCM or IEEE float (including extensible WAV).
pub fn read_lossless_wav_bit_depth(path: &Path) -> Option<u8> {
    read_lossless_wav_bit_depth_from(std::fs::File::open(path).ok()?)
}

/// As [`read_lossless_wav_bit_depth`], for any seekable stream.
pub fn read_lossless_wav_bit_depth_from(mut file: impl Read + Seek) -> Option<u8> {
    let file_len = file.seek(SeekFrom::End(0)).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0; 12];
    file.read_exact(&mut header).ok()?;
    let is_rf64 = &header[..4] == b"RF64";
    if (!is_rf64 && &header[..4] != b"RIFF") || &header[8..12] != b"WAVE" {
        return None;
    }
    let declared_end =
        u64::from(u32::from_le_bytes(header[4..8].try_into().ok()?)).checked_add(8)?;
    let riff_end = if is_rf64 {
        file_len
    } else {
        declared_end.min(file_len)
    };
    let mut offset = 12_u64;

    // WAV chunks have even-byte alignment. Bound both the number and size of inspected chunks;
    // metadata parsing must never allocate according to an untrusted chunk length.
    for _ in 0..256 {
        if offset.checked_add(8)? > riff_end {
            return None;
        }
        file.seek(SeekFrom::Start(offset)).ok()?;
        let mut chunk_header = [0; 8];
        file.read_exact(&mut chunk_header).ok()?;
        let chunk_size = u64::from(u32::from_le_bytes(chunk_header[4..8].try_into().ok()?));
        let chunk_start = offset.checked_add(8)?;
        let chunk_end = chunk_start.checked_add(chunk_size)?;
        if chunk_end > riff_end {
            return None;
        }

        if &chunk_header[..4] == b"fmt " {
            if !(16..=1_048_576).contains(&chunk_size) {
                return None;
            }
            let mut format = vec![0; usize::try_from(chunk_size).ok()?];
            file.read_exact(&mut format).ok()?;
            let format_tag = u16::from_le_bytes(format[0..2].try_into().ok()?);
            let container_depth = u16::from_le_bytes(format[14..16].try_into().ok()?);
            let depth = match format_tag {
                0x0001 | 0x0003 => container_depth,
                0xfffe if format.len() >= 40 => {
                    let extension_size = u16::from_le_bytes(format[16..18].try_into().ok()?);
                    if extension_size < 22 {
                        return None;
                    }
                    let valid_depth = u16::from_le_bytes(format[18..20].try_into().ok()?);
                    let sub_format = &format[24..40];
                    const PCM_GUID_TAIL: [u8; 12] = [0, 0, 16, 0, 128, 0, 0, 170, 0, 56, 155, 113];
                    let subtype = u32::from_le_bytes(sub_format[..4].try_into().ok()?);
                    if !matches!(subtype, 1 | 3) || sub_format[4..] != PCM_GUID_TAIL {
                        return None;
                    }
                    if valid_depth == 0 {
                        container_depth
                    } else if valid_depth <= container_depth {
                        valid_depth
                    } else {
                        return None;
                    }
                }
                _ => return None,
            };
            return u8::try_from(depth)
                .ok()
                .filter(|depth| (8..=32).contains(depth));
        }

        // A valid WAV fmt chunk precedes the audio data. Don't scan into a large data region
        // looking for bytes that happen to resemble another chunk.
        if &chunk_header[..4] == b"data" {
            return None;
        }
        offset = chunk_end.checked_add(chunk_size & 1)?;
    }
    None
}

/// DSF: properties from the file header, tags from the ID3v2 tag at the end of the file (lofty has no DSF reader).
fn read_dsf_audio_metadata_from(
    reader: impl Read + Seek + Send,
) -> Result<TrackMetadata, TrackMetadataError> {
    let (info, tag) =
        player_dsd::id3::read_dsf_from(reader).map_err(|error| TrackMetadataError {
            message: error.to_string(),
        })?;
    let tag = tag.as_ref();
    let text = |value: Option<std::borrow::Cow<'_, str>>| value.map(|value| value.into_owned());
    let release_date = tag
        .and_then(|tag| tag.get_string(ItemKey::RecordingDate))
        .and_then(player_core::parse_recording_date);
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year);
    Ok(TrackMetadata {
        embedded_lyrics: None,
        title: tag
            .and_then(|tag| text(tag.title()))
            .filter(|value| !value.trim().is_empty()),
        artist: tag.and_then(|tag| text(tag.artist())),
        album: tag.and_then(|tag| text(tag.album())),
        album_artist: tag.and_then(|tag| tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)),
        track_number: tag.and_then(|tag| tag.track()),
        disc_number: tag.and_then(|tag| tag.disk()),
        duration_ms: Some(info.duration().as_millis().min(u64::MAX as u128) as u64),
        codec: Some(info.rate_name()),
        bitrate_bps: Some(info.sample_rate.saturating_mul(u32::from(info.channels))),
        sample_rate_hz: Some(info.sample_rate),
        release_date,
        year,
        bit_depth: Some(1),
    })
}

/// A `Read + Seek + Send` stream as Symphonia's media source (which must also be `Sync`: every access takes a lock).
struct SyncMedia {
    reader: std::sync::Mutex<Box<dyn ReadSeek>>,
    len: Option<u64>,
}

impl SyncMedia {
    fn new(mut reader: Box<dyn ReadSeek>) -> Self {
        // Symphonia's Matroska demuxer needs the length to seek.
        let len = reader.seek(SeekFrom::End(0)).ok();
        let _ = reader.seek(SeekFrom::Start(0));
        Self {
            reader: std::sync::Mutex::new(reader),
            len,
        }
    }
}

impl Read for SyncMedia {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.reader
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .read(buf)
    }
}

impl Seek for SyncMedia {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.reader
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .seek(position)
    }
}

impl symphonia::core::io::MediaSource for SyncMedia {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

fn read_matroska_audio_metadata(
    source: &dyn ByteSource,
) -> Result<TrackMetadata, TrackMetadataError> {
    use symphonia::core::codecs::{
        CODEC_TYPE_AAC, CODEC_TYPE_ALAC, CODEC_TYPE_FLAC, CODEC_TYPE_MP3, CODEC_TYPE_OPUS,
        CODEC_TYPE_VORBIS,
    };

    let open = |what: &str| {
        source.open().map_err(|error| TrackMetadataError {
            message: format!("could not open Matroska audio file ({what}): {error}"),
        })
    };
    let stream =
        MediaSourceStream::new(Box::new(SyncMedia::new(open("audio")?)), Default::default());
    // Symphonia identifies Matroska from the EBML header; the supported demuxer advertises .mkv,
    // so use that container hint for audio-only .mka files as well.
    let mut hint = Hint::new();
    hint.with_extension("mkv");
    let mut probed = get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| TrackMetadataError {
            message: format!("could not parse Matroska audio metadata: {error}"),
        })?;
    let reader = &mut probed.format;
    let track = reader
        .tracks()
        .iter()
        .find(|track| track.codec_params.sample_rate.is_some())
        .ok_or_else(|| TrackMetadataError {
            message: "Matroska file contains no audio track".to_owned(),
        })?;
    let params = track.codec_params.clone();
    let symphonia_tags = reader
        .metadata()
        .current()
        .map(|revision| revision.tags().to_vec())
        .unwrap_or_default();
    // Symphonia demuxes Matroska audio but does not expose the container Info title
    // consistently. Read only Info and Tags elements separately; this avoids loading
    // large attached artwork while keeping audio probing on Symphonia.
    let info = matroska::get::<_, matroska::Info>(std::io::BufReader::new(open("info")?)).map_err(
        |error| TrackMetadataError {
            message: format!("could not read Matroska Info metadata: {error}"),
        },
    )?;
    let container_tags = matroska::get::<_, matroska::Tag>(std::io::BufReader::new(open("tags")?))
        .map_err(|error| TrackMetadataError {
            message: format!("could not read Matroska Tags metadata: {error}"),
        })?
        .unwrap_or_default();
    let container_tracks =
        matroska::get::<_, matroska::Track>(std::io::BufReader::new(open("tracks")?))
            .map_err(|error| TrackMetadataError {
                message: format!("could not read Matroska track metadata: {error}"),
            })?
            .unwrap_or_default();
    let matroska_tag_text = |keys: &[&str]| {
        container_tags
            .iter()
            .flat_map(|tag| tag.simple.iter())
            .find(|tag| keys.iter().any(|key| tag.name.eq_ignore_ascii_case(key)))
            .and_then(|tag| match &tag.value {
                Some(matroska::TagValue::String(value)) => Some(value.clone()),
                _ => None,
            })
            .filter(|value| !value.trim().is_empty())
    };
    let symphonia_tag_text = |keys: &[&str]| {
        symphonia_tags
            .iter()
            .find(|tag| keys.iter().any(|key| tag.key.eq_ignore_ascii_case(key)))
            .map(|tag| tag.value.to_string())
            .filter(|value| !value.trim().is_empty())
    };
    let tag_text = |keys: &[&str]| matroska_tag_text(keys).or_else(|| symphonia_tag_text(keys));
    let parse_number = |keys: &[&str]| {
        tag_text(keys).and_then(|value| {
            value
                .split('/')
                .next()
                .and_then(|number| number.trim().parse::<u32>().ok())
        })
    };
    let bitrate_bps = matroska_bitrate_bps(&container_tags, &symphonia_tags);
    let codec = if params.codec == CODEC_TYPE_AAC {
        "AAC"
    } else if params.codec == CODEC_TYPE_ALAC {
        "ALAC"
    } else if params.codec == CODEC_TYPE_FLAC {
        "FLAC"
    } else if params.codec == CODEC_TYPE_MP3 {
        "MP3"
    } else if params.codec == CODEC_TYPE_OPUS {
        "Opus"
    } else if params.codec == CODEC_TYPE_VORBIS {
        "Vorbis"
    } else {
        "Matroska audio"
    };
    let duration_ms = params
        .time_base
        .zip(params.n_frames)
        .map(|(time_base, frames)| {
            let duration = time_base.calc_time(frames);
            (duration.seconds as u128 * 1000 + (duration.frac * 1000.0).round() as u128)
                .min(u64::MAX as u128) as u64
        });
    let release_date = tag_text(&["DATE_RECORDED", "RECORDING_DATE", "DATE"])
        .and_then(|value| player_core::parse_recording_date(&value));
    let year = release_date
        .as_deref()
        .and_then(player_core::recording_date_year);

    Ok(TrackMetadata {
        embedded_lyrics: None,
        title: matroska_tag_text(&["TITLE"])
            .or_else(|| info.as_ref().and_then(|info| info.title.clone()))
            .or_else(|| {
                container_tracks
                    .iter()
                    .find(|track| track.is_audio())
                    .and_then(|track| track.name.clone())
            })
            .or_else(|| symphonia_tag_text(&["TITLE", "TRACK_NAME"])),
        artist: tag_text(&["ARTIST", "LEAD_PERFORMER", "ARTIST/LEADPERFORMER"]),
        album: tag_text(&["ALBUM"]),
        album_artist: tag_text(&["ALBUM_ARTIST", "ALBUMARTIST", "ARTIST/ALBUMARTIST"]),
        track_number: parse_number(&["PART_NUMBER", "TRACK_NUMBER", "TRACKNUMBER"]),
        disc_number: parse_number(&["DISC_NUMBER", "DISCNUMBER"]),
        duration_ms,
        codec: Some(codec.to_owned()),
        bitrate_bps,
        sample_rate_hz: params.sample_rate,
        release_date,
        year,
        bit_depth: if matches!(codec, "FLAC" | "ALAC" | "Matroska audio") {
            params
                .bits_per_sample
                .and_then(|value| u8::try_from(value).ok())
        } else {
            None
        },
    })
}

pub fn matroska_bitrate_bps(
    container_tags: &[matroska::Tag],
    symphonia_tags: &[symphonia::core::meta::Tag],
) -> Option<u32> {
    let container_value = container_tags
        .iter()
        .flat_map(|tag| tag.simple.iter())
        .find(|tag| tag.name.eq_ignore_ascii_case("BPS"))
        .and_then(|tag| match &tag.value {
            Some(matroska::TagValue::String(value)) => Some(value.clone()),
            _ => None,
        });
    let value = container_value.or_else(|| {
        symphonia_tags
            .iter()
            .find(|tag| tag.key.eq_ignore_ascii_case("BPS"))
            .map(|tag| tag.value.to_string())
    })?;
    value.trim().parse::<u32>().ok().filter(|value| *value > 0)
}
