use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use player_core::{TrackMetadata, TrackMetadataError};
use symphonia::core::{
    codecs::{CODEC_TYPE_AAC, CODEC_TYPE_ALAC, CODEC_TYPE_MP3, CODEC_TYPE_NULL},
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::{MetadataOptions, StandardTagKey, Value},
};

pub(crate) fn read(path: &Path) -> Result<TrackMetadata, TrackMetadataError> {
    match read_symphonia(path) {
        Ok(metadata) => Ok(metadata),
        Err(symphonia_error) => read_mp4_properties(path).or(Err(symphonia_error)),
    }
}

fn read_symphonia(path: &Path) -> Result<TrackMetadata, TrackMetadataError> {
    let file = File::open(path).map_err(|error| TrackMetadataError {
        message: format!("could not open Android audio metadata: {error}"),
    })?;
    let size_bytes = file.metadata().ok().map(|metadata| metadata.len());
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let probed = symphonia::default::get_probe()
        .format(
            &Default::default(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| TrackMetadataError {
            message: format!("could not parse Android audio container: {error}"),
        })?;
    let mut format = probed.format;
    let metadata = format.metadata();
    let tags = metadata
        .current()
        .map(|revision| revision.tags().to_vec())
        .unwrap_or_default();
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| TrackMetadataError {
            message: "Android audio container has no audio track".to_owned(),
        })?;
    let params = &track.codec_params;
    let codec = match params.codec {
        CODEC_TYPE_AAC => Some("AAC"),
        CODEC_TYPE_ALAC => Some("ALAC"),
        CODEC_TYPE_MP3 => Some("MP3"),
        _ => None,
    };
    let duration_ms = params
        .time_base
        .zip(params.n_frames)
        .map(|(time_base, frames)| {
            let duration = time_base.calc_time(frames);
            ((duration.seconds as f64 + duration.frac) * 1000.0)
                .round()
                .min(u64::MAX as f64) as u64
        });
    let value = |key| {
        tags.iter()
            .find(|tag| tag.std_key == Some(key))
            .map(|tag| match &tag.value {
                Value::String(value) => value.clone(),
                other => other.to_string(),
            })
            .filter(|value| !value.trim().is_empty())
    };
    let recording_date = [
        StandardTagKey::Date,
        StandardTagKey::ReleaseDate,
        StandardTagKey::OriginalDate,
    ]
    .into_iter()
    .find_map(value)
    .and_then(|value| player_core::parse_recording_date(&value));
    let year = recording_date
        .as_deref()
        .and_then(player_core::recording_date_year);
    let bitrate_bps = duration_ms
        .filter(|duration| *duration > 0)
        .and_then(|duration| size_bytes.map(|size| size.saturating_mul(8_000) / duration))
        .and_then(|bitrate| u32::try_from(bitrate).ok());
    let metadata = TrackMetadata {
        title: value(StandardTagKey::TrackTitle),
        artist: value(StandardTagKey::Artist),
        album: value(StandardTagKey::Album),
        album_artist: value(StandardTagKey::AlbumArtist),
        track_number: value(StandardTagKey::TrackNumber).and_then(|value| parse_tag_number(&value)),
        disc_number: value(StandardTagKey::DiscNumber).and_then(|value| parse_tag_number(&value)),
        duration_ms,
        codec: codec.map(str::to_owned),
        bitrate_bps,
        sample_rate_hz: params.sample_rate,
        release_date: recording_date,
        year,
        bit_depth: None,
    };
    if metadata.codec.is_none()
        && metadata.duration_ms.is_none()
        && metadata.sample_rate_hz.is_none()
    {
        return Err(TrackMetadataError {
            message: "Android audio container did not provide usable audio properties".to_owned(),
        });
    }
    Ok(metadata)
}

#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    start: usize,
    payload: usize,
    end: usize,
}

