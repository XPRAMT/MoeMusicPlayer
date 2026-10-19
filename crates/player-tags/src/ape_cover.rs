//! Covers of APEv2 tags (WavPack, Monkey's Audio): lofty keeps them as `Cover Art (...)` binary items and does not
//! turn them into the pictures of the generic `Tag` it hands out, so they are read from the `ApeTag` directly.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use lofty::ape::{ApeFile, ApeTag, APE_PICTURE_TYPES};
use lofty::config::ParseOptions;
use lofty::file::FileType;
use lofty::picture::{Picture, PictureType};
use lofty::prelude::AudioFile;
use lofty::tag::ItemValue;
use lofty::wavpack::WavPackFile;

/// Whether `file_type` keeps its tag as APEv2 with covers as binary items.
pub fn is_ape_family(file_type: FileType) -> bool {
    matches!(file_type, FileType::WavPack | FileType::Ape)
}

/// The file's APEv2 tag (`None` when it has none or it cannot be read).
pub(crate) fn read_ape_tag<R: Read + Seek>(reader: &mut R, file_type: FileType) -> Option<ApeTag> {
    let options = ParseOptions::new().read_properties(false);
    match file_type {
        FileType::WavPack => WavPackFile::read_from(reader, options)
            .ok()
            .and_then(|file| file.ape().cloned()),
        FileType::Ape => ApeFile::read_from(reader, options)
            .ok()
            .and_then(|file| file.ape().cloned()),
        _ => None,
    }
}

/// Every cover picture of an APEv2 tag.
pub fn ape_pictures(tag: &ApeTag) -> Vec<Picture> {
    tag.into_iter()
        .filter(|item| APE_PICTURE_TYPES.contains(&item.key()))
        .filter_map(|item| match item.value() {
            ItemValue::Binary(bytes) => Picture::from_ape_bytes(item.key(), bytes).ok(),
            _ => None,
        })
        .collect()
}

/// The front cover (or the first picture) of an APE-family file.
pub fn ape_front_cover<R: Read + Seek>(reader: &mut R, file_type: FileType) -> Option<Picture> {
    let pictures = ape_pictures(&read_ape_tag(reader, file_type)?);
    let front = pictures
        .iter()
        .position(|picture| picture.pic_type() == PictureType::CoverFront)
        .unwrap_or(0);
    pictures.into_iter().nth(front)
}

/// [`ape_front_cover`] for a path.
pub fn ape_front_cover_of_path(path: &Path) -> Option<Picture> {
    let file_type = FileType::from_path(path).filter(|t| is_ape_family(*t))?;
    ape_front_cover(&mut BufReader::new(File::open(path).ok()?), file_type)
}
