//! Covers and lyrics of remote tracks for Android (ADR 0026), read through the shared `RemoteContent` provider. Windows
//! has the same lookups in `player-platform-windows`; this is the platform-neutral version the Android code uses (the
//! platform crate cannot be built for Android).

use std::io::{BufReader, Seek};
use std::path::Path;

use lofty::{
    config::ParseOptions, file::FileType, picture::PictureType, prelude::TaggedFileExt,
    probe::Probe, tag::ItemKey,
};
use player_core::{
    read_remote_file_capped, remote_content, remote_sibling, remote_with_extension, RemoteStream,
    SourceId, MAX_ARTWORK_BYTES, MAX_LYRIC_PAYLOAD_BYTES,
};

const SIDECAR_COVERS: [&str; 2] = ["cover.jpg", "folder.jpg"];

/// The raw bytes of a cover for a remote track: the picture embedded in the file (the front cover first), else the
/// `cover.jpg` / `folder.jpg` beside it. The caller checks that the bytes are a supported image.
pub(crate) fn remote_cover_bytes(source_id: SourceId, path: &str) -> Option<Vec<u8>> {
    let content = remote_content()?;
    if let Ok(handle) = content.open(source_id, path) {
        if let Some(bytes) = embedded_cover(handle.reader, path) {
            return Some(bytes);
        }
    }
    SIDECAR_COVERS.iter().find_map(|name| {
        read_remote_file_capped(
            source_id,
            &remote_sibling(path, name),
            MAX_ARTWORK_BYTES as u64,
        )
    })
}

fn embedded_cover(reader: Box<dyn RemoteStream>, path: &str) -> Option<Vec<u8>> {
    let name = Path::new(path);
    // DSF keeps its cover in the ID3v2 tag at the end of the file, which lofty cannot read as a DSF.
    let dsd_tags;
    let tagged;
    let tags: &[lofty::tag::Tag] =
        if player_formats::decode_engine(name) == player_formats::DecodeEngine::Dsd {
            let (_, tag) = player_dsd::id3::read_dsf_from(reader).ok()?;
            dsd_tags = tag.into_iter().collect::<Vec<_>>();
            &dsd_tags
        } else {
            let mut reader = BufReader::new(reader);
            let file_type = match FileType::from_path(name) {
                Some(file_type) => file_type,
                None => Probe::new(&mut reader)
                    .guess_file_type()
                    .ok()?
                    .file_type()?,
            };
            // WavPack and Monkey's Audio keep their covers as APEv2 items that lofty's generic tag does not carry.
            if player_tags::ape_cover::is_ape_family(file_type) {
                reader.rewind().ok()?;
                let picture = player_tags::ape_cover::ape_front_cover(&mut reader, file_type)?;
                return Some(picture.data().to_vec());
            }
            reader.rewind().ok()?;
            tagged = Probe::with_file_type(reader, file_type)
                .options(ParseOptions::new().read_properties(false))
                .read()
                .ok()?;
            tagged.tags()
        };
    let front = tags
        .iter()
        .flat_map(|tag| tag.pictures())
        .find(|picture| picture.pic_type() == PictureType::CoverFront);
    let any = tags.iter().flat_map(|tag| tag.pictures()).next();
    front
        .or(any)
        .map(|picture| picture.data().to_vec())
        .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_ARTWORK_BYTES)
}

/// The lyric texts of a remote track: the `.lrc` beside it, and the lyrics embedded in its tags (each raw, not yet
/// parsed). Texts larger than the lyric limit are dropped and reported through the second value.
pub(crate) fn remote_lyric_texts(
    source_id: SourceId,
    path: &str,
    with_sidecar: bool,
) -> (Option<String>, Vec<String>, bool) {
    let mut oversized = false;
    let sidecar_path = remote_with_extension(path, "lrc");
    let sidecar = with_sidecar
        .then(|| read_remote_file_capped(source_id, &sidecar_path, MAX_LYRIC_PAYLOAD_BYTES as u64))
        .flatten()
        .map(|bytes| {
            String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                .into_owned()
        });
    if with_sidecar && sidecar.is_none() {
        if let Some(handle) = remote_content().and_then(|c| c.open(source_id, &sidecar_path).ok()) {
            oversized |= handle.len > MAX_LYRIC_PAYLOAD_BYTES as u64;
        }
    }
    let mut embedded = Vec::new();
    if let Some(handle) = remote_content().and_then(|content| content.open(source_id, path).ok()) {
        let tagged = Probe::new(BufReader::new(handle.reader))
            .options(ParseOptions::new().read_properties(false))
            .guess_file_type()
            .ok()
            .and_then(|probe| probe.read().ok());
        if let Some(tagged) = tagged {
            for key in [ItemKey::Lyrics, ItemKey::UnsyncLyrics] {
                for raw in tagged.tags().iter().filter_map(|tag| tag.get_string(key)) {
                    if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                        oversized = true;
                    } else {
                        embedded.push(raw.to_owned());
                    }
                }
            }
        }
    }
    (sidecar, embedded, oversized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_core::{RemoteContent, RemoteFileHandle};
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Folder(PathBuf, SourceId);

    impl RemoteContent for Folder {
        fn open(&self, source_id: SourceId, path: &str) -> std::io::Result<RemoteFileHandle> {
            if source_id != self.1 {
                return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "unknown"));
            }
            let file = std::fs::File::open(self.0.join(path))?;
            let len = file.metadata()?.len();
            Ok(RemoteFileHandle {
                reader: Box::new(file),
                len,
            })
        }
    }

    #[test]
    fn remote_covers_and_lyrics_are_read_beside_and_inside_the_track() {
        let root = std::env::temp_dir().join(format!("moe-remote-android-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a")).unwrap();
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../crates/player-wavpack/tests/fixtures");
        std::fs::copy(fixtures.join("s16_lyrics.wv"), root.join("a/song.wv")).unwrap();
        std::fs::write(root.join("a/song.lrc"), "\u{feff}[00:01.00]line\n").unwrap();
        std::fs::write(root.join("a/cover.jpg"), b"not really a jpeg").unwrap();
        let source = SourceId::new();
        player_core::set_remote_content(Arc::new(Folder(root.clone(), source)));

        let (sidecar, embedded, oversized) = remote_lyric_texts(source, "a/song.wv", true);
        assert_eq!(
            sidecar.as_deref(),
            Some("[00:01.00]line\n"),
            "the BOM is dropped"
        );
        assert!(!embedded.is_empty(), "the APEv2 lyrics of the WavPack file");
        assert!(!oversized);
        // No cover is embedded in the fixture, so the sidecar bytes are returned for the caller to validate.
        assert_eq!(
            remote_cover_bytes(source, "a/song.wv").as_deref(),
            Some(&b"not really a jpeg"[..])
        );
        assert_eq!(
            remote_cover_bytes(source, "a/missing.wv"),
            Some(b"not really a jpeg".to_vec())
        );
        let (none, empty, _) = remote_lyric_texts(source, "a/missing.wv", true);
        assert!(none.is_none());
        assert!(empty.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
