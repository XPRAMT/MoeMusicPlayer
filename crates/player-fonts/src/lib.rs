//! Installed fonts for the font pickers (ADR 0042).
//!
//! Everything here is a pure function of font files and the OS region, except `enumerate` (which lists the installed
//! font files of this system) and `system_region`. Font bytes are only read here, never kept: callers cache the
//! parsed [`FaceInfo`] (names, scripts, weight) keyed by [`FileStamp`] and reparse only new or changed files.

mod catalog;
mod diff;
mod enumerate;
mod names;
mod parse;
mod region;
mod scripts;
mod sfnt;
mod subset;

pub use catalog::{build_catalog, visible_families, FamilyFace, FontFamily};
pub use diff::{diff_files, FileDiff, FileStamp};
pub use enumerate::{android_font_directories, file_stamp, system_font_files};
pub use names::{canonical_name, language_preference, pick_name, LocalizedName};
pub use parse::{face_from_tables, read_font_file, FaceInfo, FaceNames, PARSE_VERSION};
pub use region::{font_group, group_order, system_region, FontGroup, FontRegion};
pub use scripts::{
    assess_coverage, detect_scripts, is_cjk_mark, legacy_encoding_scripts, Coverage, Scripts,
};
pub use subset::{name_subset, read_standalone_face, stable_hash, SUBSET_VERSION};
