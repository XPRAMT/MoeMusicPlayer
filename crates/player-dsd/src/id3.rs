//! Tags of a DSF file: the ID3v2 tag that follows the audio data (feature `id3`).
//!
//! lofty has no DSF support and does not expose its ID3v2 parser, so the tag bytes are wrapped in a minimal MP3
//! (the tag followed by a few silent frames) and read with lofty's MPEG reader, which gives the usual generic tag,
//! including embedded pictures.

use std::fs::File;
use std::io::{BufReader, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::AudioFile;
use lofty::file::TaggedFileExt;
use lofty::mpeg::MpegFile;
use lofty::tag::{Tag, TagExt};

use crate::dsf::{DsfReader, DATA_HEADER_SIZE, DATA_OFFSET};
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
    read_dsf_from(File::open(path)?)
}

/// Like [`read_dsf`], for a file the caller already opened (for example with extra path-safety checks).
pub fn read_dsf_from<R: Read + Seek + Send>(
    mut reader: R,
) -> Result<(DsdInfo, Option<Tag>), DsdError> {
    let dsf = DsfReader::open(BufReader::new(&mut reader))?;
    let info = dsf.info();
    let Some(offset) = dsf.id3v2_offset() else {
        return Ok((info, None));
    };
    drop(dsf);
    let length = reader.seek(SeekFrom::End(0))?;
    let size = length.saturating_sub(offset);
    if size == 0 || size > MAX_TAG_BYTES {
        return Ok((info, None));
    }
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0_u8; size as usize];
    reader.read_exact(&mut bytes)?;
    // A damaged tag must not hide a playable file.
    Ok((info, parse_id3v2_bytes(&bytes).unwrap_or(None)))
}

