//! Remote accounts (ADR 0026): what `settings.json` stores for a server, with the password only in sealed form, and the
//! passphrase-sealed export that carries accounts to another device.
//!
//! An account is the connection (protocol, host, user, password); the folders chosen under it are sources that point at
//! the account by id. The stored `password` is a device-sealed secret (see [`crate::secret`]) bound to the account id,
//! so it is never plaintext in the file and cannot be pasted onto another account. An export seals the whole account
//! list, passwords included, under a passphrase the user types, so it opens on any device.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::secret::{self, DeviceKey, KdfCost, SecretError};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteProtocol {
    #[default]
    Webdav,
    Smb,
}

/// How a TLS (https) connection is trusted. SMB ignores it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum TlsMode {
    /// The certificate must be valid for the host (the default).
    #[default]
    Verify,
    /// A self-signed certificate the user accepted once: only this SHA-256 (hex) of its DER is trusted.
    Pinned { sha256: String },
    /// Plain `http://` (shown with an "unencrypted" warning).
    Off,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccount {
    /// Stable identity (random hex); sources and sealed passwords refer to it.
    pub id: String,
    pub name: String,
    pub protocol: RemoteProtocol,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The SMB share name, or the base path of a WebDAV server (`/remote.php/dav/files/me`).
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub username: String,
    /// The device-sealed password (`v1.…`); absent for an anonymous account or after a restore without a passphrase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default)]
    pub tls: TlsMode,
    /// Android: do not use this account on mobile data.
    #[serde(default)]
    pub only_on_wifi: bool,
}

/// A fresh account id: 128 random bits as hex.
pub fn new_account_id() -> String {
    secret::random_hex(16)
}

fn password_scope(account_id: &str) -> String {
    format!("account:{account_id}:password")
}

/// The machine's key plus the account operations that need it.
pub struct CredentialVault {
    key: DeviceKey,
}

impl CredentialVault {
    pub fn new(key: DeviceKey) -> Self {
        Self { key }
    }

    pub fn open(key_path: &std::path::Path) -> Result<Self, SecretError> {
        DeviceKey::load_or_create(key_path).map(Self::new)
    }

    /// Seal a password for `account_id`; an empty password means "none".
    pub fn seal_password(
        &self,
        account_id: &str,
        plaintext: &str,
    ) -> Result<Option<String>, SecretError> {
        if plaintext.is_empty() {
            return Ok(None);
        }
        secret::seal(&self.key, &password_scope(account_id), plaintext).map(Some)
    }

    /// The plaintext password of an account (`None` when it has none). Keep it only as long as the connection needs.
    pub fn password_of(
        &self,
        account: &RemoteAccount,
    ) -> Result<Option<Zeroizing<String>>, SecretError> {
        match &account.password {
            None => Ok(None),
            Some(sealed) => secret::open(&self.key, &password_scope(&account.id), sealed).map(Some),
        }
    }

    /// The account list as an export file's text, sealed under `passphrase` (passwords included).
    pub fn export(
        &self,
        accounts: &[RemoteAccount],
        passphrase: &str,
    ) -> Result<String, SecretError> {
        self.export_with_cost(accounts, passphrase, KdfCost::default())
    }

    fn export_with_cost(
        &self,
        accounts: &[RemoteAccount],
        passphrase: &str,
        cost: KdfCost,
    ) -> Result<String, SecretError> {
        let mut plain = Vec::with_capacity(accounts.len());
        for account in accounts {
            let password = self.password_of(account)?;
            plain.push(ExportedAccount {
                account: RemoteAccount {
                    password: None,
                    ..account.clone()
                },
                password: password.map(|password| (*password).clone()),
            });
        }
        let payload = Zeroizing::new(
            serde_json::to_vec(&ExportPayload { accounts: plain })
                .map_err(|e| SecretError::Format(e.to_string()))?,
        );
        let bundle = secret::seal_with_passphrase(passphrase, EXPORT_PURPOSE, &payload, cost)?;
        serde_json::to_string_pretty(&ExportFile {
            format: EXPORT_FORMAT.to_owned(),
            version: 1,
            bundle,
        })
        .map_err(|e| SecretError::Format(e.to_string()))
    }

    /// Open an export file and return its accounts with their passwords sealed under THIS device's key (so they can be
    /// stored in `settings.json`). The accounts keep their ids.
    pub fn import(&self, text: &str, passphrase: &str) -> Result<Vec<RemoteAccount>, SecretError> {
        let file: ExportFile = serde_json::from_str(text)
            .map_err(|_| SecretError::Format("不是遠端帳號匯出檔".to_owned()))?;
        if file.format != EXPORT_FORMAT {
            return Err(SecretError::Format("不是遠端帳號匯出檔".to_owned()));
        }
        if file.version != 1 {
            return Err(SecretError::Format(format!(
                "不支援的匯出檔版本 {}",
                file.version
            )));
        }
        let plain = secret::open_with_passphrase(passphrase, EXPORT_PURPOSE, &file.bundle)?;
        let payload: ExportPayload = serde_json::from_slice(&plain)
            .map_err(|_| SecretError::Format("匯出檔內容無法讀取".to_owned()))?;
        payload
            .accounts
            .into_iter()
            .map(|exported| {
                let password = match exported.password {
                    Some(password) => self.seal_password(&exported.account.id, &password)?,
                    None => None,
                };
                Ok(RemoteAccount {
                    password,
                    ..exported.account
                })
            })
            .collect()
    }
}

const EXPORT_FORMAT: &str = "moe-remote-accounts";
const EXPORT_PURPOSE: &str = "account-export";

