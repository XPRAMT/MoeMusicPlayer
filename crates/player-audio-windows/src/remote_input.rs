//! Playing files that are not on this machine (ADR 0026).
//!
//! The engine loads a [`Path`]. A remote track is handed over as a *pseudo path* (`moeremote://<source id>/<path>`, built
//! by `player_core::remote_locator_uri`); every place that opens a file asks this module first, and the app supplies the
//! [`RemoteOpener`] that turns the pseudo path into an open, cached reader. The engine knows nothing about servers.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

pub use player_wavpack::ReadSeek;

use crate::AudioError;

const REMOTE_PREFIX: &str = "moeremote://";

/// One open remote file: its bytes, how long it is, and the WavPack correction file next to it when there is one.
pub struct RemoteInput {
    pub main: Box<dyn ReadSeek>,
    pub len: u64,
    pub correction: Option<Box<dyn ReadSeek>>,
}

/// Opens remote files for playback. `locator` is the whole pseudo path.
pub trait RemoteOpener: Send + Sync {
    fn open(&self, locator: &str) -> io::Result<RemoteInput>;
}

static OPENER: RwLock<Option<Arc<dyn RemoteOpener>>> = RwLock::new(None);

/// Install the opener the app provides (once at start-up; a second call replaces it).
pub fn set_remote_opener(opener: Arc<dyn RemoteOpener>) {
    *OPENER.write().unwrap_or_else(PoisonError::into_inner) = Some(opener);
}

/// Whether `path` is a remote pseudo path.
pub fn is_remote(path: &Path) -> bool {
    path.to_str()
        .is_some_and(|text| text.starts_with(REMOTE_PREFIX))
}

pub(crate) fn open(path: &Path) -> Result<RemoteInput, AudioError> {
    let failed = |message: String| AudioError::FileOpen {
        path: path.to_path_buf(),
        message,
    };
    let opener = OPENER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| failed("remote playback is not available".to_owned()))?;
    let locator = path
        .to_str()
        .ok_or_else(|| failed("the remote location is not text".to_owned()))?;
    opener.open(locator).map_err(|error| failed(error.to_string()))
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
        self.0.get_mut().unwrap_or_else(PoisonError::into_inner).read(buf)
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

#[cfg(test)]
mod tests {
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

    /// Serves fixtures by name, so a remote pseudo path decodes the same bytes a local path does.
    struct FixtureOpener;

    fn fixture_path(name: &str) -> std::path::PathBuf {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(temporary) = name.strip_prefix("tmp/") {
            return std::env::temp_dir().join(temporary);
        }
        match name.strip_prefix("wv/") {
            Some(wavpack) => manifest.join("../player-wavpack/tests/fixtures").join(wavpack),
            None => manifest.join("tests/fixtures").join(name),
        }
    }

    impl RemoteOpener for FixtureOpener {
        fn open(&self, locator: &str) -> io::Result<RemoteInput> {
            let name = locator
                .strip_prefix("moeremote://fixtures/")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown source"))?;
            let file = std::fs::File::open(fixture_path(name))?;
            let len = file.metadata()?.len();
            Ok(RemoteInput {
                main: Box::new(file),
                len,
                correction: None,
            })
        }
    }

    #[test]
    fn remote_pseudo_paths_decode_exactly_like_local_files() {
        use crate::DsdOptions;
        use crate::rodio_backend::open_decoder_with;
        use rodio::Source;

        set_remote_opener(Arc::new(FixtureOpener));
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
            let remote_path = std::path::PathBuf::from(format!("moeremote://fixtures/{name}"));
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
        use crate::DsdOptions;
        use crate::rodio_backend::open_decoder_with;

        set_remote_opener(Arc::new(FixtureOpener));
        let error = open_decoder_with(
            Path::new("moeremote://fixtures/no-such-file.flac"),
            &DsdOptions::default(),
        )
        .err()
        .expect("opening fails");
        assert!(matches!(error, AudioError::FileOpen { .. }), "{error}");
        // A pseudo path for a server that was never configured fails the same way, not with a panic.
        assert!(open_decoder_with(
            Path::new("moeremote://elsewhere/a.flac"),
            &DsdOptions::default()
        )
        .is_err());
    }
}
