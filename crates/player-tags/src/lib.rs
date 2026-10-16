//! Single-track tag details, staged-edit writing and spectrogram rendering (ADR 0013).
//!
//! Decisions: MP3 always written as ID3v2.4; mtime updates normally; artist fields are
//! plain text written as-is; lyrics are always embedded; one front cover only; DSF read-only.

pub mod cover;
pub mod spectrogram;
pub mod tags;

pub use cover::{prepare_cover, CoverOptions, PreparedCover};
pub use spectrogram::{render_spectrogram, Spectrogram, SpectrogramOptions};
pub use tags::{read_details, write_edit, AudioProperties, TagEdit, TagFields, TrackDetails};
