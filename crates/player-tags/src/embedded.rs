//! The pictures embedded in a track's tags, read from any byte stream. One code path for every container, used by the
//! local cover lookups (Windows, Android) and the remote ones, so a cover that one finds the others find too (the user's
//! rule: no local-vs-remote differences). DSF keeps its tag as ID3v2 at the end of the file (lofty does not read it),
//! WavPack and Monkey's Audio keep covers as APEv2 binary items (lofty's generic tag does not carry them), every other
//! container goes through lofty.

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

use lofty::config::ParseOptions;
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{Picture, PictureType};
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag};

use crate::ape_cover::{ape_pictures, is_ape_family, read_ape_tag};

/// What the tags of one file hold.
pub struct EmbeddedTags {
    /// Every embedded picture, front covers first.
    pub pictures: Vec<Picture>,
    /// The tags of the file as lofty's generic tags, whatever the container (DSF and APE-family tags converted), so
    /// the caller can take the lyrics of the same read.
    pub tags: Vec<Tag>,
}

impl EmbeddedTags {
    /// The front cover, else the first picture.
    pub fn front_cover(&self) -> Option<&Picture> {
        self.pictures.first()
    }

    /// Every lyric text in the tags (`Lyrics`, then `UnsyncLyrics`), raw, in that order. The one reading of embedded
    /// lyrics for every container, platform and local or remote file.
    pub fn lyric_texts(&self) -> Vec<String> {
        lyric_texts_in_tags(&self.tags)
    }
}

/// [`EmbeddedTags::lyric_texts`] for tags already in hand.
pub fn lyric_texts_in_tags(tags: &[Tag]) -> Vec<String> {
    [ItemKey::Lyrics, ItemKey::UnsyncLyrics]
        .into_iter()
        .flat_map(|key| tags.iter().filter_map(move |tag| tag.get_string(key)))
        .map(str::to_owned)
        .collect()
}

/// The lyric texts in the tags of `reader` (see [`read_embedded_tags`]); none when the tags cannot be read.
pub fn read_embedded_lyric_texts<R: Read + Seek + Send>(reader: R, name: &Path) -> Vec<String> {
    read_embedded_tags(reader, name)
        .map(|tags| tags.lyric_texts())
        .unwrap_or_default()
}

/// Read the pictures of the tags of `reader`; `name` only names the file (its extension picks the container, and the
/// content decides when the extension says nothing). `None` when the tags cannot be read at all.
pub fn read_embedded_tags<R: Read + Seek + Send>(reader: R, name: &Path) -> Option<EmbeddedTags> {
    if player_formats::decode_engine(name) == player_formats::DecodeEngine::Dsd {
        let (_, tag) = player_dsd::id3::read_dsf_from(reader).ok()?;
        return Some(EmbeddedTags {
            pictures: front_first(
                tag.iter()
                    .flat_map(|tag| tag.pictures().iter().cloned())
                    .collect(),
            ),
            tags: tag.into_iter().collect(),
        });
    }
    let mut reader = BufReader::new(reader);
    let file_type = match FileType::from_path(name) {
        Some(file_type) => file_type,
        None => Probe::new(&mut reader)
            .guess_file_type()
            .ok()?
            .file_type()?,
    };
    reader.rewind().ok()?;
    if is_ape_family(file_type) {
        let tag = read_ape_tag(&mut reader, file_type)?;
        return Some(EmbeddedTags {
            pictures: front_first(ape_pictures(&tag)),
            tags: vec![Tag::from(tag)],
        });
    }
    let tagged = Probe::with_file_type(reader, file_type)
        .options(ParseOptions::new().read_properties(false))
        .read()
        .ok()?;
    let tags = tagged.tags().to_vec();
    Some(EmbeddedTags {
        pictures: front_first(
            tags.iter()
                .flat_map(|tag| tag.pictures().iter().cloned())
                .collect(),
        ),
        tags,
    })
}

/// [`read_embedded_tags`] for a file on disk.
pub fn read_embedded_tags_of_path(path: &Path) -> Option<EmbeddedTags> {
    read_embedded_tags(File::open(path).ok()?, path)
}

fn front_first(mut pictures: Vec<Picture>) -> Vec<Picture> {
    // A stable sort keeps the file's order inside each group.
    pictures.sort_by_key(|picture| picture.pic_type() != PictureType::CoverFront);
    pictures
}
