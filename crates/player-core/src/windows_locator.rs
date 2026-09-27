use std::path::Path;

/// Return the stable Windows identity key shared by filesystem indexing and playlist matching.
///
/// This is a comparison token, not a display path or a filesystem locator. Separators, drive
/// letter casing, and the extended-length prefix are normalized; component casing and all UTF-16
/// code units are preserved.
pub fn windows_locator_key(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;

    let mut units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    for unit in &mut units {
        if *unit == b'/' as u16 {
            *unit = b'\\' as u16;
        }
    }
    normalize_verbatim_prefix(&mut units);
    if units.len() >= 2 && units[1] == b':' as u16 && units[0] <= 0x7f {
        units[0] = (units[0] as u8).to_ascii_uppercase() as u16;
    }

    let mut key = String::with_capacity("windows-u16-v1:".len() + units.len() * 4);
    key.push_str("windows-u16-v1:");
    for unit in units {
        use std::fmt::Write as _;
        let _ = write!(key, "{unit:04x}");
    }
    key
}

fn normalize_verbatim_prefix(units: &mut Vec<u16>) {
    const VERBATIM: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC: [u16; 4] = [b'U' as u16, b'N' as u16, b'C' as u16, b'\\' as u16];
    if units.len() >= 8
        && units[..4] == VERBATIM
        && units[4..8]
            .iter()
            .zip(UNC)
            .all(|(left, right)| ascii_u16_eq_ignore_case(*left, right))
    {
        let remainder = units[8..].to_vec();
        *units = [b'\\' as u16, b'\\' as u16]
            .into_iter()
            .chain(remainder)
            .collect();
    } else if units.len() >= 4 && units[..4] == VERBATIM {
        units.drain(..4);
    }
}

fn ascii_u16_eq_ignore_case(left: u16, right: u16) -> bool {
    left <= 0x7f && right <= 0x7f && (left as u8).eq_ignore_ascii_case(&(right as u8))
}

#[cfg(test)]
mod tests {
    use super::windows_locator_key;
    use std::path::{Path, PathBuf};

    #[test]
    fn ordinary_and_extended_paths_share_identity_with_unicode_spaces_and_long_names() {
        let path =
            PathBuf::from(r"D:\Music\Playlists\漢字 東京🌸\長檔名 空白與空格-0123456789.flac");
        let verbatim = PathBuf::from(format!(r"\\?\{}", path.display()));
        let forward_slashes =
            PathBuf::from(r"d:/Music/Playlists/漢字 東京🌸/長檔名 空白與空格-0123456789.flac");
        assert_eq!(windows_locator_key(&path), windows_locator_key(&verbatim));
        assert_eq!(
            windows_locator_key(&path),
            windows_locator_key(&forward_slashes)
        );
        assert_eq!(
            windows_locator_key(Path::new(r"\\server\share\音樂 東京\track.flac")),
            windows_locator_key(Path::new(r"\\?\UNC\server\share\音樂 東京\track.flac"))
        );
    }

    #[test]
    fn key_preserves_unpaired_utf16_code_units() {
        use std::os::windows::ffi::OsStringExt;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            b'C' as u16,
            b':' as u16,
            b'\\' as u16,
            0xd800,
            b'\\' as u16,
            b'x' as u16,
        ]));
        assert!(windows_locator_key(&path).contains("d800"));
    }
}
