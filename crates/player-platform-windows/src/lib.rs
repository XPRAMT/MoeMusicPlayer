//! Windows media-index adapter.
//!
//! The adapter prefers a trustworthy platform index when one can be proven complete for the
//! configured root. The initial implementation uses a complete filesystem traversal because an
//! empty Windows Search result alone cannot prove that a root was indexed.

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::WindowsMediaIndex;
