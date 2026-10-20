//! Open connections to remote servers (ADR 0026): one per account, made when first needed and kept while the account is
//! unchanged, so a library scan, the tag reads and playback share one signed-in connection instead of each logging in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use player_core::{MediaLocator, SourceId};
use player_remote::account::{CredentialVault, RemoteAccount, RemoteProtocol};
use player_remote::fs::{RemoteError, RemoteFs};
use player_remote::index::{RemoteResolver, ResolvedSource};
use player_remote::smb::{SmbFs, SmbSettings};
use player_remote::webdav::WebDavFs;

use crate::settings::{AppSettings, SourceEntryKind};

/// What identifies an account's connection: any change (host, share, user, password…) makes a new one.
fn fingerprint(account: &RemoteAccount) -> String {
    format!(
        "{:?}|{}|{:?}|{}|{}|{:?}|{:?}",
        account.protocol,
        account.host,
        account.port,
        account.path,
        account.username,
        account.tls,
        account.password
    )
}

#[derive(Default)]
pub(crate) struct RemoteConnections {
    open: Mutex<HashMap<String, (String, Arc<dyn RemoteFs>)>>,
}

impl RemoteConnections {
    /// The (shared) connection of an account, made now when there is none or the account changed.
    pub(crate) fn connect(
        &self,
        data_dir: &Path,
        account: &RemoteAccount,
    ) -> Result<Arc<dyn RemoteFs>, RemoteError> {
        let print = fingerprint(account);
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((existing, fs)) = open.get(&account.id) {
            if *existing == print {
                return Ok(Arc::clone(fs));
            }
        }
        // The lock is held while connecting: a second caller for the same account waits for this connection instead
        // of making another, which is what a scan's parallel readers need.
        let fs = connect_account(data_dir, account)?;
        open.insert(account.id.clone(), (print, Arc::clone(&fs)));
        Ok(fs)
    }

    /// Forget an account's connection (it was edited, deleted or failed).
    pub(crate) fn forget(&self, account_id: &str) {
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account_id);
    }

    /// A resolver for the remote sources in `settings`: which account and folder each source is.
    pub(crate) fn resolver(
        self: &Arc<Self>,
        data_dir: PathBuf,
        settings: &AppSettings,
    ) -> Arc<dyn RemoteResolver> {
        let mut sources = HashMap::new();
        for source in &settings.sources {
            let SourceEntryKind::Folder {
                path,
                remote_account: Some(account_id),
                ..
            } = &source.kind
            else {
                continue;
            };
            let base = match path.to_media_locator() {
                Ok(MediaLocator::Remote { path, .. }) => path,
                _ => continue,
            };
            if let Some(account) = settings
                .remote_accounts
                .iter()
                .find(|account| &account.id == account_id)
            {
                sources.insert(source.id, (account.clone(), base));
            }
        }
        Arc::new(SourceResolver {
            connections: Arc::clone(self),
            data_dir,
            sources,
        })
    }
}

struct SourceResolver {
    connections: Arc<RemoteConnections>,
    data_dir: PathBuf,
    sources: HashMap<SourceId, (RemoteAccount, String)>,
}

impl RemoteResolver for SourceResolver {
    fn resolve(&self, source_id: SourceId) -> Result<ResolvedSource, RemoteError> {
        let (account, base_path) = self.sources.get(&source_id).ok_or(RemoteError::NotFound)?;
        let fs = self.connections.connect(&self.data_dir, account)?;
        Ok(ResolvedSource {
            fs,
            base_path: base_path.clone(),
        })
    }
}

/// Connect to an account's server and sign in.
pub(crate) fn connect_account(
    data_dir: &Path,
    account: &RemoteAccount,
) -> Result<Arc<dyn RemoteFs>, RemoteError> {
    let vault = CredentialVault::open(&data_dir.join(crate::remote_accounts::KEY_FILE_NAME))
        .map_err(|error| RemoteError::Protocol(error.to_string()))?;
    let password = vault
        .password_of(account)
        .map_err(|error| RemoteError::Protocol(error.to_string()))?;
    match account.protocol {
        RemoteProtocol::Smb => Ok(Arc::new(SmbFs::connect(SmbSettings {
            host: account.host.clone(),
            port: account.port,
            share: account.path.clone(),
            base_path: String::new(),
            user: account.username.clone(),
            password: password.unwrap_or_default(),
        })?)),
        RemoteProtocol::Webdav => Ok(Arc::new(WebDavFs::new(account, password)?)),
    }
}

/// A resolver for when no remote source is involved (tests, platforms without remote sources): every source is
/// unknown.
#[allow(dead_code)]
pub(crate) fn none() -> Arc<dyn RemoteResolver> {
    struct NoRemote;
    impl RemoteResolver for NoRemote {
        fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
            Err(RemoteError::NotFound)
        }
    }
    Arc::new(NoRemote)
}

/// Opens remote files for the Windows audio engine (ADR 0026): the engine hands over `moeremote://<source id>/<path>`, this
/// finds the source's account, reuses its open connection and returns a cached reader over the file.
#[cfg(target_os = "windows")]
pub(crate) struct RemotePlayback {
    pub(crate) connections: Arc<RemoteConnections>,
    pub(crate) settings: Arc<crate::settings::SettingsStore>,
}

#[cfg(target_os = "windows")]
impl player_audio_windows::remote_input::RemoteOpener for RemotePlayback {
    fn open(
        &self,
        locator: &str,
    ) -> std::io::Result<player_audio_windows::remote_input::RemoteInput> {
        use player_audio_windows::remote_input::RemoteInput;
        use player_remote::fs::open_reader;
        use player_remote::index::join_path;
        use std::io::{Error, ErrorKind};

        let Some(MediaLocator::Remote { source_id, path }) =
            player_core::parse_remote_locator_uri(locator)
        else {
            return Err(Error::new(ErrorKind::InvalidInput, "not a remote location"));
        };
        let data_dir = self
            .settings
            .data_directory()
            .map(Path::to_path_buf)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "no UserData folder"))?;
        let settings = self
            .settings
            .snapshot()
            .map_err(|error| Error::other(error.to_string()))?;
        let resolved = self
            .connections
            .resolver(data_dir, &settings)
            .resolve(source_id)?;
        let full = join_path(&resolved.base_path, &path);
        let main = open_reader(resolved.fs.as_ref(), &full)?;
        let len = main.len();
        // A hybrid WavPack file is lossless with its `.wvc` correction file next to it.
        let correction = if full.to_ascii_lowercase().ends_with(".wv") {
            open_reader(resolved.fs.as_ref(), &format!("{full}c"))
                .ok()
                .map(|reader| Box::new(reader) as Box<dyn player_audio_windows::remote_input::ReadSeek>)
        } else {
            None
        };
        Ok(RemoteInput {
            main: Box::new(main),
            len,
            correction,
        })
    }
}
