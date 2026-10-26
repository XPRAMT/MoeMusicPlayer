//! The installed font files of this system (ADR 0042). Only names and stat calls: nothing here opens a font.

use crate::diff::FileStamp;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Font files the pickers read (bitmap `.fon` / `.fnt` and Type 1 fonts are left out).
fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc" | "otc"
            )
        })
}

/// The size and modification time of `path`, or `None` if it is not a readable file.
pub fn file_stamp(path: &Path) -> Option<FileStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_millis() as i64);
    Some(FileStamp {
        path: path.to_string_lossy().into_owned(),
        size_bytes: metadata.len(),
        modified_ms,
    })
}

/// A registry font value resolved against its key's default folder (values may be bare file names).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn resolve_registry_value(value: &str, default_dir: &Path) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        default_dir.join(path)
    };
    is_font_file(&resolved).then_some(resolved)
}

/// Keep the first of paths that differ only in letter case (Windows paths are case-insensitive).
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn dedupe_case_insensitive(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path.to_string_lossy().to_lowercase()))
        .collect()
}

/// Folders scanned for system fonts on Android (plus whatever `SystemFonts.getAvailableFonts()` reports).
pub fn android_font_directories() -> &'static [&'static str] {
    &[
        "/system/fonts",
        "/product/fonts",
        "/system_ext/fonts",
        "/vendor/fonts",
        "/data/fonts/files",
    ]
}

/// Font files below `directory`, `depth` folder levels deep (updatable Android fonts sit one level down).
#[cfg_attr(target_os = "windows", allow(dead_code))]
fn font_files_below(directory: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() && depth > 0 => font_files_below(&path, depth - 1, out),
            Ok(kind) if kind.is_file() && is_font_file(&path) => out.push(path),
            _ => {}
        }
    }
}

/// The installed font files: on Windows the per-user (HKCU) and machine (HKLM)
/// `Software\Microsoft\Windows NT\CurrentVersion\Fonts` values, resolved against
/// `%LOCALAPPDATA%\Microsoft\Windows\Fonts` and `%WINDIR%\Fonts`; on Android the system font folders.
pub fn system_font_files() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        const KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";
        let user_dir = std::env::var_os("LOCALAPPDATA")
            .map(|dir| PathBuf::from(dir).join(r"Microsoft\Windows\Fonts"))
            .unwrap_or_default();
        let system_dir = std::env::var_os("WINDIR")
            .map(|dir| PathBuf::from(dir).join("Fonts"))
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\Fonts"));
        let mut paths = Vec::new();
        for (root, default_dir) in [
            (windows_registry::CURRENT_USER, &user_dir),
            (windows_registry::LOCAL_MACHINE, &system_dir),
        ] {
            let Ok(key) = root.open(KEY) else { continue };
            let Ok(values) = key.values() else { continue };
            for (_, value) in values {
                if let Ok(text) = String::try_from(value) {
                    paths.extend(resolve_registry_value(&text, default_dir));
                }
            }
        }
        dedupe_case_insensitive(paths)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut paths = Vec::new();
        for directory in android_font_directories() {
            font_files_below(Path::new(directory), 2, &mut paths);
        }
        paths.sort();
        paths.dedup();
        paths
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_values_resolve_against_the_default_folder() {
        let base = Path::new("/fonts");
        assert_eq!(
            resolve_registry_value("arial.ttf", base),
            Some(base.join("arial.ttf"))
        );
        assert_eq!(
            resolve_registry_value("msjh.TTC", base),
            Some(base.join("msjh.TTC"))
        );
        let absolute = std::env::temp_dir().join("user.otf");
        assert_eq!(
            resolve_registry_value(&absolute.to_string_lossy(), base),
            Some(absolute)
        );
        assert_eq!(resolve_registry_value("vgasys.fon", base), None);
        assert_eq!(resolve_registry_value("  ", base), None);
    }

    #[test]
    fn duplicate_paths_differing_in_case_are_dropped() {
        let paths = dedupe_case_insensitive(vec![
            PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
            PathBuf::from(r"C:\WINDOWS\Fonts\ARIAL.TTF"),
            PathBuf::from(r"C:\Windows\Fonts\msjh.ttc"),
        ]);
        assert_eq!(paths.len(), 2);
    }

    #[test]
    fn stamps_and_folder_walks() {
        let directory =
            std::env::temp_dir().join(format!("player-fonts-enum-{}", std::process::id()));
        let nested = directory.join("abc");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(directory.join("a.ttf"), b"1234").unwrap();
        std::fs::write(directory.join("notes.txt"), b"x").unwrap();
        std::fs::write(nested.join("b.otf"), b"12").unwrap();
        let mut found = Vec::new();
        font_files_below(&directory, 1, &mut found);
        found.sort();
        assert_eq!(found, vec![directory.join("a.ttf"), nested.join("b.otf")]);
        let stamp = file_stamp(&directory.join("a.ttf")).unwrap();
        assert_eq!(stamp.size_bytes, 4);
        assert!(stamp.modified_ms > 0);
        assert!(file_stamp(&directory).is_none());
        std::fs::remove_dir_all(&directory).ok();
    }
}
