//! Remote accounts (ADR 0026): the commands that create, change, delete, export and import them.
//!
//! Passwords are managed by the app itself, not by an operating-system credential store: a typed password is sealed with
//! the device key (`player_remote::secret`) before it is written to `settings.json`, and an export seals the accounts
//! under a passphrase so they can be restored on another device.

use std::path::Path;

use player_remote::account::{new_account_id, CredentialVault, RemoteAccount, RemoteProtocol, TlsMode};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::settings::SettingsStore;

pub(crate) const KEY_FILE_NAME: &str = "remote-credentials.key";
/// The extension of an account export file.
pub(crate) const EXPORT_EXTENSION: &str = "moeremote";

/// What the page sees of an account: everything but the password (only whether one is stored).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAccountView {
    pub id: String,
    pub name: String,
    pub protocol: RemoteProtocol,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub username: String,
    pub has_password: bool,
    pub tls: TlsMode,
    pub only_on_wifi: bool,
}

impl From<&RemoteAccount> for RemoteAccountView {
    fn from(account: &RemoteAccount) -> Self {
        Self {
            id: account.id.clone(),
            name: account.name.clone(),
            protocol: account.protocol,
            host: account.host.clone(),
            port: account.port,
            path: account.path.clone(),
            username: account.username.clone(),
            has_password: account.password.is_some(),
            tls: account.tls.clone(),
            only_on_wifi: account.only_on_wifi,
        }
    }
}

/// What the page sends when it saves an account.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAccountInput {
    /// `None` creates a new account.
    pub id: Option<String>,
    pub name: String,
    pub protocol: RemoteProtocol,
    pub host: String,
    pub port: Option<u16>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub username: String,
    /// `None` keeps the stored password, `Some("")` removes it, anything else replaces it.
    pub password: Option<String>,
    #[serde(default)]
    pub tls: TlsMode,
    #[serde(default)]
    pub only_on_wifi: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoteAccountsImportResult {
    pub added: usize,
    pub updated: usize,
}

fn vault(data_dir: &Path) -> Result<CredentialVault, String> {
    CredentialVault::open(&data_dir.join(KEY_FILE_NAME)).map_err(|error| error.to_string())
}

/// A host name or address: no scheme, path, credentials or spaces.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.contains(|c: char| c.is_whitespace() || matches!(c, '/' | '\\' | '@' | '?' | '#'))
        && !host.contains("://")
}

fn normalize_input(input: &RemoteAccountInput) -> Result<(String, String, String, String), String> {
    let name = input.name.trim().to_owned();
    if name.is_empty() || name.chars().count() > 80 {
        return Err("請輸入名稱（80 字以內）。".to_owned());
    }
    let host = input.host.trim().trim_end_matches('/').to_owned();
    // A pasted URL is the usual mistake: say what to do instead of storing a broken host.
    if !valid_host(&host) {
        return Err("主機請只輸入名稱或 IP 位址（不含 http://、路徑或空格）。".to_owned());
    }
    if input.port == Some(0) {
        return Err("連接埠必須在 1 到 65535 之間。".to_owned());
    }
    let username = input.username.trim().to_owned();
    if username.chars().count() > 256 {
        return Err("帳號太長。".to_owned());
    }
    let path = input.path.trim();
    let path = match input.protocol {
        RemoteProtocol::Smb => {
            let share = path.trim_matches(|c| c == '/' || c == '\\');
            if share.is_empty() || share.contains(['/', '\\']) {
                return Err("SMB 請輸入共用資料夾的名稱（不含斜線）。".to_owned());
            }
            share.to_owned()
        }
        RemoteProtocol::Webdav => {
            let trimmed = path.trim_matches('/');
            if trimmed.is_empty() {
                String::new()
            } else {
                format!("/{trimmed}")
            }
        }
    };
    Ok((name, host, username, path))
}

