//! Whether a local file's times are absolute instants (ADR 0013): the details panel shows the time-zone suffix only when
//! they may be a wall-clock time without a zone (FAT, exFAT, an unknown file system).
//!
//! The decisions are pure functions over a file-system name or a mount table; the OS calls that feed them are thin.
//! Windows asks the volume of the path for its file-system name (`GetVolumePathNameW` → `GetVolumeInformationW`); a network
//! share mounted as a drive or reached by a UNC path reports the name the server gives (usually "NTFS"), which is followed
//! as is. Android (and Linux) reads `/proc/self/mountinfo` (or `/proc/mounts`) and takes the mount with the longest mount
//! point that contains the path; a layered mount (FUSE, sdcardfs) hides its backing file system, see
//! [`mount_times_absolute`].

/// File systems that store file times as UTC instants. Everything else (FAT, exFAT, NTFS-3G over FUSE, unknown) is not.
const ABSOLUTE_FILE_SYSTEMS: &[&str] = &[
    "ntfs", "refs", "ext2", "ext3", "ext4", "f2fs", "btrfs", "xfs", "apfs", "hfs", "hfsplus",
    "erofs", "tmpfs", "zfs", "jfs", "reiserfs", "nilfs2", "bcachefs",
];

/// Layered file systems that show another file system's files and do not tell its type.
const LAYERED_FILE_SYSTEMS: &[&str] = &["fuse", "fuseblk", "sdcardfs", "esdfs"];

/// The file system keeps absolute (UTC) times. Case-insensitive; unknown names are not absolute.
pub fn file_system_keeps_absolute_times(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    ABSOLUTE_FILE_SYSTEMS.contains(&name.as_str())
}

fn is_layered(name: &str) -> bool {
    let name = name.trim().to_ascii_lowercase();
    // `fuse.<helper>` (e.g. `fuse.ntfs-3g`) is a FUSE mount too.
    LAYERED_FILE_SYSTEMS.contains(&name.as_str()) || name.starts_with("fuse.")
}

/// One mount of a mount table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub mount_point: String,
    pub fs_type: String,
}

/// `mount_point` contains `path` (as whole path components).
fn contains(mount_point: &str, path: &str) -> bool {
    if mount_point == "/" {
        return path.starts_with('/');
    }
    let mount_point = mount_point.trim_end_matches('/');
    path == mount_point
        || path
            .strip_prefix(mount_point)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The mount holding `path`: the one with the longest mount point that contains it (the last one wins a tie, as a later
/// mount covers an earlier one on the same point).
fn mount_of<'a>(path: &str, mounts: &'a [Mount]) -> Option<&'a Mount> {
    let mut best: Option<&Mount> = None;
    for mount in mounts {
        if contains(&mount.mount_point, path)
            && best.is_none_or(|current| mount.mount_point.len() >= current.mount_point.len())
        {
            best = Some(mount);
        }
    }
    best
}

/// The times of the file at `path` (an absolute, resolved path) are absolute, by the mount table.
///
/// A layered mount (Android's FUSE `/storage`, older sdcardfs) hides the backing type, so:
/// - under `/storage/emulated` (or `/sdcard`, `/mnt/user/<n>/emulated`) the backing is the data partition: the type of the
///   mount holding `/data/media` decides, and when the table does not show it the path counts as absolute (Android's data
///   partition is always f2fs or ext4);
/// - under `/storage/<volume>` (an SD card, a USB drive) the backing is mounted at `/mnt/media_rw/<volume>`: its type
///   decides when the table shows exactly that mount (often vfat or exfat), otherwise it is not absolute;
/// - any other layered mount is not absolute.
pub fn mount_times_absolute(path: &str, mounts: &[Mount]) -> bool {
    let Some(mount) = mount_of(path, mounts) else {
        return false;
    };
    if !is_layered(&mount.fs_type) {
        return file_system_keeps_absolute_times(&mount.fs_type);
    }
    let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let emulated = matches!(
        parts.as_slice(),
        ["storage", "emulated", ..] | ["sdcard", ..]
    ) || matches!(parts.as_slice(), ["mnt", "user", _, "emulated", ..]);
    if emulated {
        return match mount_of("/data/media", mounts) {
            Some(backing) if !is_layered(&backing.fs_type) => {
                file_system_keeps_absolute_times(&backing.fs_type)
            }
            _ => true,
        };
    }
    let volume = match parts.as_slice() {
        ["storage", volume, ..] => Some(*volume),
        ["mnt", "user", _, volume, ..] => Some(*volume),
        _ => None,
    };
    let Some(volume) = volume else {
        return false;
    };
    let backing_point = format!("/mnt/media_rw/{volume}");
    match mount_of(&backing_point, mounts) {
        Some(backing) if backing.mount_point.trim_end_matches('/') == backing_point => {
            !is_layered(&backing.fs_type) && file_system_keeps_absolute_times(&backing.fs_type)
        }
        _ => false,
    }
}

