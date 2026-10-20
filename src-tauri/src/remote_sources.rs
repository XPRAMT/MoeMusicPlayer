//! Remote sources (ADR 0026): testing a connection, listing the shares and folders of a server, and adding a folder of an
//! account to the library. All of it runs off the UI thread because every call waits for a server.

use player_core::{LibraryRoot, MediaLocator, MediaSourceKind, SourceId};
use player_db::natural_title_cmp;
use player_remote::account::RemoteAccount;
use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::remote_accounts::{account_from_input, vault, RemoteAccountInput};
use crate::settings::{SourceEntry, SourceEntryKind, StoredPath};
use crate::{AppState, LibrarySource};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteFolderView {
    pub name: String,
    /// Relative to the account's root, `/`-separated.
    pub path: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteTestView {
    /// The folders at the top of the account, so the user sees it reached the right place.
    pub folders: Vec<String>,
}

/// `a/b/` → `a/b`: the form a folder path is stored in.
fn normalize_folder(path: &str) -> String {
    path.split(['/', '\\'])
        .filter(|piece| !piece.is_empty() && *piece != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn folders_of(
    fs: &dyn player_remote::fs::RemoteFs,
    dir: &str,
) -> Result<Vec<RemoteFolderView>, String> {
    let mut folders: Vec<RemoteFolderView> = fs
        .list(dir)
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|entry| entry.is_dir && !entry.name.starts_with('.'))
        .map(|entry| RemoteFolderView {
            path: if dir.is_empty() {
                entry.name.clone()
            } else {
                format!("{dir}/{}", entry.name)
            },
            name: entry.name,
        })
        .collect();
    folders.sort_by(|left, right| natural_title_cmp(&left.name, &right.name));
    Ok(folders)
}

fn account_of_id(state: &AppState, account_id: &str) -> Result<RemoteAccount, String> {
    state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .remote_accounts
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| "找不到這個遠端帳號。".to_owned())
}

/// Connect with the account the form describes (nothing is saved) and list its top-level folders.
#[tauri::command]
pub(crate) async fn remote_account_test(
    app: AppHandle,
    input: RemoteAccountInput,
) -> Result<RemoteTestView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let data_dir = crate::user_data_directory(&state)?;
        let account = account_from_input(&state.settings, &vault(&data_dir)?, &input)?;
        let fs = crate::remote_connections::connect_account(&data_dir, &account)
            .map_err(|error| error.to_string())?;
        let folders = folders_of(fs.as_ref(), "")?
            .into_iter()
            .map(|folder| folder.name)
            .collect();
        Ok(RemoteTestView { folders })
    })
    .await
    .map_err(|error| format!("測試連線失敗：{error}"))?
}

/// The shares an SMB server offers to the account in the form.
#[tauri::command]
pub(crate) async fn remote_smb_shares(
    app: AppHandle,
    input: RemoteAccountInput,
) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let data_dir = crate::user_data_directory(&state)?;
        let vault = vault(&data_dir)?;
        let account = account_from_input(&state.settings, &vault, &input)?;
        let password = vault
            .password_of(&account)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        player_remote::smb::list_shares(
            &account.host,
            account.port,
            &account.username,
            password.as_str(),
        )
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("無法取得共用資料夾：{error}"))?
}

