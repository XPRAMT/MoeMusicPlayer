use player_core::{ArtworkImage, MAX_ARTWORK_BYTES};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ArtworkError {
    Missing,
    Oversized,
    Invalid,
}

/// Read bounds from the compressed image header without decoding or re-encoding pixels.
pub(crate) fn image_from_bytes(
    bytes: &[u8],
    reported_mime: Option<&str>,
) -> Result<ArtworkImage, ArtworkError> {
    if bytes.len() > MAX_ARTWORK_BYTES {
        return Err(ArtworkError::Oversized);
    }
    if bytes.is_empty() {
        return Err(ArtworkError::Missing);
    }
    let mime_type = sniff_mime_type(bytes).ok_or(ArtworkError::Invalid)?;
    if reported_mime.is_some_and(|mime| mime.starts_with("image/") && mime != mime_type) {
        return Err(ArtworkError::Invalid);
    }
    let dimensions = imagesize::blob_size(bytes).map_err(|_| ArtworkError::Invalid)?;
    let width = u32::try_from(dimensions.width).map_err(|_| ArtworkError::Oversized)?;
    let height = u32::try_from(dimensions.height).map_err(|_| ArtworkError::Oversized)?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ArtworkError::Oversized)?;
    if width == 0
        || height == 0
        || width > player_core::MAX_ARTWORK_DIMENSION
        || height > player_core::MAX_ARTWORK_DIMENSION
        || pixels > player_core::MAX_ARTWORK_PIXELS
    {
        return Err(ArtworkError::Oversized);
    }
    ArtworkImage::try_new(mime_type, bytes.to_vec(), width, height).ok_or(ArtworkError::Oversized)
}

/// The cover embedded in a real-path file's tags, read by the reader every cover lookup shares (DSF's trailing ID3v2 tag,
/// APEv2 covers, lofty's tags). `Missing` when the file has none.
pub(crate) fn embedded_image_of_path(path: &std::path::Path) -> Result<ArtworkImage, ArtworkError> {
    let embedded =
        player_tags::embedded::read_embedded_tags_of_path(path).ok_or(ArtworkError::Missing)?;
    let mut failure = ArtworkError::Missing;
    for picture in &embedded.pictures {
        match image_from_bytes(
            picture.data(),
            picture.mime_type().map(|mime| mime.as_str()),
        ) {
            Ok(image) => return Ok(image),
            Err(ArtworkError::Oversized) => failure = ArtworkError::Oversized,
            Err(_) => {}
        }
    }
    Err(failure)
}

fn sniff_mime_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_png() -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.extend_from_slice(&1_u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(b"IEND");
        bytes.extend_from_slice(&[0; 4]);
        bytes
    }

    #[test]
    fn android_artwork_keeps_original_bytes_and_checks_mime_and_dimensions() {
        let bytes = tiny_png();
        let image = image_from_bytes(&bytes, Some("image/png")).expect("valid PNG header");
        assert_eq!(image.bytes(), bytes);
        assert_eq!(image.mime_type(), "image/png");
        assert_eq!(image.dimensions(), (1, 1));
        assert_eq!(
            image_from_bytes(&bytes, Some("image/jpeg")),
            Err(ArtworkError::Invalid)
        );
    }

    #[test]
    fn android_artwork_classifies_size_limits_and_accepts_jpeg_trailing_metadata() {
        assert_eq!(image_from_bytes(&[], None), Err(ArtworkError::Missing));
        assert_eq!(
            image_from_bytes(&vec![0; MAX_ARTWORK_BYTES + 1], Some("image/png")),
            Err(ArtworkError::Oversized)
        );
        assert_eq!(
            image_from_bytes(b"not an image", None),
            Err(ArtworkError::Invalid)
        );

        // JPEG readers must accept harmless bytes after EOI (for example trailing metadata).
        let jpeg = [
            0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
            0x00, 0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9, 0x45, 0x58, 0x49, 0x46,
        ];
        let image = image_from_bytes(&jpeg, Some("image/jpeg")).expect("JPEG with trailing bytes");
        assert_eq!(image.bytes(), jpeg);
        assert_eq!(image.dimensions(), (1, 1));
    }

    #[test]
    fn android_artwork_rejects_oversized_dimensions_and_accepts_all_webp_bitstreams() {
        let mut oversized_png = tiny_png();
        oversized_png[16..20].copy_from_slice(&16_385_u32.to_be_bytes());
        assert_eq!(
            image_from_bytes(&oversized_png, Some("image/png")),
            Err(ArtworkError::Oversized)
        );

        let make_webp = |chunk: &[u8], payload: &[u8]| {
            let mut bytes = b"RIFF".to_vec();
            bytes.extend_from_slice(&((4 + chunk.len() + 4 + payload.len()) as u32).to_le_bytes());
            bytes.extend_from_slice(b"WEBP");
            bytes.extend_from_slice(chunk);
            bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            bytes.extend_from_slice(payload);
            bytes
        };

        // VP8X has three-byte little-endian width/height minus one fields.
        let vp8x = make_webp(b"VP8X", &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            image_from_bytes(&vp8x, Some("image/webp"))
                .unwrap()
                .dimensions(),
            (1, 1)
        );

        // VP8L stores packed fourteen-bit width/height minus one fields after signature 0x2f.
        let vp8l = make_webp(b"VP8L", &[0x2f, 0, 0, 0, 0]);
        assert_eq!(
            image_from_bytes(&vp8l, Some("image/webp"))
                .unwrap()
                .dimensions(),
            (1, 1)
        );

        // Lossy VP8 carries a 14-bit width and height in its frame header.
        let vp8 = make_webp(b"VP8 ", &[0, 0, 0, 0, 0, 0, 1, 0, 1, 0]);
        assert_eq!(
            image_from_bytes(&vp8, Some("image/webp"))
                .unwrap()
                .dimensions(),
            (1, 1)
        );
    }

    /// A local DSF keeps its cover in the ID3v2 tag at the end of the file: the Android local lookup must find it the way
    /// the remote one does (it used to ask only the platform's media retriever, which cannot read that tag).
    #[test]
    fn a_local_dsf_cover_is_found_and_matches_the_remote_reader() {
        use lofty::config::WriteOptions;
        use lofty::picture::{MimeType, Picture, PictureType};
        use lofty::tag::{Tag, TagExt, TagType};

        let png = tiny_png();
        let mut file =
            player_dsd::testing::build_dsf(2_822_400, 8_000, 4096, true, &[vec![0x69; 1000]]);
        let offset = file.len() as u64;
        file[20..28].copy_from_slice(&offset.to_le_bytes());
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(
            Picture::unchecked(png.clone())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.dump_to(&mut file, WriteOptions::default()).unwrap();
        let directory =
            std::env::temp_dir().join(format!("moe-android-art-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("track.dsf");
        std::fs::write(&path, &file).unwrap();

        let local = embedded_image_of_path(&path).expect("the embedded cover of a local DSF");
        assert_eq!(local.bytes(), png);
        assert_eq!(local.mime_type(), "image/png");
        let remote = player_tags::embedded::read_embedded_tags(std::io::Cursor::new(file), &path)
            .and_then(|tags| tags.front_cover().map(|picture| picture.data().to_vec()));
        assert_eq!(remote.as_deref(), Some(png.as_slice()));

        std::fs::write(
            &path,
            player_dsd::testing::build_dsf(2_822_400, 8_000, 4096, true, &[vec![0; 8]]),
        )
        .unwrap();
        assert_eq!(embedded_image_of_path(&path), Err(ArtworkError::Missing));
        let _ = std::fs::remove_dir_all(directory);
    }
}
