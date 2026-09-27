//! Bounded cover-art payload shared by the platform artwork readers.

/// Maximum compressed image payload returned for one current track.
///
/// The original image bytes and dimensions are preserved. Images larger than this limit are
/// skipped so a single IPC response cannot grow without bound.
pub const MAX_ARTWORK_BYTES: usize = 32 * 1024 * 1024;

/// Maximum width or height accepted from an image header.
pub const MAX_ARTWORK_DIMENSION: u32 = 16_384;

/// Maximum pixel count accepted from an image header.
pub const MAX_ARTWORK_PIXELS: u64 = 64 * 1024 * 1024;

/// One on-demand cover image. The payload remains in its original encoded form.
///
/// This is deliberately not `Serialize`: callers should return the bytes through a binary IPC
/// response rather than JSON-encoding a potentially large byte array.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtworkImage {
    mime_type: &'static str,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

impl ArtworkImage {
    /// Construct a validated, bounded cover-art payload.
    pub fn try_new(
        mime_type: &'static str,
        bytes: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let pixels = u64::from(width).checked_mul(u64::from(height))?;
        if !matches!(
            mime_type,
            "image/jpeg" | "image/png" | "image/gif" | "image/bmp" | "image/webp"
        ) || bytes.is_empty()
            || bytes.len() > MAX_ARTWORK_BYTES
            || width == 0
            || height == 0
            || width > MAX_ARTWORK_DIMENSION
            || height > MAX_ARTWORK_DIMENSION
            || pixels > MAX_ARTWORK_PIXELS
        {
            return None;
        }

        Some(Self {
            mime_type,
            bytes,
            width,
            height,
        })
    }

    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_preserves_original_bytes_and_dimensions_within_limits() {
        let bytes = vec![0xA5; 19];
        let image = ArtworkImage::try_new("image/jpeg", bytes.clone(), 6000, 4000)
            .expect("normal original-resolution image is accepted");

        assert_eq!(image.mime_type(), "image/jpeg");
        assert_eq!(image.bytes(), bytes);
        assert_eq!(image.dimensions(), (6000, 4000));
    }

    #[test]
    fn payload_rejects_oversize_or_excessive_dimensions_without_resizing() {
        assert!(
            ArtworkImage::try_new("image/jpeg", vec![0; MAX_ARTWORK_BYTES + 1], 100, 100,)
                .is_none()
        );
        assert!(
            ArtworkImage::try_new("image/png", vec![1], MAX_ARTWORK_DIMENSION + 1, 1).is_none()
        );
        assert!(ArtworkImage::try_new("image/png", vec![1], 8193, 8192).is_none());
        assert!(ArtworkImage::try_new("image/tiff", vec![1], 1, 1).is_none());
    }
}