/// Name every remote source after its server (the library's "source" column and groups show `smb1`, not the folder).
/// Sources of a server that no longer exists lose the name.
pub(crate) fn refresh_source_labels(
    database: &player_db::Database,
    settings: &crate::settings::AppSettings,
) -> Result<(), String> {
    for source in &settings.sources {
        let (SourceEntryKind::Folder {
            remote_account: Some(account_id),
            ..
        }
        | SourceEntryKind::PlaylistFile {
            remote_account: Some(account_id),
            ..
        }) = &source.kind
        else {
            continue;
        };
        let name = settings
            .remote_accounts
            .iter()
            .find(|account| &account.id == account_id)
            .map(|account| account.name.as_str());
        database
            .set_source_label(source.id, name)
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The disk cache of played remote files as the page sees it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteCacheView {
    /// The limit in MB (decimal), 100 to 5,000 in steps of 100.
    pub limit_mb: u32,
    pub used_bytes: u64,
}

fn cache_view(state: &AppState) -> Result<RemoteCacheView, String> {
    let settings = state.settings.snapshot().map_err(|error| error.to_string())?;
    Ok(RemoteCacheView {
        limit_mb: settings.remote_cache_limit_mb,
        used_bytes: state.remote_cache.as_ref().map_or(0, |cache| cache.used()),
    })
}

#[tauri::command]
pub(crate) fn remote_cache_get(state: tauri::State<'_, AppState>) -> Result<RemoteCacheView, String> {
    cache_view(&state)
}

/// Change the size limit of the cache: stored in the settings, applied at once (a smaller limit deletes the oldest
/// chunks).
#[tauri::command]
pub(crate) fn remote_cache_set_limit(
    state: tauri::State<'_, AppState>,
    limit_mb: u32,
) -> Result<RemoteCacheView, String> {
    let limit_mb = crate::settings::clamp_remote_cache_limit_mb(limit_mb);
    state
        .settings
        .update(|settings| {
            settings.remote_cache_limit_mb = limit_mb;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    if let Some(cache) = state.remote_cache.as_ref() {
        cache.set_limit(u64::from(limit_mb) * 1_000_000);
    }
    cache_view(&state)
}

#[tauri::command]
pub(crate) fn remote_cache_clear(state: tauri::State<'_, AppState>) -> Result<RemoteCacheView, String> {
    if let Some(cache) = state.remote_cache.as_ref() {
        cache.clear();
    }
    cache_view(&state)
}

/// An excluded remote folder as the page sees it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteExcludedView {
    pub id: String,
    pub account_id: String,
    pub account_name: String,
    pub path: String,
}

fn excluded_views(settings: &crate::settings::AppSettings) -> Vec<RemoteExcludedView> {
    settings
        .remote_excluded_folders
        .iter()
        .map(|folder| RemoteExcludedView {
            id: folder.id.clone(),
            account_id: folder.account_id.clone(),
            account_name: settings
                .remote_accounts
                .iter()
                .find(|account| account.id == folder.account_id)
                .map_or_else(String::new, |account| account.name.clone()),
            path: folder.path.clone(),
        })
        .collect()
}

#[tauri::command]
pub(crate) fn remote_excluded_list(state: tauri::State<'_, AppState>) -> Result<Vec<RemoteExcludedView>, String> {
    let settings = state.settings.snapshot().map_err(|error| error.to_string())?;
    Ok(excluded_views(&settings))
}

/// Exclude a folder of a saved account: library scans skip it and everything under it.
#[tauri::command]
pub(crate) fn remote_excluded_add(
    state: tauri::State<'_, AppState>,
    account_id: String,
    path: String,
) -> Result<Vec<RemoteExcludedView>, String> {
    let account = account_of_id(&state, &account_id)?;
    let path = normalize_folder(&path);
    if path.is_empty() {
        return Err("請選擇要排除的資料夾（不能是伺服器的最上層）。".to_owned());
    }
    let settings = state
        .settings
        .update(|settings| {
            let covered = settings.remote_excluded_folders.iter().any(|folder| {
                folder.account_id == account.id
                    && player_remote::index::is_excluded(&path, std::slice::from_ref(&folder.path))
            });
            if !covered {
                // A folder that contains earlier exclusions replaces them.
                settings.remote_excluded_folders.retain(|folder| {
                    folder.account_id != account.id
                        || !player_remote::index::is_excluded(&folder.path, std::slice::from_ref(&path))
                });
                settings.remote_excluded_folders.push(crate::settings::RemoteExcludedFolder {
                    id: player_remote::account::new_account_id(),
                    account_id: account.id.clone(),
                    path: path.clone(),
                });
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(excluded_views(&settings))
}

#[tauri::command]
pub(crate) fn remote_excluded_remove(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Vec<RemoteExcludedView>, String> {
    let settings = state
        .settings
        .update(|settings| {
            settings.remote_excluded_folders.retain(|folder| folder.id != id);
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(excluded_views(&settings))
}

/// Browse the server the form describes before anything is saved: at the top (`path` empty) the folders are the shares,
/// below that they are the folders of a share (`path` is `share/folder/...`). Used by the folder picker of the form.
#[tauri::command]
pub(crate) async fn remote_browse_draft(
    app: AppHandle,
    input: RemoteAccountInput,
    path: String,
) -> Result<Vec<RemoteFolderView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let data_dir = crate::user_data_directory(&state)?;
        let vault = vault(&data_dir)?;
        let path = normalize_folder(&path);
        let mut input = input;
        if path.is_empty() {
            // The shares of the server.
            input.path = "x".to_owned();
            let account = account_from_input(&state.settings, &vault, &input)?;
            let password = vault
                .password_of(&account)
                .map_err(|error| error.to_string())?
                .unwrap_or_default();
            let mut shares = player_remote::smb::list_shares(
                &account.host,
                account.port,
                &account.username,
                password.as_str(),
            )
            .map_err(|error| error.to_string())?;
            shares.sort_by(|left, right| natural_title_cmp(left, right));
            return Ok(shares
                .into_iter()
                .map(|name| RemoteFolderView {
                    path: name.clone(),
                    name,
                })
                .collect());
        }
        // Inside a share: the first piece is the share, the rest the folder in it.
        let (share, inner) = path
            .split_once('/')
            .map_or((path.as_str(), ""), |(share, rest)| (share, rest));
        input.path = share.to_owned();
        let account = account_from_input(&state.settings, &vault, &input)?;
        let fs = crate::remote_connections::connect_account(&data_dir, &account)
            .map_err(|error| error.to_string())?;
        let folders = folders_of(fs.as_ref(), inner)?;
        Ok(folders
            .into_iter()
            .map(|folder| RemoteFolderView {
                path: format!("{share}/{}", folder.path),
                name: folder.name,
            })
            .collect())
    })
    .await
    .map_err(|error| format!("無法讀取遠端資料夾：{error}"))?
}

/// The sub-folders of `path` (relative to the account's root) on a saved account's server.
#[tauri::command]
pub(crate) async fn remote_browse(
    app: AppHandle,
    account_id: String,
    path: String,
) -> Result<Vec<RemoteFolderView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let data_dir = crate::user_data_directory(&state)?;
        let account = account_of_id(&state, &account_id)?;
        let fs = state
            .remote_connections
            .connect(&data_dir, &account)
            .map_err(|error| {
                state.remote_connections.forget(&account.id);
                error.to_string()
            })?;
        folders_of(fs.as_ref(), &normalize_folder(&path))
    })
    .await
    .map_err(|error| format!("無法讀取遠端資料夾：{error}"))?
}

/// The folders and the playlist files (`.m3u`, `.m3u8`) of one folder of a server, for choosing a playlist.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemotePlaylistBrowseView {
    pub folders: Vec<RemoteFolderView>,
    pub files: Vec<RemoteFolderView>,
}

fn is_playlist_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".m3u") || lower.ends_with(".m3u8")
}

