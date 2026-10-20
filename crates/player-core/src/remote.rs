//! Reading files of remote sources from code that knows nothing about servers (ADR 0026): the audio engine, the cover and
//! lyrics lookups and the tag reader all ask the one [`RemoteContent`] the app installs. A file is named the way a
//! [`MediaLocator::Remote`](crate::MediaLocator::Remote) names it: the source it belongs to and its path under that
//! source's root.

use std::io::{self, Read, Seek};
use std::sync::{Arc, PoisonError, RwLock};

use crate::SourceId;

/// A stream that can be moved to another thread, read and seeked.
pub trait RemoteStream: Read + Seek + Send {}
impl<T: Read + Seek + Send> RemoteStream for T {}

/// An open remote file.
pub struct RemoteFileHandle {
    pub reader: Box<dyn RemoteStream>,
    pub len: u64,
}

pub trait RemoteContent: Send + Sync {
    /// Open `path` (relative to the root of `source_id`, `/`-separated) for reading, cached and tuned for the
    /// protocol. `NotFound` when the file or the source does not exist.
    fn open(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle>;
}

static CONTENT: RwLock<Option<Arc<dyn RemoteContent>>> = RwLock::new(None);

/// Install the provider (once at start-up; a second call replaces it).
pub fn set_remote_content(provider: Arc<dyn RemoteContent>) {
    *CONTENT.write().unwrap_or_else(PoisonError::into_inner) = Some(provider);
}

/// The installed provider, if any.
pub fn remote_content() -> Option<Arc<dyn RemoteContent>> {
    CONTENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Read at most `limit` bytes of a remote file, or `None` when it is missing or larger than `limit` (for sidecars such
/// as `cover.jpg` and `.lrc`, which must not be loaded without a bound).
pub fn read_remote_file_capped(source_id: SourceId, path: &str, limit: u64) -> Option<Vec<u8>> {
    let handle = remote_content()?.open(source_id, path).ok()?;
    if handle.len == 0 || handle.len > limit {
        return None;
    }
    let mut bytes = Vec::with_capacity(handle.len as usize);
    handle.reader.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// `a/b/c.flac` + `cover.jpg` → `a/b/cover.jpg`: a file beside a remote track.
pub fn remote_sibling(path: &str, name: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name.to_owned(),
    }
}

/// `a/b/c.flac` → `a/b/c.lrc`: the same name with another extension.
pub fn remote_with_extension(path: &str, extension: &str) -> String {
    let (directory, file) = match path.rsplit_once('/') {
        Some((parent, file)) => (Some(parent), file),
        None => (None, path),
    };
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    match directory {
        Some(parent) => format!("{parent}/{stem}.{extension}"),
        None => format!("{stem}.{extension}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siblings_and_extensions_stay_in_the_same_remote_folder() {
        assert_eq!(remote_sibling("a/b/c.flac", "cover.jpg"), "a/b/cover.jpg");
        assert_eq!(remote_sibling("c.flac", "cover.jpg"), "cover.jpg");
        assert_eq!(remote_with_extension("a/b/c.flac", "lrc"), "a/b/c.lrc");
        assert_eq!(remote_with_extension("a/b.c/d", "lrc"), "a/b.c/d.lrc");
        assert_eq!(remote_with_extension("c.tar.gz", "lrc"), "c.tar.lrc");
        assert_eq!(remote_with_extension("noext", "lrc"), "noext.lrc");
    }
}
