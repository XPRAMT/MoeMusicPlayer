use std::io::Cursor;

use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    imageops::FilterType,
    metadata::Orientation,
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverOptions {
    /// JPEG quality 1–100 used whenever the cover is re-encoded as JPEG.
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

/// What `prepare_cover` will do with an image (also shown on the details page as 「儲存時：…」).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoverPlan {
    pub width: u32,
    pub height: u32,
    /// "image/jpeg" or "image/png".
    pub mime: &'static str,
    /// false = the original bytes are written unchanged.
    pub reencode: bool,
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

struct Decoded {
    format: ImageFormat,
    image: DynamicImage,
    orientation: Orientation,
    icc: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
}

fn decode(input: &[u8]) -> Result<Decoded, String> {
    let format = image::guess_format(input).map_err(|e| format!("無法辨識圖片格式：{e}"))?;
    let reader = ImageReader::with_format(Cursor::new(input), format);
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| format!("無法解碼圖片：{e}"))?;
    let icc = decoder.icc_profile().ok().flatten().filter(|v| !v.is_empty());
    let exif = decoder.exif_metadata().ok().flatten().filter(|v| !v.is_empty());
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let image = DynamicImage::from_decoder(decoder).map_err(|e| format!("無法解碼圖片：{e}"))?;
    Ok(Decoded { format, image, orientation, icc, exif })
}

fn oriented_size(image: &DynamicImage, orientation: Orientation) -> (u32, u32) {
    let (w, h) = (image.width(), image.height());
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (h, w),
        _ => (w, h),
    }
}

/// True when at least one pixel has alpha < max (an alpha channel that is fully opaque does not count).
fn has_transparency(image: &DynamicImage) -> bool {
    match image {
        DynamicImage::ImageLumaA8(i) => i.pixels().any(|p| p[1] < u8::MAX),
        DynamicImage::ImageRgba8(i) => i.pixels().any(|p| p[3] < u8::MAX),
        DynamicImage::ImageLumaA16(i) => i.pixels().any(|p| p[1] < u16::MAX),
        DynamicImage::ImageRgba16(i) => i.pixels().any(|p| p[3] < u16::MAX),
        DynamicImage::ImageRgba32F(i) => i.pixels().any(|p| p[3] < 1.0),
        other if other.color().has_alpha() => other.to_rgba8().pixels().any(|p| p[3] < u8::MAX),
        _ => false,
    }
}

/// The rule (user decision 2026-10-22): every cover is written as JPG, except an image with at
/// least one transparent pixel (alpha < 255), which is written as PNG with its transparency. An unscaled JPG — and an unscaled
/// PNG with alpha — keeps its original bytes.
fn plan_for(decoded: &Decoded, options: CoverOptions) -> CoverPlan {
    let (w, h) = oriented_size(&decoded.image, decoded.orientation);
    let (tw, th) = if options.auto_resize {
        fit_short_edge(w, h, options.short_edge_px)
    } else {
        (w, h)
    };
    let resized = (tw, th) != (w, h);
    let alpha = has_transparency(&decoded.image);
    let mime = if alpha { "image/png" } else { "image/jpeg" };
    let keep = !resized
        && match decoded.format {
            ImageFormat::Jpeg => !alpha,
            ImageFormat::Png => alpha,
            _ => false,
        };
    CoverPlan { width: tw, height: th, mime, reencode: !keep }
}

/// Describe how `prepare_cover` would write this image, without encoding it.
pub fn plan_cover(input: &[u8], options: CoverOptions) -> Result<CoverPlan, String> {
    Ok(plan_for(&decode(input)?, options))
}

