//! On-demand cover lookup for enabled Windows filesystem track mappings.

use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

use lofty::{config::ParseOptions, picture::PictureType, prelude::TaggedFileExt, probe::Probe};
use player_core::{ArtworkImage, MediaLocator, MAX_ARTWORK_BYTES};

use crate::windows::to_extended_path;

const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const SIDE_CAR_NAMES: [&str; 2] = ["cover.jpg", "folder.jpg"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtworkLookup {
    Found(ArtworkImage),
    Missing,
    Oversized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ImageHeader {
    mime_type: &'static str,
    width: u32,
    height: u32,
}

/// Find one original-resolution cover for a track.
///
/// `locators` must come from `Database::track_locators(track_id)`, which filters disabled
/// library roots and keeps the native filesystem path. Content URIs are ignored by this Windows
/// adapter. All embedded pictures across enabled filesystem mappings are tried before sidecars;
/// sidecars are then tried in the order `cover.jpg`, `folder.jpg`. No path is returned.
///
/// The work is synchronous filesystem and metadata I/O. Callers should run it on a blocking worker
/// and discard the result when the active TrackId changes. Malformed, unsupported, inaccessible,
/// or over-limit candidates are skipped and lookup continues.
pub fn find_artwork(locators: &[MediaLocator]) -> ArtworkLookup {
    let paths = locators
        .iter()
        .filter_map(|locator| match locator {
            MediaLocator::FileSystem(path) => Some(path),
            MediaLocator::ContentUri(_) => None,
        })
        .collect::<Vec<_>>();

    let mut oversized = false;
    for path in &paths {
        if let Some(image) = embedded_artwork(path, &mut oversized) {
            return ArtworkLookup::Found(image);
        }
    }

    for sidecar_name in SIDE_CAR_NAMES {
        for path in &paths {
            if let Some(image) = sidecar_artwork(path, sidecar_name, &mut oversized) {
                return ArtworkLookup::Found(image);
            }
        }
    }

    if oversized {
        ArtworkLookup::Oversized
    } else {
        ArtworkLookup::Missing
    }
}

fn embedded_artwork(path: &Path, oversized: &mut bool) -> Option<ArtworkImage> {
    let native_path = to_extended_path(path).ok()?;
    let file = open_regular_file_without_following_reparse_points(&native_path)?;
    let options = ParseOptions::new().read_properties(false);
    let tagged_file = Probe::new(std::io::BufReader::new(file))
        .options(options)
        .guess_file_type()
        .ok()?
        .read()
        .ok()?;
    let tags = tagged_file.tags();

    for picture in tags
        .iter()
        .flat_map(|tag| tag.pictures())
        .filter(|picture| picture.pic_type() == PictureType::CoverFront)
    {
        if let Some(image) = image_from_raw_bytes(picture.data(), oversized) {
            return Some(image);
        }
    }
    for picture in tags
        .iter()
        .flat_map(|tag| tag.pictures())
        .filter(|picture| picture.pic_type() != PictureType::CoverFront)
    {
        if let Some(image) = image_from_raw_bytes(picture.data(), oversized) {
            return Some(image);
        }
    }

    None
}

fn sidecar_artwork(audio_path: &Path, name: &str, oversized: &mut bool) -> Option<ArtworkImage> {
    let audio_parent = audio_path.parent()?;
    let native_parent = to_extended_path(audio_parent).ok()?;
    let canonical_parent = fs::canonicalize(&native_parent).ok()?;
    let candidate = native_parent.join(name);
    let mut file = open_regular_file_without_following_reparse_points(&candidate)?;

    // Recheck the resolved candidate parent so a sidecar symlink cannot redirect an image read
    // to an arbitrary file outside the audio file's directory.
    let canonical_candidate = fs::canonicalize(&candidate).ok()?;
    if canonical_candidate.parent() != Some(canonical_parent.as_path()) {
        return None;
    }

    let metadata = file.metadata().ok()?;
    if metadata.len() == 0 {
        return None;
    }
    if metadata.len() > MAX_ARTWORK_BYTES as u64 {
        *oversized = true;
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.by_ref()
        .take(MAX_ARTWORK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    image_from_raw_bytes(&bytes, oversized)
}

fn image_from_raw_bytes(bytes: &[u8], oversized: &mut bool) -> Option<ArtworkImage> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.len() > MAX_ARTWORK_BYTES {
        *oversized = true;
        return None;
    }
    let header = inspect_image_header(bytes, oversized)?;
    ArtworkImage::try_new(
        header.mime_type,
        bytes.to_vec(),
        header.width,
        header.height,
    )
}

/// Inspect dimensions and container structure without decoding or rewriting any pixels.
fn inspect_image_header(bytes: &[u8], oversized: &mut bool) -> Option<ImageHeader> {
    let header = if bytes.starts_with(&[0xff, 0xd8]) {
        inspect_jpeg(bytes)?
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        inspect_png(bytes)?
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        inspect_gif(bytes)?
    } else if bytes.starts_with(b"BM") {
        inspect_bmp(bytes)?
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        inspect_webp_vp8x(bytes)?
    } else {
        return None;
    };

    let pixels = u64::from(header.width).checked_mul(u64::from(header.height))?;
    if header.width == 0 || header.height == 0 {
        return None;
    }
    if header.width > player_core::MAX_ARTWORK_DIMENSION
        || header.height > player_core::MAX_ARTWORK_DIMENSION
        || pixels > player_core::MAX_ARTWORK_PIXELS
    {
        *oversized = true;
        return None;
    }
    Some(header)
}

fn inspect_jpeg(bytes: &[u8]) -> Option<ImageHeader> {
    if !bytes.ends_with(&[0xff, 0xd9]) {
        return None;
    }
    let mut offset = 2_usize;
    while offset < bytes.len() {
        if bytes.get(offset) != Some(&0xff) {
            return None;
        }
        while bytes.get(offset) == Some(&0xff) {
            offset += 1;
        }
        let marker = *bytes.get(offset)?;
        offset += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }

        let segment_length = usize::from(u16::from_be_bytes([
            *bytes.get(offset)?,
            *bytes.get(offset + 1)?,
        ]));
        if segment_length < 2 || offset.checked_add(segment_length)? > bytes.len() {
            return None;
        }
        if is_jpeg_start_of_frame(marker) {
            if segment_length < 8 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([
                *bytes.get(offset + 3)?,
                *bytes.get(offset + 4)?,
            ]));
            let width = u32::from(u16::from_be_bytes([
                *bytes.get(offset + 5)?,
                *bytes.get(offset + 6)?,
            ]));
            return Some(ImageHeader {
                mime_type: "image/jpeg",
                width,
                height,
            });
        }
        offset += segment_length;
    }
    None
}

fn is_jpeg_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn inspect_png(bytes: &[u8]) -> Option<ImageHeader> {
    if bytes.len() < 8 {
        return None;
    }
    let mut offset = 8_usize;
    let mut dimensions = None;
    loop {
        let length = usize::try_from(u32::from_be_bytes(
            bytes.get(offset..offset + 4)?.try_into().ok()?,
        ))
        .ok()?;
        let kind = bytes.get(offset + 4..offset + 8)?;
        let data_start = offset.checked_add(8)?;
        let crc_start = data_start.checked_add(length)?;
        let next = crc_start.checked_add(4)?;
        if next > bytes.len() {
            return None;
        }
        match kind {
            b"IHDR" if dimensions.is_none() && length == 13 => {
                let width =
                    u32::from_be_bytes(bytes.get(data_start..data_start + 4)?.try_into().ok()?);
                let height =
                    u32::from_be_bytes(bytes.get(data_start + 4..data_start + 8)?.try_into().ok()?);
                dimensions = Some((width, height));
            }
            b"IEND" if length == 0 => {
                let (width, height) = dimensions?;
                return Some(ImageHeader {
                    mime_type: "image/png",
                    width,
                    height,
                });
            }
            _ => {}
        }
        offset = next;
    }
}

fn inspect_gif(bytes: &[u8]) -> Option<ImageHeader> {
    if bytes.len() < 14 || !bytes.ends_with(b";") {
        return None;
    }
    let width = u32::from(u16::from_le_bytes(bytes.get(6..8)?.try_into().ok()?));
    let height = u32::from(u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?));
    let global_color_table = bytes[10] & 0x80 != 0;
    let table_length = if global_color_table {
        3_usize.checked_mul(1_usize.checked_shl(u32::from((bytes[10] & 0x07) + 1))?)?
    } else {
        0
    };
    let first_block = 13_usize.checked_add(table_length)?;
    if first_block >= bytes.len() - 1
        || !bytes[first_block..bytes.len() - 1]
            .iter()
            .any(|byte| *byte == 0x2c || *byte == 0x21)
    {
        return None;
    }
    Some(ImageHeader {
        mime_type: "image/gif",
        width,
        height,
    })
}

