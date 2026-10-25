//! Single-track tag details, staged-edit writing and spectrogram rendering (ADR 0013).
//!
//! Decisions: MP3 always written as ID3v2.4; mtime updates normally; artist fields are
//! plain text written as-is; lyrics are always embedded; one front cover only; DSF is rewritten through its trailing ID3v2 tag (ADR 0013 D10), DFF is read-only.

pub mod ape_cover;
pub mod cover;
pub mod file_times;
pub mod spectrogram;
pub mod tags;

pub use cover::{plan_cover, prepare_cover, CoverOptions, CoverPlan, PreparedCover};
pub use spectrogram::{
    colormap_lut, render_spectrogram, ChannelSelect, Colormap, Spectrogram, SpectrogramOptions,
    WindowFunction,
};
pub use tags::{read_details, write_edit, AudioProperties, TagEdit, TagFields, TrackDetails};