fn playlists_of(
    fs: &dyn player_remote::fs::RemoteFs,
    dir: &str,
) -> Result<RemotePlaylistBrowseView, String> {
    let entries = fs.list(dir).map_err(|error| error.to_string())?;
    let join = |name: &str| {
        if dir.is_empty() {
            name.to_owned()
        } else {
            format!("{dir}/{name}")
        }
    };
    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in entries {
        if entry.name.starts_with('.') {
            continue;
        }
        let view = RemoteFolderView {
            path: join(&entry.name),
            name: entry.name.clone(),
        };
        if entry.is_dir {
            folders.push(view);
        } else if is_playlist_name(&entry.name) {
            files.push(view);
        }
    }
    let by_name = |left: &RemoteFolderView, right: &RemoteFolderView| natural_title_cmp(&left.name, &right.name);
    folders.sort_by(by_name);
    files.sort_by(by_name);
    Ok(RemotePlaylistBrowseView { folders, files })
}

/// The sub-folders and playlist files of `path` (relative to the account's root) on a saved account's server.
#[tauri::command]
pub(crate) async fn remote_browse_playlists(
    app: AppHandle,
    account_id: String,
    path: String,
) -> Result<RemotePlaylistBrowseView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let data_dir = crate::user_data_directory(&state)?;
        let account = account_of_id(&state, &account_id)?;
        let fs = state
            .remote_connections
            .connect(&data_dir, &account)
            .map_err(|error| {
                state.remote_connections.forget(&account.id);
                error.to_string()
            })?;
        playlists_of(fs.as_ref(), &normalize_folder(&path))
    })
    .await
    .map_err(|error| format!("無法讀取遠端資料夾：{error}"))?
}

