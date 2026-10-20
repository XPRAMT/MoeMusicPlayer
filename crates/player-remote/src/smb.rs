//! The SMB client (ADR 0026): SMB 2/3 through the pure-Rust `smb` crate (signing and encryption when the server offers
//! them), behind the same [`RemoteFs`] trait as WebDAV. SMB 1 is not supported.
//!
//! One [`SmbFs`] is one account on one share. The connection is made when the value is created and made again, once,
//! when an operation finds it broken (a server that was restarted, a link that dropped). The password lives only inside
//! this value (wiped when dropped) and is never put in an error.

use std::sync::{Mutex, PoisonError};

use smb::{
    Client, ClientConfig, ConnectionConfig, DirAccessMask, FileAccessMask, FileBasicInformation,
    FileCreateArgs, FileFullDirectoryInformation, FileStandardInformation, ReadAt, Resource,
    UncPath,
};
use zeroize::Zeroizing;

use crate::fs::{RangeReader, RemoteEntry, RemoteError, RemoteFile, RemoteFs};

/// Blocks (256 KiB each) a sequential reader fetches per request. Every request costs a round trip, which dominates on
/// a server far away: measured against a server ~50 ms away, 4 blocks gave 16 MiB/s, 16 blocks 43 MiB/s, 32 blocks
/// 57 MiB/s.
pub const SMB_READ_AHEAD_BLOCKS: u64 = 32;

/// Where an SMB share is and who may read it.
#[derive(Clone)]
pub struct SmbSettings {
    pub host: String,
    /// `None` is the standard port, 445.
    pub port: Option<u16>,
    pub share: String,
    /// The folder inside the share that is the account's root (`""` is the whole share), `/`-separated.
    pub base_path: String,
    pub user: String,
    pub password: Zeroizing<String>,
}

impl std::fmt::Debug for SmbSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmbSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("share", &self.share)
            .field("base_path", &self.base_path)
            .field("user", &self.user)
            .field("password", &"<hidden>")
            .finish()
    }
}

/// NTSTATUS values this client reacts to.
mod status {
    pub const ACCESS_DENIED: u32 = 0xC000_0022;
    pub const LOGON_FAILURE: u32 = 0xC000_006D;
    pub const ACCOUNT_DISABLED: u32 = 0xC000_0072;
    pub const PASSWORD_EXPIRED: u32 = 0xC000_0071;
    pub const NO_SUCH_FILE: u32 = 0xC000_000F;
    pub const OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
    pub const OBJECT_PATH_NOT_FOUND: u32 = 0xC000_003A;
    pub const BAD_NETWORK_NAME: u32 = 0xC000_00CC;
}

/// What an `smb` error means to the user. `Network` is the kind a reconnect may fix.
fn classify(error: &smb::Error) -> RemoteError {
    use smb::Error as E;
    match error {
        E::UnexpectedMessageStatus(code) | E::ReceivedErrorMessage(code, _) => match *code {
            status::LOGON_FAILURE
            | status::ACCOUNT_DISABLED
            | status::PASSWORD_EXPIRED
            | status::ACCESS_DENIED => RemoteError::Auth,
            status::NO_SUCH_FILE
            | status::OBJECT_NAME_NOT_FOUND
            | status::OBJECT_PATH_NOT_FOUND
            | status::BAD_NETWORK_NAME => RemoteError::NotFound,
            _ => RemoteError::Protocol(error.to_string()),
        },
        E::MissingPermissions(_) => RemoteError::Auth,
        E::NotFound(_) => RemoteError::NotFound,
        E::SspiError(_) | E::UnsupportedAuthenticationMechanism(_) => RemoteError::Auth,
        E::IoError(_)
        | E::ConnectionStopped
        | E::OperationTimeout(..)
        | E::ChannelRecvError(_)
        | E::UnexpectedMessageId(..) => RemoteError::Network(error.to_string()),
        _ => RemoteError::Protocol(error.to_string()),
    }
}

/// `a/b/c` → `a\b\c`, rejecting `..` so a path can never leave the account's root.
fn windows_path(parts: &[&str]) -> Result<String, RemoteError> {
    let mut out: Vec<&str> = Vec::new();
    for part in parts {
        for piece in part.split(['/', '\\']) {
            match piece {
                "" | "." => {}
                ".." => return Err(RemoteError::NotFound),
                other => out.push(other),
            }
        }
    }
    Ok(out.join("\\"))
}

/// The FILETIME (100 ns steps since 1601) as UTC Unix milliseconds; 0 means "no time".
fn unix_ms(filetime: u64) -> Option<i64> {
    const EPOCH_DIFFERENCE_MS: i64 = 11_644_473_600_000;
    if filetime == 0 {
        return None;
    }
    Some((filetime / 10_000) as i64 - EPOCH_DIFFERENCE_MS)
}