/// Prepare replacement cover bytes per `plan_for`. When re-encoding, the EXIF orientation is applied
/// to the pixels and the orientation tag in the kept EXIF is reset to 1 (so it is not rotated twice);
/// the ICC profile and the rest of the EXIF are written to the new JPG/PNG.
pub fn prepare_cover(input: &[u8], options: CoverOptions) -> Result<PreparedCover, String> {
    let decoded = decode(input)?;
    let plan = plan_for(&decoded, options);
    if !plan.reencode {
        return Ok(PreparedCover {
            data: input.to_vec(),
            mime: plan.mime,
            width: plan.width,
            height: plan.height,
        });
    }
    let Decoded { mut image, orientation, icc, mut exif, .. } = decoded;
    image.apply_orientation(orientation);
    if let Some(chunk) = exif.as_mut() {
        let _ = Orientation::remove_from_exif_chunk(chunk);
    }
    if (image.width(), image.height()) != (plan.width, plan.height) {
        image = image.resize_exact(plan.width, plan.height, FilterType::Lanczos3);
    }
    let mut out = Vec::new();
    if plan.mime == "image/png" {
        let image = match image {
            DynamicImage::ImageLumaA8(_)
            | DynamicImage::ImageRgba8(_)
            | DynamicImage::ImageLumaA16(_)
            | DynamicImage::ImageRgba16(_) => image,
            DynamicImage::ImageRgba32F(_) => DynamicImage::ImageRgba16(image.to_rgba16()),
            other => DynamicImage::ImageRgba8(other.to_rgba8()),
        };
        let mut encoder = PngEncoder::new(&mut out);
        attach_metadata(&mut encoder, icc, exif);
        let color: ExtendedColorType = image.color().into();
        encoder
            .write_image(image.as_bytes(), image.width(), image.height(), color)
            .map_err(|e| format!("PNG 編碼失敗：{e}"))?;
    } else {
        let rgb = image.to_rgb8();
        let mut encoder = JpegEncoder::new_with_quality(&mut out, options.jpeg_quality.clamp(1, 100));
        attach_metadata(&mut encoder, icc, exif);
        encoder
            .write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG 編碼失敗：{e}"))?;
    }
    Ok(PreparedCover {
        data: out,
        mime: plan.mime,
        width: plan.width,
        height: plan.height,
    })
}

