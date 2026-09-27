use std::path::PathBuf;

use player_core::MediaLocator;

use crate::DatabaseError;

pub(crate) struct StoredLocator {
    pub kind: &'static str,
    pub encoding: &'static str,
    pub data: Vec<u8>,
}

pub(crate) fn encode(locator: &MediaLocator) -> StoredLocator {
    match locator {
        MediaLocator::ContentUri(uri) => StoredLocator {
            kind: "content_uri",
            encoding: "utf8",
            data: uri.as_bytes().to_vec(),
        },
        MediaLocator::FileSystem(path) => encode_path(path),
    }
}

#[cfg(windows)]
fn encode_path(path: &std::path::Path) -> StoredLocator {
    use std::os::windows::ffi::OsStrExt;

    let data = path
        .as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect();
    StoredLocator {
        kind: "filesystem_path",
        encoding: "windows_utf16le",
        data,
    }
}

#[cfg(unix)]
fn encode_path(path: &std::path::Path) -> StoredLocator {
    use std::os::unix::ffi::OsStrExt;

    StoredLocator {
        kind: "filesystem_path",
        encoding: "unix_os_bytes",
        data: path.as_os_str().as_bytes().to_vec(),
    }
}

#[cfg(not(any(windows, unix)))]
fn encode_path(path: &std::path::Path) -> StoredLocator {
    match path.to_str() {
        Some(value) => StoredLocator {
            kind: "filesystem_path",
            encoding: "utf8",
            data: value.as_bytes().to_vec(),
        },
        None => StoredLocator {
            kind: "filesystem_path",
            encoding: "unsupported_native_path",
            data: Vec::new(),
        },
    }
}

pub(crate) fn decode(
    kind: &str,
    encoding: &str,
    data: &[u8],
) -> Result<MediaLocator, DatabaseError> {
    if kind == "content_uri" {
        return String::from_utf8(data.to_vec())
            .map(MediaLocator::ContentUri)
            .map_err(|_| DatabaseError::CorruptData("stored content URI is not UTF-8".to_owned()));
    }
    if kind != "filesystem_path" {
        return Err(DatabaseError::CorruptData(format!(
            "unknown locator kind: {kind}"
        )));
    }

    let path = decode_path(encoding, data)?;
    Ok(MediaLocator::FileSystem(path))
}

#[cfg(windows)]
fn decode_path(encoding: &str, data: &[u8]) -> Result<PathBuf, DatabaseError> {
    use std::os::windows::ffi::OsStringExt;

    if encoding != "windows_utf16le" || !data.len().is_multiple_of(2) {
        return Err(DatabaseError::UnsupportedLocatorEncoding(
            encoding.to_owned(),
        ));
    }
    let words = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&words)))
}

#[cfg(unix)]
fn decode_path(encoding: &str, data: &[u8]) -> Result<PathBuf, DatabaseError> {
    use std::os::unix::ffi::OsStringExt;

    match encoding {
        "unix_os_bytes" => Ok(PathBuf::from(std::ffi::OsString::from_vec(data.to_vec()))),
        "utf8" => String::from_utf8(data.to_vec())
            .map(PathBuf::from)
            .map_err(|_| DatabaseError::CorruptData("stored path is not UTF-8".to_owned())),
        _ => Err(DatabaseError::UnsupportedLocatorEncoding(
            encoding.to_owned(),
        )),
    }
}

#[cfg(not(any(windows, unix)))]
fn decode_path(encoding: &str, data: &[u8]) -> Result<PathBuf, DatabaseError> {
    match encoding {
        "utf8" => String::from_utf8(data.to_vec())
            .map(PathBuf::from)
            .map_err(|_| DatabaseError::CorruptData("stored path is not UTF-8".to_owned())),
        _ => Err(DatabaseError::UnsupportedLocatorEncoding(
            encoding.to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, encode};
    use player_core::MediaLocator;
    use std::path::PathBuf;

    #[test]
    fn unicode_filesystem_path_round_trips_without_display_string_conversion() {
        let locator =
            MediaLocator::FileSystem(PathBuf::from(r"C:\音樂\專輯 🎧\演奏者 - 曲目 01.flac"));
        let stored = encode(&locator);
        let decoded = decode(stored.kind, stored.encoding, &stored.data).expect("decode locator");
        assert_eq!(decoded, locator);
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_round_trip_preserves_unpaired_utf16_code_unit() {
        use std::os::windows::ffi::OsStringExt;

        let os = std::ffi::OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xD800]);
        let locator = MediaLocator::FileSystem(PathBuf::from(os));
        let stored = encode(&locator);
        let decoded = decode(stored.kind, stored.encoding, &stored.data).expect("decode locator");
        assert_eq!(decoded, locator);
    }
}