pub struct SmbFs {
    settings: SmbSettings,
    client: Mutex<std::sync::Arc<Client>>,
}

impl SmbFs {
    /// Connect and sign in. Fails with [`RemoteError::Auth`] for a wrong account, name or password.
    pub fn connect(settings: SmbSettings) -> Result<Self, RemoteError> {
        let client = Self::new_client(&settings)?;
        Ok(Self {
            settings,
            client: Mutex::new(std::sync::Arc::new(client)),
        })
    }

    fn new_client(settings: &SmbSettings) -> Result<Client, RemoteError> {
        let client = Client::new(ClientConfig {
            connection: ConnectionConfig {
                port: settings.port,
                ..Default::default()
            },
            ..Default::default()
        });
        let share = Self::share_path(settings)?;
        client
            .share_connect(&share, &settings.user, settings.password.to_string())
            .map_err(|error| classify(&error))?;
        Ok(client)
    }

    fn share_path(settings: &SmbSettings) -> Result<UncPath, RemoteError> {
        UncPath::new(&settings.host)
            .and_then(|path| path.with_share(&settings.share))
            .map_err(|error| RemoteError::Protocol(error.to_string()))
    }

    fn client(&self) -> std::sync::Arc<Client> {
        self.client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Run `operation`; when it fails because the link is gone, connect again and try once more.
    fn with_retry<T>(
        &self,
        mut operation: impl FnMut(&Client) -> Result<T, smb::Error>,
    ) -> Result<T, RemoteError> {
        let client = self.client();
        match operation(&client) {
            Ok(value) => Ok(value),
            Err(error) => match classify(&error) {
                RemoteError::Network(_) => {
                    let fresh = std::sync::Arc::new(Self::new_client(&self.settings)?);
                    *self.client.lock().unwrap_or_else(PoisonError::into_inner) =
                        std::sync::Arc::clone(&fresh);
                    operation(&fresh).map_err(|error| classify(&error))
                }
                other => Err(other),
            },
        }
    }

    fn unc(&self, path: &str) -> Result<UncPath, RemoteError> {
        let windows = windows_path(&[&self.settings.base_path, path])?;
        Ok(Self::share_path(&self.settings)?.with_path(&windows))
    }
}

impl SmbFs {
    /// A cached `Read + Seek` over a file, tuned for SMB (see [`SMB_READ_AHEAD_BLOCKS`]).
    pub fn open_reader(&self, path: &str) -> Result<RangeReader, RemoteError> {
        Ok(RangeReader::new(self.open(path)?).with_read_ahead(SMB_READ_AHEAD_BLOCKS))
    }
}

impl Drop for SmbFs {
    fn drop(&mut self) {
        if let Ok(client) = self.client.get_mut() {
            let _ = client.close();
        }
    }
}

fn open_dir_args() -> FileCreateArgs {
    FileCreateArgs::make_open_existing(
        DirAccessMask::new()
            .with_list_directory(true)
            .with_synchronize(true)
            .into(),
    )
}

fn open_file_args() -> FileCreateArgs {
    FileCreateArgs::make_open_existing(FileAccessMask::new().with_generic_read(true))
}

impl RemoteFs for SmbFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        let target = self.unc(dir)?;
        self.with_retry(|client| {
            let directory = client.create_file(&target, &open_dir_args())?.unwrap_dir();
            let mut entries = Vec::new();
            let listing = directory.query::<FileFullDirectoryInformation>("*");
            let result = (|| {
                for item in listing? {
                    let item = item?;
                    let name = item.file_name.to_string();
                    if name == "." || name == ".." {
                        continue;
                    }
                    // Hidden and system entries ($RECYCLE.BIN, System Volume Information…) are not music.
                    if item.file_attributes.hidden() || item.file_attributes.system() {
                        continue;
                    }
                    entries.push(RemoteEntry {
                        name,
                        is_dir: item.file_attributes.directory(),
                        size: if item.file_attributes.directory() {
                            0
                        } else {
                            item.end_of_file
                        },
                        modified_ms: unix_ms(*item.last_write_time),
                    });
                }
                Ok(())
            })();
            let _ = directory.close();
            result.map(|()| entries)
        })
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        let target = self.unc(path)?;
        let name = path
            .rsplit(['/', '\\'])
            .find(|piece| !piece.is_empty())
            .unwrap_or("")
            .to_owned();
        self.with_retry(|client| {
            // Read attributes only, so this works for files and folders alike.
            let args = FileCreateArgs::make_open_existing(
                FileAccessMask::new().with_file_read_attributes(true),
            );
            let resource = client.create_file(&target, &args)?;
            let (basic, standard) = match &resource {
                Resource::File(file) => (
                    file.query_info::<FileBasicInformation>()?,
                    file.query_info::<FileStandardInformation>()?,
                ),
                Resource::Directory(dir) => (
                    dir.query_info::<FileBasicInformation>()?,
                    dir.query_info::<FileStandardInformation>()?,
                ),
                Resource::Pipe(_) => return Err(smb::Error::NotFound(name.clone())),
            };
            let is_dir = standard.directory.into();
            match resource {
                Resource::File(file) => {
                    let _ = file.close();
                }
                Resource::Directory(dir) => {
                    let _ = dir.close();
                }
                Resource::Pipe(_) => {}
            }
            Ok(RemoteEntry {
                name: name.clone(),
                is_dir,
                size: if is_dir { 0 } else { standard.end_of_file },
                modified_ms: unix_ms(*basic.last_write_time),
            })
        })
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        let target = self.unc(path)?;
        let (file, len) = self.with_retry(|client| {
            let resource = client.create_file(&target, &open_file_args())?;
            let Resource::File(file) = resource else {
                if let Resource::Directory(dir) = resource {
                    let _ = dir.close();
                }
                return Err(smb::Error::NotFound(path.to_owned()));
            };
            let len = file.query_info::<FileStandardInformation>()?.end_of_file;
            Ok((file, len))
        })?;
        Ok(Box::new(SmbFile { file, len }))
    }
}

