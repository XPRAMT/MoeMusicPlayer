//! DSD playback (ADR 0016): the rodio source over the DSD→PCM conversion in `player-dsd`. DSF files and DSD inside
//! WavPack (ADR 0025) reach the same converter through the `DsdStream` trait.

pub(crate) mod pcm;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use player_dsd::dsf::DsfReader;
use player_dsd::stream::{DsdError, DsdStream};

use crate::{AudioError, DsdOptions};
use pcm::DsdPcmSource;

/// The concrete DSD source the playback chain plays, whatever container the 1-bit stream came from.
pub(crate) type DsdFileSource = DsdPcmSource<Box<dyn DsdStream>>;

/// Whether `path` is a DSF file (the only DSD container DoP can output).
pub(crate) fn is_dsf_path(path: &Path) -> bool {
    player_formats::decode_engine(path) == player_formats::DecodeEngine::Dsd
}

/// Whether `path` is played through the DSD converter: a DSF file or a WavPack file that holds DSD.
pub(crate) fn is_dsd_path(path: &Path) -> bool {
    if is_dsf_path(path) {
        return true;
    }
    if player_formats::decode_engine(path) != player_formats::DecodeEngine::WavPack {
        return false;
    }
    if crate::remote_input::is_remote(path) {
        return crate::remote_input::open(path)
            .is_ok_and(|input| player_wavpack::is_dsd_from(input.main));
    }
    player_wavpack::is_dsd(path)
}

fn decode_error(path: &Path) -> impl Fn(DsdError) -> AudioError + '_ {
    move |error| AudioError::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

fn source_for(
    path: &Path,
    stream: Box<dyn DsdStream>,
    options: &DsdOptions,
) -> Result<DsdFileSource, AudioError> {
    DsdPcmSource::with_gain(
        stream,
        options.settings().output_rate,
        options.gain_handle(),
    )
    .map_err(decode_error(path))
}

/// Open a DSF file for playback with the current DSD settings; the source follows the shared gain.
pub(crate) fn open_dsf(path: &Path, options: &DsdOptions) -> Result<DsdFileSource, AudioError> {
    if crate::remote_input::is_remote(path) {
        let input = crate::remote_input::open(path)?;
        let reader = DsfReader::open(BufReader::with_capacity(256 * 1024, input.main))
            .map_err(decode_error(path))?;
        return source_for(path, Box::new(reader), options);
    }
    let file = File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let reader =
        DsfReader::open(BufReader::with_capacity(256 * 1024, file)).map_err(decode_error(path))?;
    source_for(path, Box::new(reader), options)
}

/// Open a WavPack file that holds DSD for playback through the same converter, settings and gain as a DSF file.
pub(crate) fn open_wavpack_dsd(
    path: &Path,
    options: &DsdOptions,
) -> Result<DsdFileSource, AudioError> {
    let stream = if crate::remote_input::is_remote(path) {
        let input = crate::remote_input::open(path)?;
        player_wavpack::dsd::WavPackDsdStream::open(input.main, input.correction)
    } else {
        player_wavpack::dsd::WavPackDsdStream::open_path(path)
    }
    .map_err(|error| AudioError::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    source_for(path, Box::new(stream), options)
}
