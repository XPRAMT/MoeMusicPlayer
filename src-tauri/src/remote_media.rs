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
    is_remote_network_error, read_remote_file_capped_checked, read_remote_sidecar_checked,
    remember_remote_tag_lyrics, remembered_remote_tag_lyrics, remote_content, remote_sibling,
    remote_with_extension, LyricSidecarLookup, MediaLocator, RemoteStream, SourceId,
    MAX_ARTWORK_BYTES, MAX_LYRIC_PAYLOAD_BYTES,
};

const SIDECAR_COVERS: [&str; 2] = ["cover.jpg", "folder.jpg"];

/// The raw bytes of a cover for a remote track: the picture embedded in the file (the front cover first), else the
/// `cover.jpg` / `folder.jpg` beside it. The caller checks that the bytes are a supported image.
/// How many covers are looked up on servers at the same time. Many at once on one connection only make every one of them
/// slow (a page of albums asks for dozens), so the rest wait their turn. Two lines: what a person is waiting for (one
/// track's cover, a notification) and the bulk work of picking album covers, which must not starve the first.
const MAX_CONCURRENT_COVER_LOOKUPS: usize = 3;
const MAX_CONCURRENT_BULK_COVER_LOOKUPS: usize = 2;

static COVER_LOOKUPS: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());
static BULK_COVER_LOOKUPS: (std::sync::Mutex<usize>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());

