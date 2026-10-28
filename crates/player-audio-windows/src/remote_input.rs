//! Playing files that are not on this machine (ADR 0026).
//!
//! The engine loads a [`Path`]. A remote track is handed over as a *pseudo path* (`moeremote://<source id>/<path>`, built
//! by `player_core::remote_locator_uri`); every place that opens a file asks this module first, which reads it through the
//! provider the app installed with `player_core::set_remote_content`. The engine knows nothing about servers.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Mutex, PoisonError};

pub use player_wavpack::ReadSeek;

use crate::AudioError;

/// One open remote file: its bytes, how long it is, and the WavPack correction file next to it when there is one.
pub struct RemoteInput {
    pub main: Box<dyn ReadSeek>,
    pub len: u64,
    pub correction: Option<Box<dyn ReadSeek>>,
}

/// Whether `path` is a remote pseudo path.
pub fn is_remote(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|text| text.starts_with("moeremote://"))
}

/// The message of the last read that failed because the server could not be reached (the stream retried and gave
/// up). A decoder ends its track on any read error; the worker asks this when the track ends, to stop there with the
/// message instead of moving on (ADR 0026, "link lost").
static LINK_LOST: Mutex<Option<String>> = Mutex::new(None);

/// Take the "server could not be reached" failure of the playing remote track, if its stream reported one.
pub(crate) fn take_link_lost() -> Option<String> {
    LINK_LOST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}

/// Whether the playing remote track's stream reported "server could not be reached" (not taken).
pub(crate) fn link_lost_pending() -> bool {
    LINK_LOST
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
}

thread_local! {
    /// The calling thread is only checking that a file opens (a [`crate::TrackProbe`]): its reads are not the playing
    /// track's, so a failure of theirs must not be mistaken for the playing track's stream giving up.
    static PROBING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `check` as a probe on the calling thread (see [`PROBING`]).
pub(crate) fn probing<T>(check: impl FnOnce() -> T) -> T {
    let before = PROBING.with(|flag| flag.replace(true));
    let result = check();
    PROBING.with(|flag| flag.set(before));
    result
}

fn is_probing() -> bool {
    PROBING.with(std::cell::Cell::get)
}

/// A shared remote stream as the engine's `Read + Seek + Send`.
struct Stream(Box<dyn player_core::RemoteStream>);

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = self.0.read(buf);
        if let Err(error) = &result {
            let message = error.to_string();
            if player_core::is_remote_link_lost(&message) && !is_probing() {
                *LINK_LOST.lock().unwrap_or_else(PoisonError::into_inner) = Some(message);
            }
        }
        result
    }
}

impl Seek for Stream {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0.seek(position)
    }
}

pub(crate) fn open(path: &Path) -> Result<RemoteInput, AudioError> {
    let failed = |message: String| AudioError::FileOpen {
        path: path.to_path_buf(),
        message,
    };
    let content = player_core::remote_content()
        .ok_or_else(|| failed("remote playback is not available".to_owned()))?;
    let text = path
        .to_str()
        .ok_or_else(|| failed("the remote location is not text".to_owned()))?;
    let Some(player_core::MediaLocator::Remote {
        source_id,
        path: relative,
    }) = player_core::parse_remote_locator_uri(text)
    else {
        return Err(failed("not a remote location".to_owned()));
    };
    let main = content
        .open_streaming(source_id, &relative)
        .map_err(|error| failed(error.to_string()))?;
    // A new track starts without the failure of an earlier one (a probe is not the start of a track).
    if !is_probing() {
        LINK_LOST
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
    // A hybrid WavPack file is lossless with its `.wvc` correction file next to it.
    let correction = if relative.to_ascii_lowercase().ends_with(".wv") {
        content
            .open_streaming(source_id, &format!("{relative}c"))
            .ok()
            .map(|file| Box::new(Stream(file.reader)) as Box<dyn ReadSeek>)
    } else {
        None
    };
    Ok(RemoteInput {
        main: Box::new(Stream(main.reader)),
        len: main.len,
        correction,
    })
}

/// Open `path` for reading, a local file or a remote pseudo path alike, with its length in bytes. This is the one place
/// that tells the two apart for code that only needs the bytes (the DSEE HX header probe); decoders keep their own
/// opening because they also need the WavPack correction file and the like.
pub(crate) fn open_reader(path: &Path) -> Result<(Box<dyn ReadSeek>, u64), AudioError> {
    if is_remote(path) {
        let input = open(path)?;
        return Ok((input.main, input.len));
    }
    let file = std::fs::File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    Ok((Box::new(file), len))
}

/// What rodio's decoders read from: a local file or a remote one, `Sync` as rodio requires.
pub(crate) trait MediaRead: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> MediaRead for T {}

/// Makes a reader that is only `Send` usable where `Sync` is required: every access takes a lock, and a decoder only
/// ever uses the reader from one thread, so the lock is never contended.
pub(crate) struct SyncReader<R>(Mutex<R>);

impl<R> SyncReader<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self(Mutex::new(reader))
    }
}

impl<R: Read> Read for SyncReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .read(buf)
    }
}

impl<R: Seek> Seek for SyncReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.0
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .seek(position)
    }
}

/// A remote provider for tests: it serves fixture files by name.
#[cfg(test)]
pub(crate) mod test_support {
    use std::collections::HashMap;
    use std::io;
    use std::sync::{Mutex, PoisonError};

    /// Serves fixtures by name, so a remote pseudo path reads the same bytes a local path does.
    pub(crate) struct FixtureContent;

    /// How many times each name was opened.
    static OPENS: Mutex<Option<HashMap<String, usize>>> = Mutex::new(None);

    pub(crate) fn opens_of(name: &str) -> usize {
        OPENS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(|opens| opens.get(name).copied())
            .unwrap_or(0)
    }