fn attach_metadata<E: ImageEncoder>(encoder: &mut E, icc: Option<Vec<u8>>, exif: Option<Vec<u8>>) {
    // Both encoders accept ICC and EXIF in image 0.25.10; an unsupported one just drops it.
    if let Some(icc) = icc {
        let _ = encoder.set_icc_profile(icc);
    }
    if let Some(exif) = exif {
        let _ = encoder.set_exif_metadata(exif);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::webp::WebPEncoder;
    use image::{Rgb, RgbImage, Rgba, RgbaImage};

    const OPTS_RESIZE: CoverOptions = CoverOptions { jpeg_quality: 80, auto_resize: true, short_edge_px: 100 };
    const OPTS_KEEP: CoverOptions = CoverOptions { jpeg_quality: 80, auto_resize: false, short_edge_px: 2160 };

    /// Minimal little-endian EXIF (TIFF) chunk: Orientation (0x0112) and Artist (0x013B, "Moe").
    pub(crate) fn exif_chunk(orientation: u16) -> Vec<u8> {
        let mut v = b"II*\0".to_vec();
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&0x0112u16.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&orientation.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&0x013Bu16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&4u32.to_le_bytes());
        v.extend_from_slice(b"Moe\0");
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    pub(crate) fn fake_icc() -> Vec<u8> {
        let mut icc = vec![0u8; 132];
        icc[..4].copy_from_slice(&132u32.to_be_bytes());
        icc[36..40].copy_from_slice(b"acsp");
        icc[100..112].copy_from_slice(b"moe-icc-test");
        icc
    }

    fn encode(img: &DynamicImage, format: ImageFormat, icc: Option<Vec<u8>>, exif: Option<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        let (w, h) = (img.width(), img.height());
        let color: ExtendedColorType = img.color().into();
        match format {
            ImageFormat::Png => {
                let mut e = PngEncoder::new(&mut out);
                attach_metadata(&mut e, icc, exif);
                e.write_image(img.as_bytes(), w, h, color).unwrap();
            }
            ImageFormat::Jpeg => {
                let rgb = img.to_rgb8();
                let mut e = JpegEncoder::new_with_quality(&mut out, 90);
                attach_metadata(&mut e, icc, exif);
                e.write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8).unwrap();
            }
            ImageFormat::WebP => {
                let mut e = WebPEncoder::new_lossless(&mut out);
                attach_metadata(&mut e, icc, exif);
                e.write_image(img.as_bytes(), w, h, color).unwrap();
            }
            other => img.write_to(&mut Cursor::new(&mut out), other).unwrap(),
        }
        out
    }

    fn opaque(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(w, h, |x, _| Rgb([(x % 256) as u8, 120, 200])))
    }

    /// Left half fully transparent, right half opaque red.
    fn transparent(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(w, h, |x, _| {
            if x < w / 2 { Rgba([0, 0, 0, 0]) } else { Rgba([255, 0, 0, 255]) }
        }))
    }

    fn metadata_of(bytes: &[u8]) -> (Option<Vec<u8>>, Option<Vec<u8>>, Orientation) {
        let d = decode(bytes).unwrap();
        (d.icc, d.exif, d.orientation)
    }

    #[test]
    fn short_edge_keeps_aspect_ratio_and_never_upscales() {
        assert_eq!(fit_short_edge(2000, 1000, 500), (1000, 500));
        assert_eq!(fit_short_edge(1000, 3000, 600), (600, 1800));
        assert_eq!(fit_short_edge(400, 300, 500), (400, 300));
    }

    #[test]
    fn unscaled_jpg_keeps_original_bytes() {
        let jpg = encode(&opaque(300, 200), ImageFormat::Jpeg, Some(fake_icc()), Some(exif_chunk(6)));
        let out = prepare_cover(&jpg, OPTS_KEEP).unwrap();
        assert_eq!(out.data, jpg);
        assert_eq!(out.mime, "image/jpeg");
        assert_eq!((out.width, out.height), (200, 300), "size reported after orientation");
    }

    #[test]
    fn unscaled_png_with_alpha_keeps_original_bytes() {
        let png = encode(&transparent(64, 64), ImageFormat::Png, None, None);
        let out = prepare_cover(&png, OPTS_KEEP).unwrap();
        assert_eq!(out.data, png);
        assert_eq!(out.mime, "image/png");
    }

    #[test]
    fn opaque_png_webp_bmp_gif_become_jpg() {
        for format in [ImageFormat::Png, ImageFormat::WebP, ImageFormat::Bmp, ImageFormat::Gif] {
            let src = encode(&opaque(120, 80), format, None, None);
            let out = prepare_cover(&src, OPTS_KEEP).unwrap();
            assert_eq!(out.mime, "image/jpeg", "{format:?}");
            assert_eq!(image::guess_format(&out.data).unwrap(), ImageFormat::Jpeg, "{format:?}");
            assert_eq!((out.width, out.height), (120, 80));
        }
    }

    #[test]
    fn fully_opaque_alpha_channel_becomes_jpg_with_metadata() {
        let rgba = DynamicImage::ImageRgba8(RgbaImage::from_pixel(60, 40, Rgba([10, 20, 30, 255])));
        for format in [ImageFormat::Png, ImageFormat::WebP] {
            let src = encode(&rgba, format, Some(fake_icc()), Some(exif_chunk(1)));
            assert!(image::load_from_memory(&src).unwrap().color().has_alpha());
            let out = prepare_cover(&src, OPTS_KEEP).unwrap();
            assert_eq!(out.mime, "image/jpeg", "{format:?}");
            let (icc, exif, _) = metadata_of(&out.data);
            assert_eq!(icc.as_deref(), Some(fake_icc().as_slice()), "{format:?}");
            assert!(exif.unwrap().windows(3).any(|w| w == b"Moe"), "{format:?}");
        }
        assert!(!has_transparency(&rgba));
        let mut one = rgba.to_rgba8();
        one.put_pixel(59, 39, Rgba([0, 0, 0, 254]));
        assert!(has_transparency(&DynamicImage::ImageRgba8(one)));
    }

    #[test]
    fn alpha_sources_become_png_and_keep_transparency() {
        for (format, opts) in [
            (ImageFormat::WebP, OPTS_KEEP),
            (ImageFormat::WebP, OPTS_RESIZE),
            (ImageFormat::Png, OPTS_RESIZE),
        ] {
            let src = encode(&transparent(400, 200), format, None, None);
            let out = prepare_cover(&src, opts).unwrap();
            assert_eq!(out.mime, "image/png", "{format:?}");
            let img = image::load_from_memory(&out.data).unwrap().to_rgba8();
            assert_eq!(img.get_pixel(1, 1)[3], 0, "transparent stays transparent, no black fill");
            assert_eq!(img.get_pixel(img.width() - 2, 1).0, [255, 0, 0, 255]);
        }
    }

    #[test]
    fn scaled_jpg_stays_jpg_and_keeps_icc_and_exif_with_orientation_baked_in() {
        // 300×200 stored, orientation 6 (rotate 90° CW) → shown 200×300; short edge 100 → 100×150.
        let src = encode(&opaque(300, 200), ImageFormat::Jpeg, Some(fake_icc()), Some(exif_chunk(6)));
        let out = prepare_cover(&src, OPTS_RESIZE).unwrap();
        assert_eq!(out.mime, "image/jpeg");
        let img = image::load_from_memory(&out.data).unwrap();
        assert_eq!((img.width(), img.height()), (100, 150));
        let (icc, exif, orientation) = metadata_of(&out.data);
        assert_eq!(icc.as_deref(), Some(fake_icc().as_slice()));
        let exif = exif.expect("EXIF kept");
        assert!(exif.windows(3).any(|w| w == b"Moe"), "other EXIF fields kept");
        assert_eq!(Orientation::from_exif_chunk(&exif), Some(Orientation::NoTransforms));
        assert_eq!(orientation, Orientation::NoTransforms);
    }

    #[test]
    fn png_with_alpha_scaled_keeps_icc_and_exif() {
        let src = encode(&transparent(400, 200), ImageFormat::Png, Some(fake_icc()), Some(exif_chunk(1)));
        let out = prepare_cover(&src, OPTS_RESIZE).unwrap();
        assert_eq!(out.mime, "image/png");
        assert_eq!((out.width, out.height), (200, 100));
        let (icc, exif, _) = metadata_of(&out.data);
        assert_eq!(icc.as_deref(), Some(fake_icc().as_slice()));
        assert!(exif.unwrap().windows(3).any(|w| w == b"Moe"));
    }

    #[test]
    fn opaque_webp_to_jpg_keeps_icc_and_exif() {
        let src = encode(&opaque(120, 80), ImageFormat::WebP, Some(fake_icc()), Some(exif_chunk(3)));
        let out = prepare_cover(&src, OPTS_KEEP).unwrap();
        assert_eq!(out.mime, "image/jpeg");
        let (icc, exif, _) = metadata_of(&out.data);
        assert_eq!(icc.as_deref(), Some(fake_icc().as_slice()));
        assert_eq!(Orientation::from_exif_chunk(&exif.unwrap()), Some(Orientation::NoTransforms));
    }

    #[test]
    fn plan_matches_prepare() {
        let jpg = encode(&opaque(300, 200), ImageFormat::Jpeg, None, None);
        assert_eq!(
            plan_cover(&jpg, OPTS_RESIZE).unwrap(),
            CoverPlan { width: 150, height: 100, mime: "image/jpeg", reencode: true }
        );
        assert!(!plan_cover(&jpg, OPTS_KEEP).unwrap().reencode);
        let webp = encode(&transparent(40, 40), ImageFormat::WebP, None, None);
        let plan = plan_cover(&webp, OPTS_KEEP).unwrap();
        assert_eq!((plan.mime, plan.reencode), ("image/png", true));
    }
}