/// Create or change an account in `settings`, sealing a new password with `vault`.
pub(crate) fn save_account(
    settings: &SettingsStore,
    vault: &CredentialVault,
    input: RemoteAccountInput,
) -> Result<RemoteAccountView, String> {
    let (name, host, username, path) = normalize_input(&input)?;
    let current = settings.snapshot().map_err(|error| error.to_string())?.remote_accounts;
    let existing = match input.id.as_deref() {
        Some(id) => Some(
            current
                .iter()
                .find(|account| account.id == id)
                .ok_or_else(|| "找不到這個遠端帳號。".to_owned())?,
        ),
        None => None,
    };
    let id = existing.map_or_else(new_account_id, |account| account.id.clone());
    let password = match (&input.password, existing) {
        (None, Some(account)) => account.password.clone(),
        (None, None) => None,
        (Some(plaintext), _) => vault.seal_password(&id, plaintext).map_err(|error| error.to_string())?,
    };
    let account = RemoteAccount {
        id,
        name,
        protocol: input.protocol,
        host,
        port: input.port,
        path,
        username,
        password,
        tls: input.tls.clone(),
        only_on_wifi: input.only_on_wifi,
    };
    let view = RemoteAccountView::from(&account);
    settings
        .update(|settings| {
            match settings.remote_accounts.iter().position(|existing| existing.id == account.id) {
                Some(index) => settings.remote_accounts[index] = account.clone(),
                None => settings.remote_accounts.push(account.clone()),
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(view)
}

pub(crate) fn delete_account(settings: &SettingsStore, id: &str) -> Result<bool, String> {
    let mut removed = false;
    settings
        .update(|settings| {
            let before = settings.remote_accounts.len();
            settings.remote_accounts.retain(|account| account.id != id);
            removed = settings.remote_accounts.len() != before;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(removed)
}

/// The text of an export file with every account (passwords included), sealed under `passphrase`.
pub(crate) fn export_text(
    settings: &SettingsStore,
    vault: &CredentialVault,
    passphrase: &str,
) -> Result<String, String> {
    let accounts = settings.snapshot().map_err(|error| error.to_string())?.remote_accounts;
    if accounts.is_empty() {
        return Err("沒有可匯出的遠端帳號。".to_owned());
    }
    vault.export(&accounts, passphrase).map_err(|error| error.to_string())
}

/// Add the accounts of an export file (same id: replaced). Passwords are re-sealed with this device's key.
pub(crate) fn import_text(
    settings: &SettingsStore,
    vault: &CredentialVault,
    text: &str,
    passphrase: &str,
) -> Result<RemoteAccountsImportResult, String> {
    let imported = vault.import(text, passphrase).map_err(|error| error.to_string())?;
    let (mut added, mut updated) = (0, 0);
    settings
        .update(|settings| {
            for account in &imported {
                match settings.remote_accounts.iter().position(|existing| existing.id == account.id) {
                    Some(index) => {
                        settings.remote_accounts[index] = account.clone();
                        updated += 1;
                    }
                    None => {
                        settings.remote_accounts.push(account.clone());
                        added += 1;
                    }
                }
            }
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok(RemoteAccountsImportResult { added, updated })
}

#[tauri::command]
pub(crate) fn remote_accounts_list(state: tauri::State<'_, crate::AppState>) -> Result<Vec<RemoteAccountView>, String> {
    Ok(state
        .settings
        .snapshot()
        .map_err(|error| error.to_string())?
        .remote_accounts
        .iter()
        .map(RemoteAccountView::from)
        .collect())
}

#[tauri::command]
pub(crate) fn remote_account_save(
    state: tauri::State<'_, crate::AppState>,
    input: RemoteAccountInput,
) -> Result<RemoteAccountView, String> {
    let data_dir = crate::user_data_directory(&state)?;
    save_account(&state.settings, &vault(&data_dir)?, input)
}

#[tauri::command]
pub(crate) fn remote_account_delete(state: tauri::State<'_, crate::AppState>, id: String) -> Result<bool, String> {
    delete_account(&state.settings, &id)
}

/// Export every remote account into a file chosen with the system dialog, sealed under `passphrase`.
/// Returns the file name, or `None` when the dialog was cancelled.
#[tauri::command]
pub(crate) async fn remote_accounts_export(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
    passphrase: String,
) -> Result<Option<String>, String> {
    if passphrase.chars().count() < 8 {
        return Err("匯出密碼至少需要 8 個字元。".to_owned());
    }
    let data_dir = crate::user_data_directory(&state)?;
    let text = {
        let settings = state.settings.clone();
        tauri::async_runtime::spawn_blocking(move || export_text(&settings, &vault(&data_dir)?, &passphrase))
            .await
            .map_err(|error| error.to_string())??
    };
    let file_name = format!("MoeMusicPlayer-remote-accounts.{EXPORT_EXTENSION}");
    crate::backup::save_text_file(&app, &window, &text, &file_name, "匯出遠端帳號", EXPORT_EXTENSION)
        .await
        .map(|saved| saved.then_some(file_name))
}

/// Import accounts from an export file chosen with the system dialog, opened with `passphrase`.
#[tauri::command]
pub(crate) async fn remote_accounts_import(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
    passphrase: String,
) -> Result<Option<RemoteAccountsImportResult>, String> {
    let data_dir = crate::user_data_directory(&state)?;
    let Some(text) = crate::backup::read_text_file(&app, &window, "匯入遠端帳號", EXPORT_EXTENSION).await? else {
        return Ok(None);
    };
    let settings = state.settings.clone();
    tauri::async_runtime::spawn_blocking(move || import_text(&settings, &vault(&data_dir)?, &text, &passphrase))
        .await
        .map_err(|error| error.to_string())?
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppSettings;
    use std::path::PathBuf;
    use player_remote::secret::DeviceKey;

    fn store(name: &str) -> (SettingsStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("moe-remote-accounts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (SettingsStore::open(dir.join("settings.json"), AppSettings::default()).unwrap(), dir)
    }

    fn input(name: &str, password: Option<&str>) -> RemoteAccountInput {
        RemoteAccountInput {
            id: None,
            name: name.to_owned(),
            protocol: RemoteProtocol::Webdav,
            host: "nas.local".to_owned(),
            port: Some(5006),
            path: "dav/music/".to_owned(),
            username: "me".to_owned(),
            password: password.map(str::to_owned),
            tls: TlsMode::Verify,
            only_on_wifi: false,
        }
    }

    #[test]
    fn a_saved_password_is_in_settings_json_only_as_sealed_text() {
        let (settings, dir) = store("seal");
        let vault = CredentialVault::new(DeviceKey::generate());
        let view = save_account(&settings, &vault, input("NAS", Some("hunter2-密碼"))).unwrap();
        assert!(view.has_password);
        assert_eq!(view.path, "/dav/music", "a WebDAV path is normalised to a leading slash and no trailing one");
        let text = std::fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(!text.contains("hunter2") && !text.contains("密碼"), "plaintext reached settings.json");
        assert!(text.contains("\"password\": \"v1."), "{text}");
        let stored = settings.snapshot().unwrap().remote_accounts;
        assert_eq!(&*vault.password_of(&stored[0]).unwrap().unwrap(), "hunter2-密碼");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn editing_keeps_or_replaces_or_clears_the_password() {
        let (settings, dir) = store("edit");
        let vault = CredentialVault::new(DeviceKey::generate());
        let id = save_account(&settings, &vault, input("NAS", Some("one"))).unwrap().id;
        let edit = |password: Option<&str>| {
            let mut changed = input("NAS 改名", password);
            changed.id = Some(id.clone());
            save_account(&settings, &vault, changed).unwrap()
        };
        assert!(edit(None).has_password, "None keeps the stored password");
        let stored = settings.snapshot().unwrap().remote_accounts;
        assert_eq!(&*vault.password_of(&stored[0]).unwrap().unwrap(), "one");
        edit(Some("two"));
        let stored = settings.snapshot().unwrap().remote_accounts;
        assert_eq!(&*vault.password_of(&stored[0]).unwrap().unwrap(), "two");
        assert!(!edit(Some("")).has_password, "an empty password removes it");
        assert_eq!(settings.snapshot().unwrap().remote_accounts.len(), 1);
        assert!(save_account(&settings, &vault, RemoteAccountInput { id: Some("nope".into()), ..input("x", None) }).is_err());
        assert!(delete_account(&settings, &id).unwrap());
        assert!(!delete_account(&settings, &id).unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_input_is_refused_with_a_hint() {
        let (settings, dir) = store("invalid");
        let vault = CredentialVault::new(DeviceKey::generate());
        for host in ["", "http://nas", "nas/music", "user@nas", "my nas"] {
            let mut bad = input("NAS", None);
            bad.host = host.to_owned();
            assert!(save_account(&settings, &vault, bad).is_err(), "{host:?}");
        }
        let mut smb = input("SMB", None);
        smb.protocol = RemoteProtocol::Smb;
        smb.path = "share/sub".to_owned();
        assert!(save_account(&settings, &vault, smb.clone()).is_err(), "an SMB share name has no slash");
        smb.path = "\\music\\".to_owned();
        assert_eq!(save_account(&settings, &vault, smb).unwrap().path, "music");
        let mut port = input("p", None);
        port.port = Some(0);
        assert!(save_account(&settings, &vault, port).is_err());
        assert!(save_account(&settings, &vault, input("   ", None)).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accounts_move_between_two_devices_through_an_export_file() {
        let (here, dir_a) = store("export-a");
        let (there, dir_b) = store("export-b");
        let vault_a = CredentialVault::new(DeviceKey::generate());
        let vault_b = CredentialVault::new(DeviceKey::generate());
        save_account(&here, &vault_a, input("NAS 一", Some("pw-1"))).unwrap();
        save_account(&here, &vault_a, input("NAS 二", Some("pw-2"))).unwrap();
        assert!(export_text(&there, &vault_b, "passphrase 1").is_err(), "nothing to export");
        let text = export_text(&here, &vault_a, "passphrase 1").unwrap();
        assert_eq!(
            import_text(&there, &vault_b, &text, "wrong").unwrap_err(),
            "無法解密：密碼錯誤，或資料已被更動"
        );
        let result = import_text(&there, &vault_b, &text, "passphrase 1").unwrap();
        assert_eq!(result, RemoteAccountsImportResult { added: 2, updated: 0 });
        let accounts = there.snapshot().unwrap().remote_accounts;
        assert_eq!(accounts.len(), 2);
        assert_eq!(&*vault_b.password_of(&accounts[1]).unwrap().unwrap(), "pw-2");
        assert!(!std::fs::read_to_string(dir_b.join("settings.json")).unwrap().contains("pw-"));
        // Importing again updates instead of duplicating.
        assert_eq!(
            import_text(&there, &vault_b, &text, "passphrase 1").unwrap(),
            RemoteAccountsImportResult { added: 0, updated: 2 }
        );
        assert_eq!(there.snapshot().unwrap().remote_accounts.len(), 2);
        let _ = std::fs::remove_dir_all(dir_a);
        let _ = std::fs::remove_dir_all(dir_b);
    }

    #[test]
    fn a_hand_edited_plaintext_password_is_dropped_on_load() {
        let (settings, dir) = store("plain");
        let path = dir.join("settings.json");
        let mut json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json["remoteAccounts"] = serde_json::json!([
            {"id": "a", "name": "n", "protocol": "webdav", "host": "h", "password": "plaintext!"},
            {"id": "a", "name": "dup", "protocol": "smb", "host": "h"},
            {"broken": true}
        ]);
        std::fs::write(&path, serde_json::to_vec_pretty(&json).unwrap()).unwrap();
        drop(settings);
        let reopened = SettingsStore::open(path, AppSettings::default()).unwrap();
        let accounts = reopened.snapshot().unwrap().remote_accounts;
        assert_eq!(accounts.len(), 1, "the duplicate id and the broken entry are skipped");
        assert!(accounts[0].password.is_none(), "plaintext is never kept");
        let _ = std::fs::remove_dir_all(dir);
    }
}