thread_local! {
    static BULK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `work` (on the calling thread) as bulk cover work: its lookups wait in the bulk line.
pub(crate) fn as_bulk_cover_work<T>(work: impl FnOnce() -> T) -> T {
    BULK.with(|flag| flag.set(true));
    let result = work();
    BULK.with(|flag| flag.set(false));
    result
}

struct CoverLookupTurn(&'static (std::sync::Mutex<usize>, std::sync::Condvar));

impl CoverLookupTurn {
    fn wait() -> Self {
        let (line, limit) = if BULK.with(std::cell::Cell::get) {
            (&BULK_COVER_LOOKUPS, MAX_CONCURRENT_BULK_COVER_LOOKUPS)
        } else {
            (&COVER_LOOKUPS, MAX_CONCURRENT_COVER_LOOKUPS)
        };
        let (count, turn) = line;
        let mut running = count.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *running >= limit {
            running = turn.wait(running).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *running += 1;
        Self(line)
    }
}

impl Drop for CoverLookupTurn {
    fn drop(&mut self) {
        let (count, turn) = self.0;
        *count.lock().unwrap_or_else(std::sync::PoisonError::into_inner) -= 1;
        turn.notify_one();
    }
}

pub(crate) fn remote_cover_bytes(source_id: SourceId, path: &str) -> Option<Vec<u8>> {
    let content = remote_content()?;
    let _turn = CoverLookupTurn::wait();
    let started = std::time::Instant::now();
    let opened = content.open(source_id, path);
    let opened_in = started.elapsed().as_millis();
    match opened {
        Ok(handle) => {
            let len = handle.len;
            let read_started = std::time::Instant::now();
            // The lyrics lookup of the song that starts needs the same tags: it finds them here.
            let mut lyrics = None;
            let found = embedded_cover(handle.reader, path, &mut lyrics);
            if let Some(texts) = lyrics {
                remember_remote_tag_lyrics(source_id, path, len, texts);
            }
            eprintln!(
                "[remote] cover stages: open {opened_in} ms, tags of {len} bytes {} ms (embedded: {})",
                read_started.elapsed().as_millis(),
                found.is_some()
            );
            if let Some(bytes) = found {
                return Some(bytes);
            }
        }
        Err(error) => {
            eprintln!("[remote] cover stages: open failed after {opened_in} ms");
            // The server cannot be reached: trying the sidecars would only fail the same way, slowly.
            if is_remote_network_error(&error) {
                return None;
            }
        }
    }
    for name in SIDECAR_COVERS {
        let sidecar_started = std::time::Instant::now();
        let read = read_remote_file_capped_checked(
            source_id,
            &remote_sibling(path, name),
            MAX_ARTWORK_BYTES as u64,
        );
        eprintln!(
            "[remote] cover stages: {name} {} ms (found: {})",
            sidecar_started.elapsed().as_millis(),
            matches!(read, Ok(Some(_)))
        );
        match read {
            Ok(Some(bytes)) => return Some(bytes),
            Err(error) if is_remote_network_error(&error) => return None,
            _ => {}
        }
    }
    None
}

/// The cover in a remote track's tags; `lyrics` gets the lyric texts of the same tags when they were read the way the
/// lyrics lookup reads them (lofty's generic tags).
fn embedded_cover(
    reader: Box<dyn RemoteStream>,
    path: &str,
    lyrics: &mut Option<Vec<String>>,
) -> Option<Vec<u8>> {
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
            *lyrics = Some(lyric_texts_in_tags(tagged.tags()));
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

/// Every lyric text in the tags (`Lyrics`, then `UnsyncLyrics`), raw, in that order.
fn lyric_texts_in_tags(tags: &[lofty::tag::Tag]) -> Vec<String> {
    [ItemKey::Lyrics, ItemKey::UnsyncLyrics]
        .into_iter()
        .flat_map(|key| tags.iter().filter_map(move |tag| tag.get_string(key)))
        .map(str::to_owned)
        .collect()
}

/// The lyric texts of a remote track, read now (ADR 0040): the ones embedded in its tags (shared with a cover lookup of
/// the same file that just read them), and its sidecar (each raw, not yet parsed). `sidecar` is what the library knows of
/// the sidecar: the file it names is read, `Absent` reads none, `NotScanned` tries `.lrc`, `.qrc`, `.ass`, `.srt`; sidecars
/// are read through the disk cache, so a song that plays without its server has its sidecar too. After a network error
/// nothing else on the server is tried. Texts larger than the lyric limit are dropped and reported through the last value.
pub(crate) fn remote_lyric_texts(
    source_id: SourceId,
    path: &str,
    sidecar: &LyricSidecarLookup,
) -> (Vec<String>, Vec<String>, bool) {
    let mut oversized = false;
    let mut unreachable = false;
    let mut embedded = Vec::new();
    match remote_content().map(|content| content.open(source_id, path)) {
        Some(Ok(handle)) => {
            let len = handle.len;
            let texts = remembered_remote_tag_lyrics(source_id, path, len).unwrap_or_else(|| {
                let texts = Probe::new(BufReader::new(handle.reader))
                    .options(ParseOptions::new().read_properties(false))
                    .guess_file_type()
                    .ok()
                    .and_then(|probe| probe.read().ok())
                    .map(|tagged| lyric_texts_in_tags(tagged.tags()))
                    .unwrap_or_default();
                remember_remote_tag_lyrics(source_id, path, len, texts.clone());
                texts
            });
            for raw in texts {
                if raw.len() > MAX_LYRIC_PAYLOAD_BYTES {
                    oversized = true;
                } else {
                    embedded.push(raw);
                }
            }
        }
        Some(Err(error)) => unreachable = is_remote_network_error(&error),
        None => {}
    }
    let candidates: Vec<String> = match sidecar {
        LyricSidecarLookup::Absent => Vec::new(),
        LyricSidecarLookup::Present { .. } => match sidecar.sidecar_locator() {
            Some(MediaLocator::Remote { path, .. }) => vec![path],
            _ => Vec::new(),
        },
        LyricSidecarLookup::NotScanned => player_core::LYRIC_SIDECAR_EXTENSIONS
            .iter()
            .map(|extension| remote_with_extension(path, extension))
            .collect(),
    };
    let mut sidecars = Vec::new();
    for sidecar_path in candidates.iter().filter(|_| !unreachable) {
        match read_remote_sidecar_checked(source_id, sidecar_path, MAX_LYRIC_PAYLOAD_BYTES as u64) {
            Ok(Some(bytes)) => sidecars.push(
                String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes))
                    .into_owned(),
            ),
            Ok(None) => {
                if let Some(handle) =
                    remote_content().and_then(|c| c.open(source_id, sidecar_path).ok())
                {
                    oversized |= handle.len > MAX_LYRIC_PAYLOAD_BYTES as u64;
                }
            }
            // The server cannot be reached: the other files fail the same way.
            Err(error) if is_remote_network_error(&error) => break,
            Err(_) => {}
        }
    }
    (sidecars, embedded, oversized)
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

        let search = LyricSidecarLookup::NotScanned;
        let (sidecar, embedded, oversized) = remote_lyric_texts(source, "a/song.wv", &search);
        assert_eq!(
            sidecar.first().map(String::as_str),
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
        let (none, empty, _) = remote_lyric_texts(source, "a/missing.wv", &search);
        assert!(none.is_empty());
        assert!(empty.is_empty());
        // The sidecar the sync found is read by its name, with its current content; a known absence reads none.
        std::fs::write(
            root.join("a/other.srt"),
            "1\n00:00:01,000 --> 00:00:02,000\nsrt\n",
        )
        .unwrap();
        let known = LyricSidecarLookup::Present {
            audio: MediaLocator::Remote {
                source_id: source,
                path: "a/song.wv".to_owned(),
            },
            name: "other.srt".to_owned(),
        };
        let (sidecar, _, _) = remote_lyric_texts(source, "a/song.wv", &known);
        assert_eq!(sidecar, ["1\n00:00:01,000 --> 00:00:02,000\nsrt\n"]);
        let (sidecar, embedded, _) =
            remote_lyric_texts(source, "a/song.wv", &LyricSidecarLookup::Absent);
        assert!(sidecar.is_empty());
        assert!(!embedded.is_empty());
        let _ = std::fs::remove_dir_all(root);

        // A server that cannot be reached: the lookups stop after the first network error instead of trying every file.
        let down = Arc::new(Down(std::sync::atomic::AtomicUsize::new(0)));
        player_core::set_remote_content(down.clone());
        assert_eq!(remote_cover_bytes(source, "a/song.wv"), None);
        assert_eq!(down.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        down.0.store(0, std::sync::atomic::Ordering::SeqCst);
        let (sidecars, embedded, _) =
            remote_lyric_texts(source, "a/song.wv", &LyricSidecarLookup::NotScanned);
        assert!(sidecars.is_empty() && embedded.is_empty());
        assert_eq!(down.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Every open fails on the network; counts the opens.
    struct Down(std::sync::atomic::AtomicUsize);

    impl RemoteContent for Down {
        fn open(&self, _: SourceId, _: &str) -> std::io::Result<RemoteFileHandle> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(std::io::Error::from(player_remote::fs::RemoteError::Network(
                "No route to host".to_owned(),
            )))
        }
    }
}