/// Write `destination` as a copy of the DSF at `source` whose trailing ID3v2 tag is replaced by `tag` (written as
/// ID3v2.4). The audio data is copied byte for byte; the header's total-file-size and metadata-pointer fields are
/// updated. An empty `tag` removes the tag (pointer 0). `destination` must not be `source`.
pub fn rewrite_dsf_with_tag(source: &Path, destination: &Path, tag: &Tag) -> Result<(), DsdError> {
    // Validate with the regular reader first (also rejects truncated or malformed files).
    DsfReader::open(BufReader::new(File::open(source)?))?;
    let mut input = File::open(source)?;
    let file_length = input.seek(SeekFrom::End(0))?;
    let mut header = [0_u8; DATA_OFFSET as usize];
    input.seek(SeekFrom::Start(0))?;
    input.read_exact(&mut header)?;
    let data_chunk_size = u64::from_le_bytes(header[84..92].try_into().expect("8 bytes"));
    // The audio (everything up to the end of the data chunk) is kept; the old tag and anything after it is dropped.
    let audio_end = DATA_OFFSET - DATA_HEADER_SIZE + data_chunk_size;
    if audio_end > file_length || audio_end < DATA_OFFSET {
        return Err(DsdError::InvalidFormat("data 區塊超出檔案範圍".into()));
    }

    let mut tag_bytes = Vec::new();
    if !tag.is_empty() {
        tag.dump_to(&mut tag_bytes, WriteOptions::default())
            .map_err(|error| DsdError::InvalidFormat(format!("無法產生 ID3v2 標籤：{error}")))?;
    }
    let total_length = audio_end + tag_bytes.len() as u64;
    let pointer = if tag_bytes.is_empty() { 0 } else { audio_end };
    header[12..20].copy_from_slice(&total_length.to_le_bytes());
    header[20..28].copy_from_slice(&pointer.to_le_bytes());

    let mut output = File::create(destination)?;
    output.write_all(&header)?;
    input.seek(SeekFrom::Start(DATA_OFFSET))?;
    let copied = std::io::copy(
        &mut Read::by_ref(&mut input).take(audio_end - DATA_OFFSET),
        &mut output,
    )?;
    if copied != audio_end - DATA_OFFSET {
        return Err(DsdError::Io("複製音訊資料時檔案提早結束".into()));
    }
    output.write_all(&tag_bytes)?;
    output.sync_all()?;
    Ok(())
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

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let directory =
            std::env::temp_dir().join(format!("moe-dsd-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn read_all_bytes(path: &Path) -> Vec<Vec<u8>> {
        let mut reader = DsfReader::open(BufReader::new(File::open(path).unwrap())).unwrap();
        let channels = usize::from(reader.info().channels);
        let mut all = vec![Vec::new(); channels];
        let mut buffers = vec![Vec::new(); channels];
        while reader.read(4096, &mut buffers).unwrap() > 0 {
            for (target, buffer) in all.iter_mut().zip(&buffers) {
                target.extend_from_slice(buffer);
            }
        }
        all
    }

    #[test]
    fn rewriting_replaces_the_tag_keeps_the_audio_and_fixes_the_header() {
        let directory = temp_dir("rewrite");
        let source = directory.join("in.dsf");
        let payload: Vec<u8> = (0..10_000_u32)
            .map(|value| (value * 31 % 251) as u8)
            .collect();
        let mut file =
            crate::testing::build_dsf(2_822_400, 80_000, 4096, true, &[payload.clone(), payload]);
        let old_tag_offset = file.len() as u64;
        file[20..28].copy_from_slice(&old_tag_offset.to_le_bytes());
        file.extend_from_slice(&id3_bytes("舊曲名", "舊演出者"));
        std::fs::write(&source, &file).unwrap();

        let (_, tag) = read_dsf(&source).unwrap();
        let mut tag = tag.expect("tag");
        tag.set_title("新曲名".to_owned());
        tag.set_artist("新演出者".to_owned());
        tag.insert_text(ItemKey::Lyrics, "[00:01.00]歌詞".to_owned());
        let destination = directory.join("out.dsf");
        rewrite_dsf_with_tag(&source, &destination, &tag).unwrap();

        let (info, edited) = read_dsf(&destination).unwrap();
        assert_eq!(info.samples_per_channel, 80_000);
        let edited = edited.expect("tag after rewrite");
        assert_eq!(edited.title().as_deref(), Some("新曲名"));
        assert_eq!(edited.artist().as_deref(), Some("新演出者"));
        assert_eq!(edited.album().as_deref(), Some("專輯"));
        assert_eq!(read_all_bytes(&source), read_all_bytes(&destination));
        let bytes = std::fs::read(&destination).unwrap();
        assert_eq!(
            u64::from_le_bytes(bytes[12..20].try_into().unwrap()),
            bytes.len() as u64
        );
        assert_eq!(
            u64::from_le_bytes(bytes[20..28].try_into().unwrap()),
            old_tag_offset
        );
        // Rewriting again (the tag grows or shrinks) still produces a valid file.
        let third = directory.join("third.dsf");
        let mut shorter = edited.clone();
        shorter.remove_key(ItemKey::Lyrics);
        shorter.set_title("短".to_owned());
        rewrite_dsf_with_tag(&destination, &third, &shorter).unwrap();
        assert_eq!(
            read_dsf(&third).unwrap().1.unwrap().title().as_deref(),
            Some("短")
        );
        assert_eq!(read_all_bytes(&source), read_all_bytes(&third));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn rewriting_with_an_empty_tag_removes_it() {
        let directory = temp_dir("remove-tag");
        let source = directory.join("in.dsf");
        let mut file = crate::testing::build_dsf(2_822_400, 8_000, 4096, true, &[vec![0x69; 1000]]);
        let old_tag_offset = file.len() as u64;
        file[20..28].copy_from_slice(&old_tag_offset.to_le_bytes());
        file.extend_from_slice(&id3_bytes("x", "y"));
        std::fs::write(&source, &file).unwrap();
        let destination = directory.join("out.dsf");
        rewrite_dsf_with_tag(&source, &destination, &Tag::new(TagType::Id3v2)).unwrap();
        let bytes = std::fs::read(&destination).unwrap();
        assert_eq!(bytes.len() as u64, old_tag_offset);
        assert_eq!(u64::from_le_bytes(bytes[20..28].try_into().unwrap()), 0);
        assert!(read_dsf(&destination).unwrap().1.is_none());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn adding_a_tag_to_an_untagged_dsf_works() {
        let directory = temp_dir("add-tag");
        let source = directory.join("in.dsf");
        std::fs::write(
            &source,
            crate::testing::build_dsf(2_822_400, 8_000, 4096, true, &[vec![0x69; 1000]]),
        )
        .unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("新增".to_owned());
        let destination = directory.join("out.dsf");
        rewrite_dsf_with_tag(&source, &destination, &tag).unwrap();
        assert_eq!(
            read_dsf(&destination)
                .unwrap()
                .1
                .unwrap()
                .title()
                .as_deref(),
            Some("新增")
        );
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