fn inspect_bmp(bytes: &[u8]) -> Option<ImageHeader> {
    if bytes.len() < 54 {
        return None;
    }
    let file_size = u32::from_le_bytes(bytes.get(2..6)?.try_into().ok()?);
    let dib_size = u32::from_le_bytes(bytes.get(14..18)?.try_into().ok()?);
    if file_size < 54 || usize::try_from(file_size).ok()? > bytes.len() || dib_size < 40 {
        return None;
    }
    let width = i32::from_le_bytes(bytes.get(18..22)?.try_into().ok()?);
    let height = i32::from_le_bytes(bytes.get(22..26)?.try_into().ok()?);
    Some(ImageHeader {
        mime_type: "image/bmp",
        width: u32::try_from(width).ok()?,
        height: height
            .checked_abs()
            .and_then(|value| u32::try_from(value).ok())?,
    })
}

fn inspect_webp_vp8x(bytes: &[u8]) -> Option<ImageHeader> {
    if bytes.len() < 30 || bytes.get(12..16)? != b"VP8X" {
        return None;
    }
    let chunk_length = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?);
    if chunk_length < 10 || 20_u32.checked_add(chunk_length)? > u32::try_from(bytes.len()).ok()? {
        return None;
    }
    let width = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
    let height = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
    Some(ImageHeader {
        mime_type: "image/webp",
        width,
        height,
    })
}