/// Add a playlist file (`.m3u`, `.m3u8`) of a saved account's server as a source. The songs it lists are read from the
/// same server; the next library sync registers them and the playlist.
#[tauri::command]
pub(crate) async fn remote_playlist_add(
    app: AppHandle,
    account_id: String,
    path: String,
) -> Result<LibrarySource, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let account = account_of_id(&state, &account_id)?;
        let file = normalize_folder(&path);
        if !is_playlist_name(file.rsplit('/').next().unwrap_or("")) {
            return Err("請選擇 .m3u 或 .m3u8 播放清單檔案。".to_owned());
        }
        let settings = state.settings.snapshot().map_err(|error| error.to_string())?;
        let duplicate = settings.sources.iter().any(|source| {
            matches!(
                &source.kind,
                SourceEntryKind::PlaylistFile { remote_account: Some(existing), path: stored, .. }
                    if *existing == account.id
                        && matches!(
                            stored.to_media_locator(),
                            Ok(MediaLocator::Remote { path: existing_path, .. }) if existing_path == file
                        )
            )
        });
        if duplicate {
            return Err("這個播放清單已經加入。".to_owned());
        }
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let _database_work = crate::enter_database_work(&state)?;
        let id = SourceId::new();
        let display_name = file
            .rsplit('/')
            .next()
            .and_then(|name| name.rsplit_once('.').map(|(stem, _)| stem))
            .filter(|stem| !stem.is_empty())
            .unwrap_or("播放清單")
            .to_owned();
        let locator = MediaLocator::Remote {
            source_id: id,
            path: file,
        };
        let entry = SourceEntry {
            id,
            display_name: display_name.clone(),
            enabled: true,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id: player_core::PlaylistId::new(),
                path: StoredPath::from_locator(&locator).map_err(|error| error.to_string())?,
                tree_uri: None,
                remote_account: Some(account.id.clone()),
            },
        };
        state
            .settings
            .upsert_source(entry.clone())
            .map_err(|error| error.to_string())?;
        database
            .save_library_root(&LibraryRoot {
                id,
                kind: MediaSourceKind::PlaylistFile,
                display_name,
                locator,
                enabled: true,
            })
            .map_err(|error| error.to_string())?;
        database
            .set_source_label(id, Some(&account.name))
            .map_err(|error| error.to_string())?;
        crate::source_entry_summary_with(Some(database), &entry, std::slice::from_ref(&account))
    })
    .await
    .map_err(|error| format!("無法加入遠端播放清單：{error}"))?
}

/// Add a folder of a saved account as a library source.
#[tauri::command]
pub(crate) async fn remote_source_add(
    app: AppHandle,
    account_id: String,
    path: String,
    display_name: Option<String>,
) -> Result<LibrarySource, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let account = account_of_id(&state, &account_id)?;
        let base = normalize_folder(&path);
        let settings = state.settings.snapshot().map_err(|error| error.to_string())?;
        let duplicate = settings.sources.iter().any(|source| {
            matches!(
                &source.kind,
                SourceEntryKind::Folder { remote_account: Some(existing), path: stored, .. }
                    if *existing == account.id
                        && matches!(
                            stored.to_media_locator(),
                            Ok(MediaLocator::Remote { path: existing_path, .. }) if existing_path == base
                        )
            )
        });
        if duplicate {
            return Err("這個遠端資料夾已經加入。".to_owned());
        }
        let database = state.database.as_ref().ok_or_else(|| {
            state
                .database_error
                .clone()
                .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
        })?;
        let _database_work = crate::enter_database_work(&state)?;
        let id = SourceId::new();
        let display_name = display_name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                let folder = base.rsplit('/').next().unwrap_or("");
                if folder.is_empty() {
                    account.name.clone()
                } else {
                    format!("{} / {folder}", account.name)
                }
            });
        let base_for_summary = base.clone();
        let root = LibraryRoot {
            id,
            kind: MediaSourceKind::Remote,
            display_name: display_name.clone(),
            locator: MediaLocator::Remote {
                source_id: id,
                path: base,
            },
            enabled: true,
        };
        let entry = SourceEntry {
            id,
            display_name,
            enabled: true,
            kind: SourceEntryKind::Folder {
                media_kind: MediaSourceKind::Remote,
                path: StoredPath::from_locator(&root.locator).map_err(|error| error.to_string())?,
                remote_account: Some(account.id.clone()),
            },
        };
        state
            .settings
            .upsert_source(entry)
            .map_err(|error| error.to_string())?;
        database
            .save_library_root(&root)
            .map_err(|error| error.to_string())?;
        database
            .set_source_label(id, Some(&account.name))
            .map_err(|error| error.to_string())?;
        let mut summary = crate::source_summary(database, &root)?;
        summary.remote_account_id = Some(account.id.clone());
        summary.location = if base_for_summary.is_empty() {
            account.name.clone()
        } else {
            format!("{} / {base_for_summary}", account.name)
        };
        Ok(summary)
    })
    .await
    .map_err(|error| format!("無法加入遠端來源：{error}"))?
}

#[cfg(test)]
mod tests {
    use super::normalize_folder;

    #[test]
    fn folder_paths_are_stored_without_slashes_at_the_ends() {
        assert_eq!(normalize_folder("/Music//Hi-Res/"), "Music/Hi-Res");
        assert_eq!(normalize_folder("Music\\Loss"), "Music/Loss");
        assert_eq!(normalize_folder("/"), "");
        assert_eq!(normalize_folder("./a/./b"), "a/b");
    }
}
