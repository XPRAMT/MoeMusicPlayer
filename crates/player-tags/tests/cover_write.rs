//! Cover writing per container × source format (user rule 2026-10-22): JPG unless the source has an
//! alpha channel (then PNG, transparency kept); unscaled JPG / alpha PNG keep their original bytes.
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use player_tags::{read_details, tags::read_cover, write_edit, CoverOptions, TagEdit};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../player-audio-windows/tests/fixtures")
}

fn encode(img: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    let img = if format == ImageFormat::Jpeg { DynamicImage::ImageRgb8(img.to_rgb8()) } else { img.clone() };
    img.write_to(&mut Cursor::new(&mut out), format).unwrap();
    out
}

fn sources() -> Vec<(&'static str, Vec<u8>, bool)> {
    let opaque = DynamicImage::ImageRgb8(RgbImage::from_fn(80, 60, |x, _| Rgb([x as u8, 90, 160])));
    let alpha = DynamicImage::ImageRgba8(RgbaImage::from_fn(80, 60, |x, _| {
        if x < 40 { Rgba([0, 255, 0, 0]) } else { Rgba([255, 0, 0, 255]) }
    }));
    vec![
        ("jpg", encode(&opaque, ImageFormat::Jpeg), false),
        ("png-opaque", encode(&opaque, ImageFormat::Png), false),
        ("png-alpha", encode(&alpha, ImageFormat::Png), true),
        ("webp-opaque", encode(&opaque, ImageFormat::WebP), false),
        ("webp-alpha", encode(&alpha, ImageFormat::WebP), true),
        ("bmp", encode(&opaque, ImageFormat::Bmp), false),
    ]
}

#[test]
fn each_container_and_source_format() {
    let dir = std::env::temp_dir().join(format!("moe-cover-write-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let options = [
        CoverOptions { jpeg_quality: 90, auto_resize: false, short_edge_px: 2160 },
        CoverOptions { jpeg_quality: 90, auto_resize: true, short_edge_px: 30 },
    ];
    for (ext, fixture) in [("mp3", "seek-tone.mp3"), ("flac", "seek-tone.flac"), ("m4a", "aac-lc.m4a")] {
        for (name, bytes, has_alpha) in sources() {
            for opts in options {
                let path = dir.join(format!("{name}-{}.{ext}", opts.auto_resize));
                fs::copy(fixtures().join(fixture), &path).unwrap();
                let fields = read_details(&path).unwrap().fields;
                let edit = TagEdit { fields, replace_cover: Some(bytes.clone()), remove_cover: false, cover_options: opts };
                write_edit(&path, &edit).unwrap_or_else(|e| panic!("{ext} {name}: {e}"));
                let (data, mime) = read_cover(&path).unwrap().expect("cover written");
                let label = format!("{ext} {name} resize={}", opts.auto_resize);
                let format = image::guess_format(&data).unwrap();
                if has_alpha {
                    assert_eq!(format, ImageFormat::Png, "{label}");
                    assert_eq!(mime, "image/png", "{label}");
                } else {
                    assert_eq!(format, ImageFormat::Jpeg, "{label}");
                    assert_eq!(mime, "image/jpeg", "{label}");
                }
                let kept = !opts.auto_resize && (name == "jpg" || name == "png-alpha");
                assert_eq!(data == bytes, kept, "{label}: original bytes kept?");
                let img = image::load_from_memory(&data).unwrap();
                let expected = if opts.auto_resize { (40, 30) } else { (80, 60) };
                assert_eq!((img.width(), img.height()), expected, "{label}");
                if has_alpha {
                    let rgba = img.to_rgba8();
                    assert_eq!(rgba.get_pixel(2, 2)[3], 0, "{label}: transparency kept");
                    assert_eq!(rgba.get_pixel(rgba.width() - 2, 2).0, [255, 0, 0, 255], "{label}");
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&dir);
}