/// Undo the octal escapes (`\040` for a space) of a mount table field.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &bytes[i + 1..i + 4];
            if digits.iter().all(|b| (b'0'..=b'7').contains(b)) {
                let value = digits
                    .iter()
                    .fold(0u32, |acc, b| acc * 8 + u32::from(b - b'0'));
                if let Ok(value) = u8::try_from(value) {
                    out.push(value);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `/proc/self/mountinfo` (`id parent major:minor root mount_point options [optional…] - fs_type source super`).
pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let mount_point = left.split(' ').nth(4)?;
            let fs_type = right.split(' ').next()?;
            Some(Mount {
                mount_point: unescape(mount_point),
                fs_type: fs_type.to_owned(),
            })
        })
        .collect()
}

/// Parse `/proc/mounts` (`source mount_point fs_type options dump pass`).
pub fn parse_proc_mounts(text: &str) -> Vec<Mount> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split(' ');
            let _source = fields.next()?;
            let mount_point = fields.next()?;
            let fs_type = fields.next()?;
            Some(Mount {
                mount_point: unescape(mount_point),
                fs_type: fs_type.to_owned(),
            })
        })
        .collect()
}

/// The times of the local file at `path` are absolute instants. Detection failing counts as not absolute.
pub fn times_absolute(path: &std::path::Path) -> bool {
    os::times_absolute(path)
}

