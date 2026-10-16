use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage, GenericImageView};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverOptions {
    /// JPEG quality 1–100 used whenever the cover is re-encoded.
    pub jpeg_quality: u8,
    /// When enabled, covers whose short edge exceeds `short_edge_px` are scaled down
    /// (aspect ratio always preserved). Never upscales.
    pub auto_resize: bool,
    pub short_edge_px: u32,
}

impl Default for CoverOptions {
    fn default() -> Self {
        Self {
            jpeg_quality: 100,
            auto_resize: false,
            short_edge_px: 2160,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedCover {
    pub data: Vec<u8>,
    pub mime: &'static str,
    pub width: u32,
    pub height: u32,
}

/// Target size keeping aspect ratio so that the short edge equals `short_edge`.
pub fn fit_short_edge(width: u32, height: u32, short_edge: u32) -> (u32, u32) {
    let short = width.min(height);
    if short_edge == 0 || short <= short_edge {
        return (width, height);
    }
    let scale = f64::from(short_edge) / f64::from(short);
    let w = ((f64::from(width) * scale).round() as u32).max(1);
    let h = ((f64::from(height) * scale).round() as u32).max(1);
    if width <= height {
        (short_edge, h)
    } else {
        (w, short_edge)
    }
}

/// Prepare replacement cover bytes. A JPEG that needs no resize is kept byte-for-byte
/// (no generation loss); anything else is (resized and) re-encoded as JPEG at the
/// configured quality.
pub fn prepare_cover(input: &[u8], options: CoverOptions) -> Result<PreparedCover, String> {
    let format = image::guess_format(input).map_err(|e| format!("無法辨識圖片格式：{e}"))?;
    let img = image::load_from_memory(input).map_err(|e| format!("無法解碼圖片：{e}"))?;
    let (w, h) = img.dimensions();
    let (tw, th) = if options.auto_resize {
        fit_short_edge(w, h, options.short_edge_px)
    } else {
        (w, h)
    };
    if format == image::ImageFormat::Jpeg && (tw, th) == (w, h) {
        return Ok(PreparedCover {
            data: input.to_vec(),
            mime: "image/jpeg",
            width: w,
            height: h,
        });
    }
    let resized = if (tw, th) == (w, h) {
        img
    } else {
        img.resize_exact(tw, th, FilterType::Lanczos3)
    };
    let rgb = DynamicImage::ImageRgb8(resized.to_rgb8());
    let mut out = Vec::new();
    let quality = options.jpeg_quality.clamp(1, 100);
    JpegEncoder::new_with_quality(&mut out, quality)
        .encode_image(&rgb)
        .map_err(|e| format!("JPEG 編碼失敗：{e}"))?;
    Ok(PreparedCover {
        data: out,
        mime: "image/jpeg",
        width: tw,
        height: th,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_edge_keeps_aspect_ratio_and_never_upscales() {
        assert_eq!(fit_short_edge(2000, 1000, 500), (1000, 500));
        assert_eq!(fit_short_edge(1000, 3000, 600), (600, 1800));
        assert_eq!(fit_short_edge(400, 300, 500), (400, 300));
    }

    #[test]
    fn png_is_reencoded_as_jpeg_with_resize() {
        let img = DynamicImage::new_rgb8(800, 400);
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let out = prepare_cover(
            &png,
            CoverOptions {
                jpeg_quality: 80,
                auto_resize: true,
                short_edge_px: 200,
            },
        )
        .unwrap();
        assert_eq!((out.width, out.height), (400, 200));
        assert_eq!(
            image::guess_format(&out.data).unwrap(),
            image::ImageFormat::Jpeg
        );
    }
}
