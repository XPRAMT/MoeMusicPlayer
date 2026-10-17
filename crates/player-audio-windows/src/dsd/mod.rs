//! DSD playback (ADR 0016): the rodio source over the DSD→PCM conversion in `player-dsd`.

pub(crate) mod pcm;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use player_dsd::dsf::DsfReader;

use crate::{AudioError, DsdOptions};
use pcm::DsdPcmSource;

/// The concrete DSD source the playback chain plays.
pub(crate) type DsfPcmSource = DsdPcmSource<DsfReader<BufReader<File>>>;

/// Whether `path` is a DSD file (played through the converter).
pub(crate) fn is_dsd_path(path: &Path) -> bool {
    player_formats::decode_engine(path) == player_formats::DecodeEngine::Dsd
}

/// Open a DSF file for playback with the current DSD settings; the source follows the shared gain.
pub(crate) fn open_dsf(path: &Path, options: &DsdOptions) -> Result<DsfPcmSource, AudioError> {
    let settings = options.settings();
    let file = File::open(path).map_err(|error| AudioError::FileOpen {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let decode_error = |error: player_dsd::stream::DsdError| AudioError::Decode {
        path: path.to_path_buf(),
        message: error.to_string(),
    };
    let reader =
        DsfReader::open(BufReader::with_capacity(256 * 1024, file)).map_err(decode_error)?;
    DsdPcmSource::with_gain(reader, settings.output_rate, options.gain_handle())
        .map_err(decode_error)
}
