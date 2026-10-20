//! The in-app folder browser for Android (the system folder picker refuses a storage root).
//!
//! The system picker (`ACTION_OPEN_DOCUMENT_TREE`) does not let the user choose the root of the internal storage or of
//! an SD card ("為保護你的隱私，請選擇其他資料夾"), even though the app holds All files access (ADR 0014) and reads real
//! paths. So when that access is granted the folder choice is made in the app's own dialog: the dialog lists real
//! directories through [`android_list_directories`] and answers the waiting request through
//! [`android_folder_picker_result`]. Every flow that used to call the system picker (adding a source, excluded folders,
//! the favourites folder, the slideshow folder) goes through [`pick_directory`] and keeps working unchanged.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;

/// How long the backend waits for the user to choose.
#[cfg(target_os = "android")]
const PICK_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryRoot {
    pub label: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryEntry {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectoryListing {
    pub path: String,
    /// `None` at the top of a storage volume.
    pub parent: Option<String>,
    pub roots: Vec<DirectoryRoot>,
    pub entries: Vec<DirectoryEntry>,
}

/// The storage volumes a folder may be chosen in: the internal storage and every other volume under `/storage`.
#[cfg(target_os = "android")]
pub(crate) fn android_roots() -> Vec<(String, PathBuf)> {
    let mut roots = vec![("內部儲存空間".to_owned(), PathBuf::from("/storage/emulated/0"))];
    if let Ok(entries) = std::fs::read_dir("/storage") {
        let mut volumes: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name != "emulated" && name != "self")
            })
            .collect();
        volumes.sort();
        for volume in volumes {
            let name = volume
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            roots.push((format!("記憶卡（{name}）"), volume));
        }
    }
    roots
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// List the sub-folders of `path` (the first root when `None`). The path must lie inside one of `roots`; hidden folders
/// (names starting with a dot) are left out, the rest is sorted by name.
pub(crate) fn list_directory(
    path: Option<&str>,
    roots: &[(String, PathBuf)],
) -> Result<DirectoryListing, String> {
    let canonical_roots: Vec<(String, PathBuf)> = roots
        .iter()
        .filter_map(|(label, root)| Some((label.clone(), std::fs::canonicalize(root).ok()?)))
        .collect();
    let first = canonical_roots
        .first()
        .ok_or_else(|| "找不到可用的儲存空間。".to_owned())?;
    let requested = match path {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => first.1.clone(),
    };
    let directory = std::fs::canonicalize(&requested)
        .map_err(|error| format!("無法開啟這個資料夾：{error}"))?;
    let root = canonical_roots
        .iter()
        .find(|(_, root)| directory.starts_with(root))
        .ok_or_else(|| "只能瀏覽裝置的儲存空間。".to_owned())?;
    if !directory.is_dir() {
        return Err("這不是資料夾。".to_owned());
    }
    let mut entries: Vec<DirectoryEntry> = std::fs::read_dir(&directory)
        .map_err(|error| format!("無法讀取這個資料夾：{error}"))?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            // A symbolic link to a folder counts as a folder.
            std::fs::metadata(&path).ok()?.is_dir().then(|| DirectoryEntry {
                name,
                path: path_text(&path),
            })
        })
        .collect();
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    Ok(DirectoryListing {
        path: path_text(&directory),
        parent: (directory != root.1)
            .then(|| directory.parent().map(path_text))
            .flatten(),
        roots: canonical_roots
            .iter()
            .map(|(label, path)| DirectoryRoot {
                label: label.clone(),
                path: path_text(path),
            })
            .collect(),
        entries,
    })
}

/// Requests waiting for the dialog's answer, by request id.
fn pending() -> &'static Mutex<HashMap<u64, Sender<Option<String>>>> {
    static PENDING: OnceLock<Mutex<HashMap<u64, Sender<Option<String>>>>> = OnceLock::new();
    PENDING.get_or_init(Mutex::default)
}

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Register a request; the receiver gets the chosen path (`None` when cancelled). The returned id goes to the dialog.
fn open_request() -> (u64, std::sync::mpsc::Receiver<Option<String>>) {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = channel();
    pending()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id, sender);
    (id, receiver)
}

/// Answer a request. Returns false for an unknown or already answered id.
fn answer_request(id: u64, path: Option<String>) -> bool {
    pending()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id)
        .is_some_and(|sender| sender.send(path).is_ok())
}