#[cfg(windows)]
mod os {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW};

    /// The file-system name of the volume holding `path` (e.g. "NTFS", "FAT32", "exFAT").
    fn file_system_name(path: &Path) -> Option<String> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        // The volume path is never longer than the path itself (plus a trailing separator).
        let mut root = vec![0u16; wide.len().max(261) + 1];
        // SAFETY: `wide` is NUL-terminated and `root` is a writable buffer of the given length.
        unsafe { GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut root) }.ok()?;
        let mut name = [0u16; 261];
        // SAFETY: `root` was NUL-terminated by GetVolumePathNameW; `name` is a writable buffer.
        unsafe {
            GetVolumeInformationW(
                PCWSTR(root.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut name),
            )
        }
        .ok()?;
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        Some(String::from_utf16_lossy(&name[..len]))
    }

    pub fn times_absolute(path: &Path) -> bool {
        match file_system_name(path) {
            Some(name) => super::file_system_keeps_absolute_times(&name),
            None => {
                eprintln!("[details] no file system name for {}", path.display());
                false
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_volume_of_a_path_answers_its_name() {
            // The type depends on the machine; only that the calls answer, also for a long (verbatim) path.
            let here = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
            let name = file_system_name(&here).expect("a file system name");
            assert!(!name.is_empty());
            let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", here.display()));
            assert_eq!(file_system_name(&verbatim).as_deref(), Some(name.as_str()));
        }
    }
}

#[cfg(any(target_os = "android", target_os = "linux"))]
mod os {
    use std::path::Path;

    pub fn times_absolute(path: &Path) -> bool {
        let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let Some(resolved) = resolved.to_str() else {
            return false;
        };
        let mounts = match std::fs::read_to_string("/proc/self/mountinfo") {
            Ok(text) => super::parse_mountinfo(&text),
            Err(_) => match std::fs::read_to_string("/proc/mounts") {
                Ok(text) => super::parse_proc_mounts(&text),
                Err(error) => {
                    eprintln!("[details] no mount table: {error}");
                    return false;
                }
            },
        };
        super::mount_times_absolute(resolved, &mounts)
    }
}

#[cfg(not(any(windows, target_os = "android", target_os = "linux")))]
mod os {
    pub fn times_absolute(_path: &std::path::Path) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mount(point: &str, fs: &str) -> Mount {
        Mount {
            mount_point: point.to_owned(),
            fs_type: fs.to_owned(),
        }
    }

    #[test]
    fn windows_file_system_names() {
        assert!(file_system_keeps_absolute_times("NTFS"));
        assert!(file_system_keeps_absolute_times("ReFS"));
        assert!(!file_system_keeps_absolute_times("FAT"));
        assert!(!file_system_keeps_absolute_times("FAT32"));
        assert!(!file_system_keeps_absolute_times("exFAT"));
        assert!(!file_system_keeps_absolute_times("UDF"));
        assert!(!file_system_keeps_absolute_times(""));
    }

    /// A phone with Android's FUSE `/storage` over an f2fs data partition and an exFAT SD card.
    fn phone_mounts() -> Vec<Mount> {
        vec![
            mount("/", "ext4"),
            mount("/data", "f2fs"),
            mount("/mnt/media_rw/1234-ABCD", "exfat"),
            mount("/mnt/media_rw/5678-EF01", "vfat"),
            mount("/storage", "tmpfs"),
            mount("/storage/emulated", "fuse"),
            mount("/storage/1234-ABCD", "fuse"),
            mount("/storage/5678-EF01", "fuse"),
        ]
    }

    #[test]
    fn android_internal_storage_is_absolute() {
        let mounts = phone_mounts();
        assert!(mount_times_absolute(
            "/storage/emulated/0/Music/a.flac",
            &mounts
        ));
        assert!(mount_times_absolute("/data/user/0/app/a.flac", &mounts));
        // The backing type decides: a data partition the table shows as not absolute is not.
        let odd = vec![mount("/data", "vfat"), mount("/storage/emulated", "fuse")];
        assert!(!mount_times_absolute("/storage/emulated/0/a.flac", &odd));
        // The app's namespace does not show /data: the emulated storage still counts as absolute.
        let hidden = vec![mount("/storage/emulated", "fuse")];
        assert!(mount_times_absolute("/storage/emulated/0/a.flac", &hidden));
        let sdcardfs = vec![mount("/", "ext4"), mount("/storage/emulated", "sdcardfs")];
        assert!(mount_times_absolute(
            "/storage/emulated/0/a.flac",
            &sdcardfs
        ));
    }

    #[test]
    fn android_removable_storage_is_not_absolute() {
        let mounts = phone_mounts();
        assert!(!mount_times_absolute(
            "/storage/1234-ABCD/Music/a.flac",
            &mounts
        ));
        assert!(!mount_times_absolute("/storage/5678-EF01/a.flac", &mounts));
        // The backing mount is hidden: a layered removable volume is not absolute.
        let hidden = vec![mount("/", "ext4"), mount("/storage/1234-ABCD", "fuse")];
        assert!(!mount_times_absolute("/storage/1234-ABCD/a.flac", &hidden));
        // A volume mounted directly as vfat.
        let direct = vec![mount("/", "ext4"), mount("/storage/1234-ABCD", "vfat")];
        assert!(!mount_times_absolute("/storage/1234-ABCD/a.flac", &direct));
        // A layered removable volume over ext4 (an adopted or Linux-formatted card) follows the backing type.
        let ext4_card = vec![
            mount("/", "ext4"),
            mount("/mnt/media_rw/CARD", "ext4"),
            mount("/storage/CARD", "fuse"),
        ];
        assert!(mount_times_absolute("/storage/CARD/a.flac", &ext4_card));
    }

    #[test]
    fn longest_whole_component_prefix_wins() {
        let mounts = vec![mount("/", "ext4"), mount("/mnt/usb", "vfat")];
        assert!(!mount_times_absolute("/mnt/usb/a.flac", &mounts));
        assert!(mount_times_absolute("/mnt/usbdrive/a.flac", &mounts));
        // A later mount on the same point covers the earlier one.
        let covered = vec![
            mount("/", "ext4"),
            mount("/mnt/x", "ext4"),
            mount("/mnt/x", "vfat"),
        ];
        assert!(!mount_times_absolute("/mnt/x/a.flac", &covered));
        assert!(!mount_times_absolute("/a.flac", &[]));
        let ntfs3g = vec![mount("/", "ext4"), mount("/media/disk", "fuseblk")];
        assert!(!mount_times_absolute("/media/disk/a.flac", &ntfs3g));
    }

    #[test]
    fn parses_mount_tables() {
        let mountinfo = "\
22 1 259:2 / / ro,relatime shared:1 - ext4 /dev/block/dm-0 ro,seclabel
40 22 259:40 / /data rw,nosuid master:2 - f2fs /dev/block/dm-48 rw,seclabel
50 40 0:61 / /storage/emulated rw,nosuid shared:3 - fuse /dev/fuse rw,user_id=0
51 40 0:62 / /mnt/my\\040disk rw - vfat /dev/sdb1 rw
bad line";
        assert_eq!(
            parse_mountinfo(mountinfo),
            vec![
                mount("/", "ext4"),
                mount("/data", "f2fs"),
                mount("/storage/emulated", "fuse"),
                mount("/mnt/my disk", "vfat"),
            ]
        );
        let proc_mounts =
            "/dev/block/dm-0 / ext4 ro 0 0\n/dev/fuse /storage/emulated fuse rw 0 0\n";
        assert_eq!(
            parse_proc_mounts(proc_mounts),
            vec![mount("/", "ext4"), mount("/storage/emulated", "fuse")]
        );
    }
}