#[derive(Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    /// Passphrase-sealed [`ExportPayload`].
    bundle: String,
}

#[derive(Serialize, Deserialize)]
struct ExportPayload {
    accounts: Vec<ExportedAccount>,
}

/// Inside the sealed bundle only: an account with its password in the clear.
#[derive(Serialize, Deserialize)]
struct ExportedAccount {
    #[serde(flatten)]
    account: RemoteAccount,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: &str, vault: &CredentialVault, password: &str) -> RemoteAccount {
        RemoteAccount {
            id: id.to_owned(),
            name: "家裡的 NAS".to_owned(),
            protocol: RemoteProtocol::Webdav,
            host: "nas.local".to_owned(),
            port: Some(5006),
            path: "/music".to_owned(),
            username: "xpramt".to_owned(),
            password: vault.seal_password(id, password).unwrap(),
            tls: TlsMode::Pinned {
                sha256: "ab".repeat(32),
            },
            only_on_wifi: true,
        }
    }

    fn fast_export(
        vault: &CredentialVault,
        accounts: &[RemoteAccount],
        passphrase: &str,
    ) -> String {
        vault
            .export_with_cost(
                accounts,
                passphrase,
                KdfCost {
                    memory_kib: 64,
                    iterations: 1,
                    parallelism: 1,
                },
            )
            .unwrap()
    }

    #[test]
    fn the_stored_account_never_contains_the_plaintext_password() {
        let vault = CredentialVault::new(DeviceKey::generate());
        let stored = serde_json::to_string(&account("a1", &vault, "s3cret-密碼")).unwrap();
        assert!(
            !stored.contains("s3cret") && !stored.contains("密碼"),
            "{stored}"
        );
        assert!(stored.contains("\"password\":\"v1."));
        let anonymous = serde_json::to_string(&account("a2", &vault, "")).unwrap();
        assert!(!anonymous.contains("password"), "no password, no field");
    }

    #[test]
    fn a_password_opens_for_its_account_and_not_on_another_one() {
        let vault = CredentialVault::new(DeviceKey::generate());
        let first = account("a1", &vault, "hunter2");
        assert_eq!(&*vault.password_of(&first).unwrap().unwrap(), "hunter2");
        let moved = RemoteAccount {
            id: "a2".to_owned(),
            ..first
        };
        assert_eq!(
            vault.password_of(&moved),
            Err(SecretError::Decrypt),
            "copied onto another account"
        );
        let anonymous = account("a3", &vault, "");
        assert!(vault.password_of(&anonymous).unwrap().is_none());
    }

    #[test]
    fn accounts_move_to_another_device_through_a_passphrase_sealed_export() {
        let here = CredentialVault::new(DeviceKey::generate());
        let there = CredentialVault::new(DeviceKey::generate());
        let accounts = vec![
            account("a1", &here, "pw-one"),
            account("a2", &here, ""),
            account("a3", &here, "pw 三"),
        ];
        let exported = fast_export(&here, &accounts, "my export passphrase");
        // The file says what it is, and shows nothing of the accounts.
        assert!(exported.contains("moe-remote-accounts"));
        for secret in ["nas.local", "xpramt", "pw-one", "家裡的 NAS", "5006"] {
            assert!(
                !exported.contains(secret),
                "{secret} leaked into the export file"
            );
        }
        let imported = there.import(&exported, "my export passphrase").unwrap();
        assert_eq!(imported.len(), 3);
        assert_eq!(imported[0].host, "nas.local");
        assert_eq!(
            imported[0].tls,
            TlsMode::Pinned {
                sha256: "ab".repeat(32)
            }
        );
        assert_eq!(
            &*there.password_of(&imported[0]).unwrap().unwrap(),
            "pw-one"
        );
        assert!(imported[1].password.is_none());
        assert_eq!(&*there.password_of(&imported[2]).unwrap().unwrap(), "pw 三");
        // Sealed under the new device's key, so the old device's key cannot open them.
        assert_eq!(here.password_of(&imported[0]), Err(SecretError::Decrypt));
        // The stored form on the new device is again free of plaintext.
        assert!(!serde_json::to_string(&imported).unwrap().contains("pw-one"));
    }

    #[test]
    fn a_wrong_passphrase_or_a_foreign_file_is_refused() {
        let vault = CredentialVault::new(DeviceKey::generate());
        let exported = fast_export(&vault, &[account("a1", &vault, "x")], "right");
        assert_eq!(vault.import(&exported, "wrong"), Err(SecretError::Decrypt));
        assert!(matches!(
            vault.import("{\"hello\":1}", "right"),
            Err(SecretError::Format(_))
        ));
        assert!(matches!(
            vault.import("not json", "right"),
            Err(SecretError::Format(_))
        ));
        let wrong_format = exported.replace("moe-remote-accounts", "something-else");
        assert!(matches!(
            vault.import(&wrong_format, "right"),
            Err(SecretError::Format(_))
        ));
    }

    #[test]
    fn an_account_json_round_trips_and_defaults_missing_fields() {
        let json = r#"{"id":"x","name":"n","protocol":"smb","host":"h"}"#;
        let account: RemoteAccount = serde_json::from_str(json).unwrap();
        assert_eq!(account.protocol, RemoteProtocol::Smb);
        assert_eq!(account.tls, TlsMode::Verify);
        assert!(account.password.is_none() && !account.only_on_wifi && account.port.is_none());
        assert_eq!(
            serde_json::from_str::<RemoteAccount>(&serde_json::to_string(&account).unwrap())
                .unwrap(),
            account
        );
    }
}
