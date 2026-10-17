//! DSD playback (ADR 0016): the rodio source over the DSD→PCM conversion in `player-dsd`.

pub(crate) mod pcm;

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use player_dsd::dsf::DsfReader;

use crate::AudioError;
use pcm::{DsdPcmSource, DEFAULT_GAIN_DB, DEFAULT_OUTPUT_RATE};

/// The concrete DSD source the playback chain plays.
pub(crate) type DsfPcmSource = DsdPcmSource<DsfReader<BufReader<File>>>;

/// Open a DSF file for playback at the default PCM rate.
pub(crate) fn open_dsf(path: &Path) -> Result<DsfPcmSource, AudioError> {
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
    DsdPcmSource::new(reader, DEFAULT_OUTPUT_RATE, DEFAULT_GAIN_DB).map_err(decode_error)
}