fn open_regular_file_without_following_reparse_points(path: &Path) -> Option<File> {
    let initial_metadata = fs::symlink_metadata(path).ok()?;
    if !initial_metadata.is_file()
        || initial_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let opened_metadata = file.metadata().ok()?;
    if !opened_metadata.is_file()
        || opened_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return None;
    }
    Some(file)
}

trait WindowsFileMetadataExt {
    fn file_attributes(&self) -> u32;
}

impl WindowsFileMetadataExt for std::fs::Metadata {
    fn file_attributes(&self) -> u32 {
        use std::os::windows::fs::MetadataExt;
        MetadataExt::file_attributes(self)
    }
}

trait WindowsOpenOptionsExt {
    fn custom_flags(&mut self, flags: u32) -> &mut Self;
}

impl WindowsOpenOptionsExt for OpenOptions {
    fn custom_flags(&mut self, flags: u32) -> &mut Self {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptionsExt::custom_flags(self, flags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moe-artwork-{label}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn tiny_png() -> Vec<u8> {
        // Valid, decodable 1x1 RGB PNG fixture (including its compressed pixel data).
        vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0,
            0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x90, 0x77, 0x53, 0xde, 0, 0, 0, 11, b'I', b'D',
            b'A', b'T', 0x78, 0x9c, 0x63, 0x60, 0x00, 0x02, 0, 0, 0x05, 0, 1, 0xa5, 0xf6, 0x45,
            0x40, 0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82,
        ]
    }

    fn write_tagged_mp3(path: &Path, embedded: &[u8]) {
        let mut apic = vec![0]; // ISO-8859-1 text encoding
        apic.extend_from_slice(b"image/png\0");
        apic.push(3); // front cover
        apic.push(0); // empty description
        apic.extend_from_slice(embedded);

        let mut tag = b"APIC".to_vec();
        let frame_len = u32::try_from(apic.len()).expect("small fixture image");
        tag.extend_from_slice(&[
            ((frame_len >> 21) & 0x7f) as u8,
            ((frame_len >> 14) & 0x7f) as u8,
            ((frame_len >> 7) & 0x7f) as u8,
            (frame_len & 0x7f) as u8,
        ]);
        tag.extend_from_slice(&[0, 0]);
        tag.extend_from_slice(&apic);

        let tag_len = u32::try_from(tag.len()).expect("small fixture tag");
        let mut bytes = b"ID3\x03\0\0".to_vec();
        bytes.extend_from_slice(&[
            ((tag_len >> 21) & 0x7f) as u8,
            ((tag_len >> 14) & 0x7f) as u8,
            ((tag_len >> 7) & 0x7f) as u8,
            (tag_len & 0x7f) as u8,
        ]);
        bytes.extend_from_slice(&tag);
        fs::write(path, bytes).expect("write synthetic ID3v2 picture");
    }

    fn only_locator(path: PathBuf) -> Vec<MediaLocator> {
        vec![MediaLocator::FileSystem(path)]
    }

    fn expect_found(locators: &[MediaLocator]) -> ArtworkImage {
        match find_artwork(locators) {
            ArtworkLookup::Found(image) => image,
            other => panic!("expected artwork, got {other:?}"),
        }
    }

    #[test]
    fn embedded_cover_is_returned_unchanged_before_sidecars() {
        let dir = TestDirectory::new("embedded");
        let audio = dir.path().join("track.mp3");
        let embedded = tiny_png();
        write_tagged_mp3(&audio, &embedded);
        fs::write(dir.path().join("cover.jpg"), tiny_png()).expect("sidecar");

        let artwork = expect_found(&only_locator(audio));
        assert_eq!(artwork.mime_type(), "image/png");
        assert_eq!(artwork.bytes(), embedded);
        assert_eq!(artwork.dimensions(), (1, 1));
    }

    #[test]
    fn corrupt_or_unsupported_embedded_picture_falls_back_to_cover_then_folder() {
        let dir = TestDirectory::new("corrupt-embedded");
        let audio = dir.path().join("track.mp3");
        write_tagged_mp3(&audio, b"not an image");
        let cover = tiny_png();
        fs::write(dir.path().join("cover.jpg"), &cover).expect("write cover fallback");
        fs::write(dir.path().join("folder.jpg"), tiny_png()).expect("write folder fallback");

        let artwork = expect_found(&only_locator(audio.clone()));
        assert_eq!(artwork.bytes(), cover);

        fs::write(dir.path().join("cover.jpg"), b"truncated").expect("corrupt cover");
        let artwork = expect_found(&only_locator(audio));
        assert_eq!(artwork.bytes(), tiny_png());
    }

    #[test]
    fn missing_cover_returns_missing_and_content_uri_is_not_opened() {
        let dir = TestDirectory::new("missing");
        let audio = dir.path().join("track.mp3");
        fs::write(&audio, b"invalid audio").expect("audio placeholder");
        let locators = vec![
            MediaLocator::ContentUri("content://no/path/access".to_owned()),
            MediaLocator::FileSystem(audio),
        ];
        assert_eq!(find_artwork(&locators), ArtworkLookup::Missing);
    }

    #[test]
    fn unicode_path_and_sidecar_bytes_are_preserved() {
        let dir = TestDirectory::new("unicode");
        let unicode = dir.path().join("音樂🎧 日本語 かな");
        fs::create_dir_all(&unicode).expect("unicode directory");
        let audio = unicode.join("歌曲 - 夜空.mp3");
        fs::write(&audio, b"invalid audio").expect("audio placeholder");
        let cover = tiny_png();
        fs::write(unicode.join("cover.jpg"), &cover).expect("unicode path sidecar");

        let artwork = expect_found(&only_locator(audio));
        assert_eq!(artwork.bytes(), cover);
        assert_eq!(artwork.dimensions(), (1, 1));
    }

    #[test]
    fn oversized_sidecar_is_reported_without_reading_full_file() {
        let dir = TestDirectory::new("oversize");
        let audio = dir.path().join("track.mp3");
        fs::write(&audio, b"invalid audio").expect("audio placeholder");
        let sidecar = dir.path().join("cover.jpg");
        let file = fs::File::create(&sidecar).expect("create sparse sidecar");
        file.set_len(MAX_ARTWORK_BYTES as u64 + 1)
            .expect("make oversize sidecar");

        assert_eq!(find_artwork(&only_locator(audio)), ArtworkLookup::Oversized);
    }

    #[test]
    fn oversized_embedded_image_falls_back_to_valid_sidecar() {
        let dir = TestDirectory::new("oversize-embedded-fallback");
        let audio = dir.path().join("track.mp3");
        let mut oversized_embedded = tiny_png();
        oversized_embedded[16..20].copy_from_slice(&20_000_u32.to_be_bytes());
        oversized_embedded[20..24].copy_from_slice(&20_000_u32.to_be_bytes());
        write_tagged_mp3(&audio, &oversized_embedded);
        let sidecar = tiny_png();
        fs::write(dir.path().join("cover.jpg"), &sidecar).expect("write sidecar");

        let artwork = expect_found(&only_locator(audio));
        assert_eq!(artwork.bytes(), sidecar);
    }

    #[test]
    fn symlink_sidecar_is_rejected() {
        let dir = TestDirectory::new("symlink");
        let audio = dir.path().join("track.mp3");
        fs::write(&audio, b"invalid audio").expect("audio placeholder");
        let outside = dir.path().join("outside.png");
        fs::write(&outside, tiny_png()).expect("outside image");
        let sidecar = dir.path().join("cover.jpg");
        if std::os::windows::fs::symlink_file(&outside, &sidecar).is_err() {
            // Windows can disable symlink creation for a non-elevated test process.
            return;
        }

        assert_eq!(find_artwork(&only_locator(audio)), ArtworkLookup::Missing);
    }

    #[test]
    fn long_path_uses_extended_windows_path() {
        let dir = TestDirectory::new("long");
        let mut long_dir = dir.path().to_path_buf();
        for index in 0..5 {
            long_dir.push(format!("資料夾-{index}-{}", "長路徑".repeat(10)));
        }
        let native_long_dir = to_extended_path(&long_dir).expect("extended long path");
        fs::create_dir_all(&native_long_dir).expect("create long path");
        let audio = native_long_dir.join("歌曲🎵.mp3");
        fs::write(&audio, b"invalid audio").expect("long-path audio");
        let cover = tiny_png();
        fs::write(native_long_dir.join("cover.jpg"), &cover).expect("long-path sidecar");

        let artwork = expect_found(&only_locator(audio));
        assert_eq!(artwork.bytes(), cover);
    }

    #[test]
    fn unsupported_and_corrupt_headers_are_skipped() {
        let mut oversized = false;
        assert!(image_from_raw_bytes(b"not an image", &mut oversized).is_none());
        assert!(!oversized);
        assert!(image_from_raw_bytes(b"\x89PNG\r\n\x1a\nshort", &mut oversized).is_none());
        assert!(!oversized);
        let mut too_large = tiny_png();
        too_large[16..20].copy_from_slice(&20_000_u32.to_be_bytes());
        too_large[20..24].copy_from_slice(&20_000_u32.to_be_bytes());
        assert!(inspect_image_header(&too_large, &mut oversized).is_none());
        assert!(oversized);
    }
}
