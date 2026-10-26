//! Covers and lyrics of remote tracks (ADR 0026), read through the shared remote content provider over a folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use player_core::{
    LyricProvider, LyricSidecarLookup, MediaLocator, RemoteContent, RemoteFileHandle, SourceId,
};
use player_platform_windows::{
    find_all_lyrics, find_artwork, find_lyrics, ArtworkLookup, LyricsLookup,
};

/// A 1x1 PNG, named `cover.jpg` below: the lookup goes by the bytes, not the name.
const PNG: [u8; 70] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0x60, 0xf8, 0x5f, 0x0f, 0x00, 0x02, 0x87, 0x01, 0x80, 0xeb, 0x47, 0xba, 0x92, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82];

struct FolderContent {
    root: PathBuf,
    source: SourceId,
    /// The files read through the disk cache (`open_cached`).
    cached: std::sync::Mutex<Vec<String>>,
}

impl RemoteContent for FolderContent {
    fn open(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle> {
        if source_id != self.source {
            return Err(io::Error::new(io::ErrorKind::NotFound, "unknown source"));
        }
        let file = fs::File::open(self.root.join(path))?;
        let len = file.metadata()?.len();
        Ok(RemoteFileHandle {
            reader: Box::new(file),
            len,
        })
    }

    fn open_cached(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle> {
        self.cached.lock().unwrap().push(path.to_owned());
        self.open(source_id, path)
    }
}

fn wavpack_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../player-wavpack/tests/fixtures").join(name)
}

#[test]
fn remote_lyrics_and_covers_are_found_beside_and_inside_the_track() {
    let root = std::env::temp_dir().join(format!("moe-remote-media-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    // a/: a track with embedded lyrics, an .lrc beside it and a cover.jpg; b/: a track with embedded lyrics only.
    fs::copy(wavpack_fixture("s16_lyrics.wv"), root.join("a/song.wv")).unwrap();
    fs::write(root.join("a/song.lrc"), "[00:01.00]sidecar line
").unwrap();
    fs::write(root.join("a/cover.jpg"), PNG).unwrap();
    fs::copy(wavpack_fixture("s16_lyrics.wv"), root.join("b/song.wv")).unwrap();

    let source = SourceId::new();
    let content = Arc::new(FolderContent {
        root: root.clone(),
        source,
        cached: Default::default(),
    });
    player_core::set_remote_content(content.clone());
    let remote = |path: &str| {
        vec![MediaLocator::Remote {
            source_id: source,
            path: path.to_owned(),
        }]
    };
    let search = LyricSidecarLookup::NotScanned;
    let find_lyrics = |locators: &[MediaLocator]| find_lyrics(locators, &search);

    // Lyrics (ADR 0041): the embedded lyrics win for playback, both are offered when choosing.
    let LyricsLookup::Found(found) = find_lyrics(&remote("a/song.wv")) else {
        panic!("lyrics beside the remote track");
    };
    assert_eq!(found.provider, LyricProvider::Embedded);
    let all = find_all_lyrics(&remote("a/song.wv"), &search);
    let providers: Vec<_> = all.iter().map(|item| item.provider).collect();
    assert_eq!(providers, vec![LyricProvider::Embedded, LyricProvider::Sidecar]);
    assert_eq!(all[1].lyrics.lines[0].text, "sidecar line");

    // Without a sidecar the embedded lyrics (an APEv2 item of the WavPack file) are used.
    let LyricsLookup::Found(embedded) = find_lyrics(&remote("b/song.wv")) else {
        panic!("embedded lyrics");
    };
    assert_eq!(embedded.provider, LyricProvider::Embedded);
    assert!(!embedded.lyrics.lines.is_empty());

    // The sidecar the sync found is read by its name, through the disk cache.
    content.cached.lock().unwrap().clear();
    let known = LyricSidecarLookup::Present {
        audio: MediaLocator::Remote {
            source_id: source,
            path: "a/song.wv".to_owned(),
        },
        name: "song.lrc".to_owned(),
    };
    let all = find_all_lyrics(&remote("a/song.wv"), &known);
    assert_eq!(all[1].lyrics.lines[0].text, "sidecar line");
    assert_eq!(*content.cached.lock().unwrap(), ["a/song.lrc"]);
    let none = find_all_lyrics(&remote("b/song.wv"), &LyricSidecarLookup::Absent);
    assert_eq!(none.len(), 1, "embedded only");

    // No lyrics for a file that is not there.
    assert!(matches!(find_lyrics(&remote("a/missing.wv")), LyricsLookup::Missing));

    // Cover: the sidecar beside the track; none in the other folder.
    let ArtworkLookup::Found(image) = find_artwork(&remote("a/song.wv")) else {
        panic!("cover.jpg beside the remote track");
    };
    assert_eq!(image.mime_type(), "image/png");
    assert!(matches!(find_artwork(&remote("b/song.wv")), ArtworkLookup::Missing));

    let _ = fs::remove_dir_all(root);
}