/// An open SMB file. Reading is `read_at`, which the `smb` crate splits into as many requests as the negotiated
/// maximum read size needs.
pub struct SmbFile {
    file: smb::File,
    len: u64,
}

impl RemoteFile for SmbFile {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        if offset >= self.len || buf.is_empty() {
            return Ok(0);
        }
        let wanted = ((self.len - offset).min(buf.len() as u64)) as usize;
        let mut done = 0;
        while done < wanted {
            let read = self
                .file
                .read_at(&mut buf[done..wanted], offset + done as u64)
                .map_err(|error| classify(&error))?;
            if read == 0 {
                break;
            }
            done += read;
        }
        Ok(done)
    }
}

impl Drop for SmbFile {
    fn drop(&mut self) {
        let _ = self.file.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_become_windows_paths_and_cannot_escape() {
        assert_eq!(windows_path(&["", "a/b/c.flac"]).unwrap(), "a\\b\\c.flac");
        assert_eq!(windows_path(&["Music", "/a//b/"]).unwrap(), "Music\\a\\b");
        assert_eq!(windows_path(&["Music/Loss", ""]).unwrap(), "Music\\Loss");
        assert_eq!(windows_path(&["", ""]).unwrap(), "");
        assert_eq!(windows_path(&["", "a/./b"]).unwrap(), "a\\b");
        assert_eq!(windows_path(&["", "../x"]), Err(RemoteError::NotFound));
        assert_eq!(windows_path(&["m", "a\\..\\x"]), Err(RemoteError::NotFound));
    }

    #[test]
    fn filetime_converts_to_unix_milliseconds() {
        assert_eq!(unix_ms(0), None);
        // 1970-01-01T00:00:00Z
        assert_eq!(unix_ms(116_444_736_000_000_000), Some(0));
        // 2026-10-20T00:00:00Z
        assert_eq!(unix_ms(134_369_280_000_000_000), Some(1_792_454_400_000));
    }

    #[test]
    fn statuses_are_classified() {
        use smb::Error as E;
        assert_eq!(
            classify(&E::UnexpectedMessageStatus(status::LOGON_FAILURE)),
            RemoteError::Auth
        );
        assert_eq!(
            classify(&E::UnexpectedMessageStatus(status::ACCESS_DENIED)),
            RemoteError::Auth
        );
        assert_eq!(
            classify(&E::UnexpectedMessageStatus(status::OBJECT_NAME_NOT_FOUND)),
            RemoteError::NotFound
        );
        assert_eq!(
            classify(&E::UnexpectedMessageStatus(status::BAD_NETWORK_NAME)),
            RemoteError::NotFound
        );
        assert!(matches!(
            classify(&E::ConnectionStopped),
            RemoteError::Network(_)
        ));
    }

    #[test]
    fn the_password_never_shows_in_debug_output() {
        let settings = SmbSettings {
            host: "h".into(),
            port: None,
            share: "s".into(),
            base_path: String::new(),
            user: "u".into(),
            password: Zeroizing::new("hunter2-secret".into()),
        };
        let text = format!("{settings:?}");
        assert!(!text.contains("hunter2-secret"));
        assert!(text.contains("<hidden>"));
    }
}
