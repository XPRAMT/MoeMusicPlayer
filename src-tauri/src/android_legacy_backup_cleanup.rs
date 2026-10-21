//! Remove snapshots created by the retired Android source-path migration.
//!
//! This deliberately inspects only direct children of the app data directory. It removes only
//! real directories whose names match the timestamped `pre-path-migration-<milliseconds>` form
//! produced by that migration; it never follows directory symlinks or touches settings/database
//! files outside those snapshots.

use std::fs;
use std::io;
use std::path::Path;

const LEGACY_BACKUP_PREFIX: &str = "pre-path-migration-";

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct LegacyBackupCleanupReport {
    pub(crate) directories_before: usize,
    /// Sum of regular file lengths inside matched snapshots, not filesystem block allocation.
    pub(crate) file_bytes_before: u64,
    pub(crate) directories_after: usize,
    pub(crate) file_bytes_after: u64,
}

pub(crate) fn remove_legacy_source_migration_backups(
    app_data_dir: &Path,
) -> io::Result<LegacyBackupCleanupReport> {
    let candidates = matching_backup_directories(app_data_dir)?;
    let file_bytes_before = candidates.iter().try_fold(0_u64, |total, path| {
        directory_file_bytes(path).map(|bytes| total.saturating_add(bytes))
    })?;

    for path in &candidates {
        fs::remove_dir_all(path)?;
    }

    let remaining = matching_backup_directories(app_data_dir)?;
    let file_bytes_after = remaining.iter().try_fold(0_u64, |total, path| {
        directory_file_bytes(path).map(|bytes| total.saturating_add(bytes))
    })?;
    Ok(LegacyBackupCleanupReport {
        directories_before: candidates.len(),
        file_bytes_before,
        directories_after: remaining.len(),
        file_bytes_after,
    })
}

fn matching_backup_directories(app_data_dir: &Path) -> io::Result<Vec<std::path::PathBuf>> {
    let entries = match fs::read_dir(app_data_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut matching = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(timestamp) = name.strip_prefix(LEGACY_BACKUP_PREFIX) else {
            continue;
        };
        if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        // DirEntry::file_type does not follow symlinks. Only directories made by the old
        // migrator are eligible; a similarly named regular file or directory symlink is left alone.
        if entry.file_type()?.is_dir() {
            matching.push(entry.path());
        }
    }
    Ok(matching)
}

fn directory_file_bytes(directory: &Path) -> io::Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            total = total.saturating_add(directory_file_bytes(&entry.path())?);
        } else if kind.is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    fn test_directory() -> std::path::PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "moe-legacy-backup-cleanup-{}-{timestamp}-{}",
            std::process::id(),
            NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn removes_only_timestamped_direct_child_backup_directories() {
        let root = test_directory();
        let backup = root.join("pre-path-migration-1790000000123");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("moemusicplayer.sqlite3"), b"old snapshot").unwrap();
        fs::write(backup.join("settings.json"), b"old settings").unwrap();

        let unrelated_directory = root.join("other-backup-1790000000123");
        fs::create_dir_all(&unrelated_directory).unwrap();
        fs::write(unrelated_directory.join("keep.txt"), b"keep").unwrap();

        let non_timestamped_directory = root.join("pre-path-migration-notes");
        fs::create_dir_all(&non_timestamped_directory).unwrap();
        fs::write(non_timestamped_directory.join("keep.txt"), b"keep").unwrap();

        let matching_name_file = root.join("pre-path-migration-1790000000456");
        fs::write(&matching_name_file, b"not a directory").unwrap();

        let nested_backup = root.join("nested/pre-path-migration-1790000000789");
        fs::create_dir_all(&nested_backup).unwrap();
        fs::write(nested_backup.join("keep.txt"), b"keep").unwrap();

        let database = root.join("moemusicplayer.sqlite3");
        let settings = root.join("settings.json");
        fs::write(&database, b"current database").unwrap();
        fs::write(&settings, b"current settings").unwrap();

        assert_eq!(
            remove_legacy_source_migration_backups(&root).unwrap(),
            LegacyBackupCleanupReport {
                directories_before: 1,
                file_bytes_before: b"old snapshot".len() as u64 + b"old settings".len() as u64,
                directories_after: 0,
                file_bytes_after: 0,
            }
        );
        assert!(!backup.exists());
        assert_eq!(
            fs::read(unrelated_directory.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(
            fs::read(non_timestamped_directory.join("keep.txt")).unwrap(),
            b"keep"
        );
        assert_eq!(fs::read(&matching_name_file).unwrap(), b"not a directory");
        assert_eq!(fs::read(nested_backup.join("keep.txt")).unwrap(), b"keep");
        assert_eq!(fs::read(database).unwrap(), b"current database");
        assert_eq!(fs::read(settings).unwrap(), b"current settings");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_app_data_directory_is_a_no_op() {
        let root = test_directory();
        assert_eq!(
            remove_legacy_source_migration_backups(&root).unwrap(),
            LegacyBackupCleanupReport::default()
        );
        assert!(!root.exists());
    }
}
