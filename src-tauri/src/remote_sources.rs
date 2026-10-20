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
        let mut summary = crate::source_summary(database, &root)?;
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