fn read_mp4_properties(path: &Path) -> Result<TrackMetadata, TrackMetadataError> {
    const MAX_MOVIE_ATOM_BYTES: u64 = 32 * 1024 * 1024;
    let mut file = File::open(path).map_err(|error| TrackMetadataError {
        message: error.to_string(),
    })?;
    let file_size = file
        .metadata()
        .map_err(|error| TrackMetadataError {
            message: error.to_string(),
        })?
        .len();
    let mut position = 0_u64;
    let mut movie_atom = None;
    let mut media_bytes = 0_u64;
    while position + 8 <= file_size {
        file.seek(SeekFrom::Start(position))
            .map_err(|error| TrackMetadataError {
                message: error.to_string(),
            })?;
        let mut header = [0_u8; 16];
        file.read_exact(&mut header[..8])
            .map_err(|error| TrackMetadataError {
                message: error.to_string(),
            })?;
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let kind: [u8; 4] = header[4..8].try_into().unwrap();
        let header_size = if size == 1 {
            file.read_exact(&mut header[8..16])
                .map_err(|error| TrackMetadataError {
                    message: error.to_string(),
                })?;
            size = u64::from_be_bytes(header[8..16].try_into().unwrap());
            16
        } else {
            8
        };
        if size == 0 {
            size = file_size - position;
        }
        if size < header_size || position.saturating_add(size) > file_size {
            return Err(TrackMetadataError {
                message: "invalid ISO base media atom size".to_owned(),
            });
        }
        if &kind == b"mdat" {
            media_bytes = media_bytes.saturating_add(size - header_size);
        }
        if &kind == b"moov" {
            if size > MAX_MOVIE_ATOM_BYTES {
                return Err(TrackMetadataError {
                    message: "ISO base media metadata atom exceeds the 32 MiB limit".to_owned(),
                });
            }
            movie_atom = Some((position, size, header_size as u64));
        }
        position += size;
    }
    let (movie_start, movie_size, movie_header_size) =
        movie_atom.ok_or_else(|| TrackMetadataError {
            message: "ISO base media file has no movie metadata atom".to_owned(),
        })?;
    let payload_size =
        usize::try_from(movie_size - movie_header_size).map_err(|_| TrackMetadataError {
            message: "ISO base media metadata atom is too large".to_owned(),
        })?;
    file.seek(SeekFrom::Start(movie_start + movie_header_size))
        .map_err(|error| TrackMetadataError {
            message: error.to_string(),
        })?;
    let mut movie = vec![0_u8; payload_size];
    file.read_exact(&mut movie)
        .map_err(|error| TrackMetadataError {
            message: error.to_string(),
        })?;
    let root = atoms(&movie, 0, movie.len())?;
    let mut duration_ms = None;
    let mut sample_rate_hz = None;
    let mut codec = None;
    for track in root.iter().filter(|atom| &atom.kind == b"trak") {
        let track_children = atoms(&movie, track.payload, track.end)?;
        let Some(media) = track_children.iter().find(|atom| &atom.kind == b"mdia") else {
            continue;
        };
        let media_children = atoms(&movie, media.payload, media.end)?;
        let is_audio = media_children
            .iter()
            .find(|atom| &atom.kind == b"hdlr")
            .is_some_and(|handler| {
                movie
                    .get(handler.payload + 8..handler.payload + 12)
                    .is_some_and(|value| value == b"soun")
            });
        if !is_audio {
            continue;
        }
        if let Some(header) = media_children.iter().find(|atom| &atom.kind == b"mdhd") {
            duration_ms = parse_media_header(&movie, *header);
        }
        let Some(minf) = media_children.iter().find(|atom| &atom.kind == b"minf") else {
            continue;
        };
        let minf_children = atoms(&movie, minf.payload, minf.end)?;
        let Some(stbl) = minf_children.iter().find(|atom| &atom.kind == b"stbl") else {
            continue;
        };
        let stbl_children = atoms(&movie, stbl.payload, stbl.end)?;
        let Some(stsd) = stbl_children.iter().find(|atom| &atom.kind == b"stsd") else {
            continue;
        };
        let description_children = atoms(&movie, stsd.payload + 8, stsd.end)?;
        let Some(description) = description_children.first() else {
            continue;
        };
        let Some(channel_count) = read_u16(&movie, description.start + 24) else {
            continue;
        };
        let Some(sample_rate_q16) = read_u32(&movie, description.start + 32) else {
            continue;
        };
        if channel_count > 0 {
            sample_rate_hz = Some(sample_rate_q16 >> 16);
        }
        codec = match &description.kind {
            b"mp4a" => Some("AAC".to_owned()),
            b"alac" => Some("ALAC".to_owned()),
            _ => None,
        };
        if codec.is_some() && sample_rate_hz.is_some() {
            break;
        }
    }
    let codec = codec.ok_or_else(|| TrackMetadataError {
        message: "ISO base media audio sample description is unsupported".to_owned(),
    })?;
    let bitrate_bps = duration_ms
        .filter(|duration| *duration > 0)
        .and_then(|duration| u32::try_from(media_bytes.saturating_mul(8_000) / duration).ok());
    Ok(TrackMetadata {
        title: None,
        artist: None,
        album: None,
        album_artist: None,
        track_number: None,
        disc_number: None,
        duration_ms,
        codec: Some(codec),
        bitrate_bps,
        sample_rate_hz,
        release_date: None,
        year: None,
        bit_depth: None,
    })
}

