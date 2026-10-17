//! Tags of a DSF file: the ID3v2 tag that follows the audio data (feature `id3`).
//!
//! lofty has no DSF support and does not expose its ID3v2 parser, so the tag bytes are wrapped in a minimal MP3
//! (the tag followed by a few silent frames) and read with lofty's MPEG reader, which gives the usual generic tag,
//! including embedded pictures.

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::mpeg::MpegFile;
use lofty::tag::Tag;

use crate::dsf::DsfReader;
use crate::stream::{DsdError, DsdInfo, DsdStream};

/// Largest tag read from a file (cover art included).
const MAX_TAG_BYTES: u64 = 64 * 1024 * 1024;
/// One silent MPEG-1 Layer III frame, 128 kbit/s, 44.1 kHz, stereo (417 bytes).
const MP3_FRAME_LEN: usize = 417;

fn silent_frames() -> Vec<u8> {
    let mut frames = Vec::with_capacity(MP3_FRAME_LEN * 4);
    for _ in 0..4 {
        let mut frame = vec![0_u8; MP3_FRAME_LEN];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        frames.extend_from_slice(&frame);
    }
    frames
}

/// Parse the ID3v2 tag found in `bytes` (which must start with `ID3`).
pub fn parse_id3v2_bytes(bytes: &[u8]) -> Result<Option<Tag>, DsdError> {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return Ok(None);
    }
    let mut wrapped = bytes.to_vec();
    wrapped.extend_from_slice(&silent_frames());
    let file = MpegFile::read_from(&mut Cursor::new(wrapped), ParseOptions::new())
        .map_err(|error| DsdError::InvalidFormat(format!("ID3v2 標籤無法解析：{error}")))?;
    let tagged = lofty::file::TaggedFile::from(file);
    Ok(tagged.primary_tag().or_else(|| tagged.first_tag()).cloned())
}

/// Stream info and tag of a DSF file. The tag is `None` when the file has none or it cannot be parsed.
pub fn read_dsf(path: &Path) -> Result<(DsdInfo, Option<Tag>), DsdError> {
    let file = File::open(path)?;
    let reader = DsfReader::open(BufReader::new(file))?;
    let info = reader.info();
    let Some(offset) = reader.id3v2_offset() else {
        return Ok((info, None));
    };
    let mut file = File::open(path)?;
    let length = file.seek(SeekFrom::End(0))?;
    let size = length.saturating_sub(offset);
    if size == 0 || size > MAX_TAG_BYTES {
        return Ok((info, None));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; size as usize];
    file.read_exact(&mut bytes)?;
    // A damaged tag must not hide a playable file.
    Ok((info, parse_id3v2_bytes(&bytes).unwrap_or(None)))
}

#[cfg(test)]
mod tests {
    use lofty::tag::{Accessor, ItemKey, TagExt, TagType};

    use super::*;

    fn id3_bytes(title: &str, artist: &str) -> Vec<u8> {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(title.to_owned());
        tag.set_artist(artist.to_owned());
        tag.set_album("專輯".to_owned());
        tag.insert_text(ItemKey::AlbumArtist, "Various".to_owned());
        let mut bytes = Vec::new();
        tag.dump_to(&mut bytes, lofty::config::WriteOptions::default())
            .unwrap();
        bytes
    }

    #[test]
    fn id3v2_bytes_parse_to_a_generic_tag() {
        let tag = parse_id3v2_bytes(&id3_bytes("曲名", "演出者"))
            .unwrap()
            .unwrap();
        assert_eq!(tag.title().as_deref(), Some("曲名"));
        assert_eq!(tag.artist().as_deref(), Some("演出者"));
        assert_eq!(tag.album().as_deref(), Some("專輯"));
        assert_eq!(tag.get_string(ItemKey::AlbumArtist), Some("Various"));
    }

    #[test]
    fn non_id3_bytes_give_no_tag() {
        assert!(parse_id3v2_bytes(b"not a tag").unwrap().is_none());
        assert!(parse_id3v2_bytes(b"").unwrap().is_none());
    }

    #[test]
    fn a_dsf_with_a_trailing_tag_exposes_it() {
        let directory = std::env::temp_dir().join(format!("moe-dsd-id3-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("tagged.dsf");
        let mut file = crate::testing::build_dsf(
            2_822_400,
            80_000,
            4096,
            true,
            &[vec![0x69; 10_000], vec![0x69; 10_000]],
        );
        let tag_offset = file.len() as u64;
        file[20..28].copy_from_slice(&tag_offset.to_le_bytes());
        file.extend_from_slice(&id3_bytes("DSD 曲名", "DSD 演出者"));
        std::fs::write(&path, &file).unwrap();
        let (info, tag) = read_dsf(&path).unwrap();
        assert_eq!(info.rate_name(), "DSD64");
        let tag = tag.expect("tag");
        assert_eq!(tag.title().as_deref(), Some("DSD 曲名"));
        assert_eq!(tag.artist().as_deref(), Some("DSD 演出者"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_dsf_without_a_tag_reads_fine() {
        let directory = std::env::temp_dir().join(format!("moe-dsd-notag-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("plain.dsf");
        std::fs::write(
            &path,
            crate::testing::build_dsf(2_822_400, 8000, 4096, true, &[vec![0x69; 1000]]),
        )
        .unwrap();
        let (info, tag) = read_dsf(&path).unwrap();
        assert_eq!(info.channels, 1);
        assert!(tag.is_none());
        let _ = std::fs::remove_dir_all(directory);
    }
}
