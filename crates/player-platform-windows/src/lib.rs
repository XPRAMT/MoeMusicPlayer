//! Windows media-index adapter.
//!
//! The adapter prefers a trustworthy platform index when one can be proven complete for the
//! configured root. The initial implementation uses a complete filesystem traversal because an
//! empty Windows Search result alone cannot prove that a root was indexed.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
mod artwork;

#[cfg(windows)]
mod system_index;

#[cfg(windows)]
pub use windows::{windows_locator_key, WindowsMediaIndex};

#[cfg(windows)]
pub use artwork::{find_artwork, ArtworkLookup};