fn atoms(data: &[u8], start: usize, end: usize) -> Result<Vec<Atom>, TrackMetadataError> {
    let mut found = Vec::new();
    let mut position = start;
    while position + 8 <= end {
        let size = read_u32(data, position).ok_or_else(invalid_atom)?;
        let kind: [u8; 4] = data
            .get(position + 4..position + 8)
            .ok_or_else(invalid_atom)?
            .try_into()
            .map_err(|_| invalid_atom())?;
        let header_size = if size == 1 { 16 } else { 8 };
        let atom_size = if size == 0 {
            end - position
        } else if size == 1 {
            usize::try_from(read_u64(data, position + 8).ok_or_else(invalid_atom)?)
                .map_err(|_| invalid_atom())?
        } else {
            size as usize
        };
        let atom_end = position.checked_add(atom_size).ok_or_else(invalid_atom)?;
        if atom_size < header_size || atom_end > end {
            return Err(invalid_atom());
        }
        found.push(Atom {
            kind,
            start: position,
            payload: position + header_size,
            end: atom_end,
        });
        position = atom_end;
    }
    Ok(found)
}

fn parse_media_header(data: &[u8], atom: Atom) -> Option<u64> {
    let version = *data.get(atom.payload)?;
    let (timescale, duration) = if version == 0 {
        (
            u64::from(read_u32(data, atom.payload + 12)?),
            u64::from(read_u32(data, atom.payload + 16)?),
        )
    } else if version == 1 {
        (
            u64::from(read_u32(data, atom.payload + 20)?),
            read_u64(data, atom.payload + 24)?,
        )
    } else {
        return None;
    };
    (timescale > 0).then_some(duration.saturating_mul(1000) / timescale)
}

fn read_u16(data: &[u8], position: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(position..position + 2)?.try_into().ok()?,
    ))
}

fn read_u32(data: &[u8], position: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(position..position + 4)?.try_into().ok()?,
    ))
}

fn read_u64(data: &[u8], position: usize) -> Option<u64> {
    Some(u64::from_be_bytes(
        data.get(position..position + 8)?.try_into().ok()?,
    ))
}

fn invalid_atom() -> TrackMetadataError {
    TrackMetadataError {
        message: "invalid ISO base media atom structure".to_owned(),
    }
}

fn parse_tag_number(value: &str) -> Option<u32> {
    value
        .split('/')
        .next()?
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{read, read_symphonia};

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/player-audio-windows/tests/fixtures/aac-lc.m4a")
    }

    #[test]
    fn reads_aac_mp4_technical_properties_without_android_framework() {
        let metadata = read(&fixture()).expect("Symphonia should read the AAC fixture");
        assert_eq!(metadata.codec.as_deref(), Some("AAC"));
        assert_eq!(metadata.sample_rate_hz, Some(48_000));
        assert!(metadata.duration_ms.is_some_and(|duration| duration > 0));
    }

    #[test]
    fn parses_the_reported_android_m4a_when_capture_is_available() {
        let Ok(path) = std::env::var("MOE_MEDIA_METADATA_TEST_FILE") else {
            return;
        };
        let path = Path::new(&path);
        assert!(
            read_symphonia(path).is_err(),
            "the captured file should exercise the tolerant MP4 fallback"
        );
        let metadata = read(path).expect("the tolerant parser should read the captured AAC");
        assert_eq!(metadata.codec.as_deref(), Some("AAC"));
        assert_eq!(metadata.sample_rate_hz, Some(16_000));
        assert_eq!(metadata.duration_ms, Some(70_144));
    }
}
