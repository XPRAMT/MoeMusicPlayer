//! DSD support without any audio-output dependency (ADR 0016).
//!
//! - [`stream`]: the container-neutral DSD bitstream contract ([`stream::DsdStream`]).
//! - [`dsf`]: the DSF reader.
//! - [`convert`]: the DSD→PCM conversion core (table-lookup decimation plus halfband stages).
//! - [`reader`]: the seekable, delay-compensated PCM reader on top of it; [`wav`] serves it as a float WAV file.
//!
//! The rodio source built on top lives in `player-audio-windows`; [`dop`] packs the same stream into DoP words for
//! exclusive-mode output.

pub mod convert;
pub mod dop;
pub mod dsf;
pub(crate) mod filters;
#[cfg(feature = "id3")]
pub mod id3;
pub mod reader;
pub mod stream;
pub mod wav;

#[cfg(any(test, feature = "testing"))]
pub mod testing;