    pub(crate) fn fixture_path(name: &str) -> std::path::PathBuf {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(temporary) = name.strip_prefix("tmp/") {
            return std::env::temp_dir().join(temporary);
        }
        match name.strip_prefix("wv/") {
            Some(wavpack) => manifest
                .join("../player-wavpack/tests/fixtures")
                .join(wavpack),
            None => manifest.join("tests/fixtures").join(name),
        }
    }

    /// The one source id the fixtures belong to.
    pub(crate) fn fixtures_source() -> player_core::SourceId {
        player_core::SourceId::parse("00000000-0000-4000-8000-000000000001").unwrap()
    }

    impl player_core::RemoteContent for FixtureContent {
        fn open(
            &self,
            source_id: player_core::SourceId,
            path: &str,
        ) -> io::Result<player_core::RemoteFileHandle> {
            if source_id != fixtures_source() {
                return Err(io::Error::new(io::ErrorKind::NotFound, "unknown source"));
            }
            let file = std::fs::File::open(fixture_path(path))?;
            let len = file.metadata()?.len();
            *OPENS
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert_with(HashMap::new)
                .entry(path.to_owned())
                .or_insert(0) += 1;
            Ok(player_core::RemoteFileHandle {
                reader: Box::new(file),
                len,
            })
        }
    }

    pub(crate) fn pseudo(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(player_core::remote_locator_uri(fixtures_source(), name))
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn only_moeremote_paths_are_remote() {
        assert!(is_remote(Path::new("moeremote://abc/Music/a.flac")));
        assert!(!is_remote(Path::new(r"C:\Music\a.flac")));
        assert!(!is_remote(Path::new(r"\\server\share\a.flac")));
    }

    #[test]
    fn a_send_only_reader_becomes_sync() {
        fn needs_sync<T: Sync>(_: &T) {}
        let mut reader = SyncReader::new(io::Cursor::new(vec![1u8, 2, 3, 4]));
        needs_sync(&reader);
        let mut buffer = [0u8; 2];
        reader.seek(SeekFrom::Start(2)).unwrap();
        reader.read_exact(&mut buffer).unwrap();
        assert_eq!(buffer, [3, 4]);
    }

    #[test]
    fn remote_pseudo_paths_decode_exactly_like_local_files() {
        use crate::rodio_backend::open_decoder_with;
        use crate::DsdOptions;
        use rodio::Source;

        player_core::set_remote_content(std::sync::Arc::new(FixtureContent));
        // An Opus file and a DSF file written to the temporary folder.
        let opus = format!("moe-remote-{}.opus", std::process::id());
        std::fs::write(
            std::env::temp_dir().join(&opus),
            player_opus::testing::write_opus(48_000, 2, 1_000.0, 0, 0),
        )
        .unwrap();
        let dsf = format!("moe-remote-{}.dsf", std::process::id());
        {
            use player_dsd::testing::{build_dsf, modulate, tone};
            let rate = 2_822_400_u32;
            let samples = (f64::from(rate) * 0.2) as usize / 16 * 16;
            let lsb = |bytes: Vec<u8>| bytes.into_iter().map(u8::reverse_bits).collect::<Vec<u8>>();
            let left = lsb(modulate(samples, tone(rate, 1_000.0, 0.25)));
            let right = lsb(modulate(samples, tone(rate, 3_000.0, 0.25)));
            std::fs::write(
                std::env::temp_dir().join(&dsf),
                build_dsf(rate, samples as u64, 4096, true, &[left, right]),
            )
            .unwrap();
        }
        let opus = format!("tmp/{opus}");
        let dsf = format!("tmp/{dsf}");
        for name in [
            opus.as_str(),
            dsf.as_str(),
            "seek-tone.flac",
            "seek-tone.mp3",
            "seek-tone.ogg",
            "seek-tone.wav",
            "wv/s16.wv",
            "wv/s24.wv",
            "wv/dsd64.wv",
        ] {
            let local = open_decoder_with(&fixture_path(name), &DsdOptions::default())
                .unwrap_or_else(|error| panic!("local {name}: {error}"));
            let remote_path = pseudo(name);
            assert!(is_remote(&remote_path));
            let remote = open_decoder_with(&remote_path, &DsdOptions::default())
                .unwrap_or_else(|error| panic!("remote {name}: {error}"));
            assert_eq!(local.sample_rate(), remote.sample_rate(), "{name}");
            assert_eq!(local.channels(), remote.channels(), "{name}");
            assert_eq!(local.total_duration(), remote.total_duration(), "{name}");
            let local: Vec<f32> = local.collect();
            let remote: Vec<f32> = remote.collect();
            assert!(!local.is_empty(), "{name}");
            assert_eq!(local, remote, "{name}: the samples differ");
        }
        let _ = std::fs::remove_file(fixture_path(&opus));
        let _ = std::fs::remove_file(fixture_path(&dsf));
    }

    #[test]
    fn a_remote_file_that_cannot_be_opened_is_a_file_open_error() {
        use crate::rodio_backend::open_decoder_with;
        use crate::DsdOptions;

        player_core::set_remote_content(std::sync::Arc::new(FixtureContent));
        let error = open_decoder_with(&pseudo("no-such-file.flac"), &DsdOptions::default())
            .err()
            .expect("opening fails");
        assert!(matches!(error, AudioError::FileOpen { .. }), "{error}");
        // A pseudo path for a server that was never configured fails the same way, not with a panic.
        let elsewhere = std::path::PathBuf::from(player_core::remote_locator_uri(
            player_core::SourceId::new(),
            "a.flac",
        ));
        assert!(open_decoder_with(&elsewhere, &DsdOptions::default()).is_err());
    }
}