/// Ask the dialog in the page to choose a folder and wait for the answer (`None` = cancelled).
#[cfg(target_os = "android")]
fn ask_dialog(app: &tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::Emitter as _;
    let (id, receiver) = open_request();
    if let Err(error) = app.emit("android-folder-picker-request", id) {
        pending()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&id);
        return Err(format!("無法開啟資料夾選擇視窗：{error}"));
    }
    match receiver.recv_timeout(PICK_TIMEOUT) {
        Ok(choice) => Ok(choice),
        Err(_) => {
            pending()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&id);
            Ok(None)
        }
    }
}

/// Choose a folder: in the app's own dialog when All files access is granted (any folder, a storage root too),
/// otherwise through the system picker. Blocking: call it from `spawn_blocking`.
#[cfg(target_os = "android")]
pub(crate) fn pick_directory(app: &tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_media_index::MediaIndexExt as _;
    let granted = app
        .media_index()
        .all_files_access_status()
        .map(|status| status.granted)
        .unwrap_or(false);
    if granted {
        ask_dialog(app)
    } else {
        app.media_index()
            .pick_directory_path()
            .map_err(|error| error.to_string())
    }
}

/// The sub-folders of a folder, for the folder dialog (Android).
#[tauri::command]
pub(crate) async fn android_list_directories(path: Option<String>) -> Result<DirectoryListing, String> {
    #[cfg(target_os = "android")]
    {
        return tauri::async_runtime::spawn_blocking(move || {
            list_directory(path.as_deref(), &android_roots())
        })
        .await
        .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = path;
        Err("資料夾瀏覽只適用於 Android。".to_owned())
    }
}

/// The folder dialog's answer: the chosen path, or `null` when it was cancelled.
#[tauri::command]
pub(crate) fn android_folder_picker_result(request_id: u64, path: Option<String>) -> bool {
    answer_request(request_id, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("moe-folder-browser-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn lists_sub_folders_sorted_without_hidden_ones_or_files() {
        let root = temp_root("list");
        for name in ["Music", "alarms", "DCIM", ".thumbnails"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::write(root.join("a.txt"), b"x").unwrap();
        let roots = vec![("內部儲存空間".to_owned(), root.clone())];
        let listing = list_directory(None, &roots).unwrap();
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alarms", "DCIM", "Music"]);
        assert_eq!(listing.parent, None, "the root has no parent");
        assert_eq!(listing.roots.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn walks_down_and_up_and_the_root_itself_can_be_chosen() {
        let root = temp_root("walk");
        std::fs::create_dir_all(root.join("Music").join("CD")).unwrap();
        let roots = vec![("內部儲存空間".to_owned(), root.clone())];
        let top = list_directory(None, &roots).unwrap();
        let music = list_directory(Some(&top.entries[0].path), &roots).unwrap();
        assert_eq!(music.entries[0].name, "CD");
        assert_eq!(music.parent.as_deref(), Some(top.path.as_str()));
        let up = list_directory(music.parent.as_deref(), &roots).unwrap();
        assert_eq!(up.path, top.path);
        assert_eq!(up.parent, None);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn paths_outside_the_storage_roots_and_non_folders_are_refused() {
        let root = temp_root("guard");
        let other = temp_root("guard-other");
        std::fs::write(root.join("file.txt"), b"x").unwrap();
        let roots = vec![("內部儲存空間".to_owned(), root.clone())];
        assert!(list_directory(other.to_str(), &roots).is_err());
        assert!(list_directory(Some(&format!("{}/..", root.display())), &roots).is_err());
        assert!(list_directory(root.join("file.txt").to_str(), &roots).is_err());
        assert!(list_directory(root.join("missing").to_str(), &roots).is_err());
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn a_request_is_answered_once_with_the_chosen_path_or_nothing() {
        let (id, receiver) = open_request();
        assert!(answer_request(id, Some("/storage/emulated/0".to_owned())));
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), Some("/storage/emulated/0".to_owned()));
        assert!(!answer_request(id, None), "already answered");
        let (id, receiver) = open_request();
        assert!(answer_request(id, None));
        assert_eq!(receiver.recv_timeout(Duration::from_secs(1)).unwrap(), None);
        assert!(!answer_request(u64::MAX, None), "unknown id");
    }
}
