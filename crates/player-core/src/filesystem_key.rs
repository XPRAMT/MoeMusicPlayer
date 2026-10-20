//! Platform-neutral identity helpers for filesystem library items (Android uses these; Windows
//! keeps `windows_locator_key`, ADR 0014).

#[cfg(unix)]
use std::path::Path;

/// Stable source item ID and locator key for a native (non-Windows) filesystem path. Valid UTF-8
/// paths keep their text so diagnostics stay readable; other byte sequences are hex-encoded.
#[cfg(unix)]
pub fn unix_path_key(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    match path.to_str() {
        Some(value) => format!("unix-path-v1:{value}"),
        None => {
            let bytes = path.as_os_str().as_bytes();
            let mut key = String::with_capacity("unix-bytes-v1:".len() + bytes.len() * 2);
            key.push_str("unix-bytes-v1:");
            for byte in bytes {
                use std::fmt::Write as _;
                let _ = write!(key, "{byte:02x}");
            }
            key
        }
    }
}

/// Decode a `unix_path_key` back to its display path (UTF-8 keys only).
pub fn unix_path_from_key(key: &str) -> Option<&str> {
    key.strip_prefix("unix-path-v1:")
}

/// `file://` URI for an absolute path, percent-encoding everything except unreserved characters
/// and `/`, so Android `Uri.parse` / Media3 / ContentResolver read it back losslessly.
#[cfg(unix)]
pub fn file_uri_for_path(path: &Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;

    if !path.is_absolute() {
        return None;
    }
    let bytes = path.as_os_str().as_bytes();
    let mut uri = String::with_capacity("file://".len() + bytes.len() * 3);
    uri.push_str("file://");
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'.' | b'_' | b'~') {
            uri.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    Some(uri)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn unix_path_key_keeps_utf8_text_and_round_trips_for_display() {
        let key = unix_path_key(Path::new("/storage/emulated/0/Music/春意 紅包.flac"));
        assert_eq!(key, "unix-path-v1:/storage/emulated/0/Music/春意 紅包.flac");
        assert_eq!(
            unix_path_from_key(&key),
            Some("/storage/emulated/0/Music/春意 紅包.flac")
        );
    }

    #[test]
    fn file_uri_percent_encodes_reserved_and_non_ascii_bytes() {
        let uri = file_uri_for_path(Path::new("/storage/emulated/0/Music/a #1?é.mp3")).unwrap();
        assert_eq!(uri, "file:///storage/emulated/0/Music/a%20%231%3F%C3%A9.mp3");
        assert_eq!(file_uri_for_path(Path::new("relative.mp3")), None);
    }
}
