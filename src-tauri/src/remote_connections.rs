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
        let mut excluded: HashMap<String, Vec<String>> = HashMap::new();
        for folder in &settings.remote_excluded_folders {
            excluded
                .entry(folder.account_id.clone())
                .or_default()
                .push(folder.path.clone());
        }
        for source in &settings.sources {
            let (path, account_id, is_playlist) = match &source.kind {
                SourceEntryKind::Folder {
                    path,
                    remote_account: Some(account_id),
                    ..
                } => (path, account_id, false),
                SourceEntryKind::PlaylistFile {
                    path,
                    remote_account: Some(account_id),
                    ..
                } => (path, account_id, true),
                _ => continue,
            };
            let base = match path.to_media_locator() {
                // The songs of a playlist are named from the account's root, whatever folder the list is in.
                Ok(MediaLocator::Remote { .. }) if is_playlist => String::new(),
                Ok(MediaLocator::Remote { path, .. }) => path,
                _ => continue,
            };
            if let Some(account) = settings
                .remote_accounts
                .iter()
                .find(|account| &account.id == account_id)
            {
                let excluded = excluded.get(&account.id).cloned().unwrap_or_default();
                sources.insert(source.id, (account.clone(), base, excluded));
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
    sources: HashMap<SourceId, (RemoteAccount, String, Vec<String>)>,
}

impl RemoteResolver for SourceResolver {
    fn resolve(&self, source_id: SourceId) -> Result<ResolvedSource, RemoteError> {
        let (account, base_path, excluded) =
            self.sources.get(&source_id).ok_or(RemoteError::NotFound)?;
        let fs = self.connections.connect(&self.data_dir, account)?;
        Ok(ResolvedSource {
            fs,
            base_path: base_path.clone(),
            excluded: excluded.clone(),
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
            // The account's path is `share` or `share/folder/...`.
            share: account.path.split('/').next().unwrap_or_default().to_owned(),
            base_path: account.path.split_once('/').map(|(_, rest)| rest.to_owned()).unwrap_or_default(),
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

/// Opens remote files for everything that reads them (ADR 0026): the audio engine, the cover and lyrics lookups. It
/// finds the source's account, reuses its open connection and returns a cached reader over the file.
pub(crate) struct RemoteFiles {
    pub(crate) connections: Arc<RemoteConnections>,
    pub(crate) settings: Arc<crate::settings::SettingsStore>,
    /// The disk cache of played remote files (`None` when its folder could not be made).
    pub(crate) cache: Option<Arc<player_remote::cache::DiskCache>>,
}

impl RemoteFiles {
    /// The file `path` of a source: its server connection and the full path under the account's root.
    fn locate(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<(std::sync::Arc<dyn player_remote::fs::RemoteFs>, String)> {
        use player_remote::index::join_path;
        use std::io::{Error, ErrorKind};

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
        let full = join_path(&resolved.base_path, path);
        Ok((resolved.fs, full))
    }
}

impl player_core::RemoteContent for RemoteFiles {
    fn open(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<player_core::RemoteFileHandle> {
        let (fs, full) = self.locate(source_id, path)?;
        let reader = player_remote::fs::open_reader(fs.as_ref(), &full)?;
        let len = reader.len();
        Ok(player_core::RemoteFileHandle {
            reader: Box::new(reader),
            len,
        })
    }

    fn open_streaming(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<player_core::RemoteFileHandle> {
        let (fs, full) = self.locate(source_id, path)?;
        let file = fs.open(&full)?;
        // Chunks that were played are kept on disk (first in, first out), so the same song played again does not go back
        // to the server. The key includes the size and modified time, so an edited file is not served stale.
        let file: Box<dyn player_remote::fs::RemoteFile> = match &self.cache {
            Some(cache) => {
                let modified = fs.stat(&full).ok().and_then(|entry| entry.modified_ms);
                let key = player_remote::cache::cache_key(
                    &source_id.to_string(),
                    &full,
                    file.len(),
                    modified,
                );
                Box::new(player_remote::cache::CachedFile::new(
                    file,
                    Arc::clone(cache),
                    key,
                ))
            }
            None => file,
        };
        let reader = player_remote::prefetch::PrefetchReader::new(file);
        let len = reader.len();
        Ok(player_core::RemoteFileHandle {
            reader: Box::new(reader),
            len,
        })
    }
}

/// The whole app-side chain against a real SMB server: an account saved with a sealed password, a source added under
/// it, a library sync through the shared connection, the playback path of a scanned track, and the reader the audio
/// engine would get for it. Skipped unless `MOE_SMB_HOST`, `MOE_SMB_SHARE`, `MOE_SMB_FILE` (user name, then password)
/// and optionally `MOE_SMB_PORT` and `MOE_SMB_DIR` are set; Windows only because the engine's opener is.
#[cfg(test)]
mod real_server_tests {
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::Arc;

    use player_core::RemoteContent;
    use player_core::{LibraryRoot, ListTracksQuery, MediaLocator, MediaSourceKind, SourceId, SyncEngine};
    use player_db::Database;
    use player_remote::account::{RemoteProtocol, TlsMode};
    use player_remote::index::RemoteMediaIndex;

    use super::{RemoteConnections, RemoteFiles};
    use crate::remote_accounts::{save_account, vault, RemoteAccountInput};
    use crate::settings::{AppSettings, SettingsStore, SourceEntry, SourceEntryKind, StoredPath};

    #[test]
    fn account_source_sync_and_playback_reader_work_against_a_real_server() {
        let (Ok(host), Ok(share), Ok(file)) = (
            std::env::var("MOE_SMB_HOST"),
            std::env::var("MOE_SMB_SHARE"),
            std::env::var("MOE_SMB_FILE"),
        ) else {
            eprintln!("skipped: MOE_SMB_* is not set");
            return;
        };
        let credentials = std::fs::read_to_string(file).unwrap();
        let mut lines = credentials.lines();
        let user = lines.next().unwrap().trim().to_owned();
        let password = lines.next().unwrap().trim().to_owned();
        let base = std::env::var("MOE_SMB_DIR").unwrap_or_default();

        let data_dir = std::env::temp_dir().join(format!("moe-remote-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let settings = Arc::new(
            SettingsStore::open(data_dir.join("settings.json"), AppSettings::default()).unwrap(),
        );
        let vault = vault(&data_dir).unwrap();

        // 1. The account: the password is sealed, never plaintext, in settings.json.
        let view = save_account(
            &settings,
            &vault,
            RemoteAccountInput {
                id: None,
                name: "test server".to_owned(),
                protocol: RemoteProtocol::Smb,
                host,
                port: std::env::var("MOE_SMB_PORT").ok().and_then(|port| port.parse().ok()),
                path: share,
                username: user,
                password: Some(password.clone()),
                tls: TlsMode::Verify,
                only_on_wifi: false,
            },
        )
        .unwrap();
        assert!(view.has_password);
        let saved = std::fs::read_to_string(data_dir.join("settings.json")).unwrap();
        assert!(!saved.contains(&password), "the password is in settings.json in the clear");

        // 2. The source under it.
        let id = SourceId::new();
        let root = LibraryRoot {
            id,
            kind: MediaSourceKind::Remote,
            display_name: "remote".to_owned(),
            locator: MediaLocator::Remote { source_id: id, path: base.clone() },
            enabled: true,
        };
        settings
            .upsert_source(SourceEntry {
                id,
                display_name: root.display_name.clone(),
                enabled: true,
                kind: SourceEntryKind::Folder {
                    media_kind: MediaSourceKind::Remote,
                    path: StoredPath::from_locator(&root.locator).unwrap(),
                    remote_account: Some(view.id.clone()),
                },
            })
            .unwrap();
        let snapshot = settings.snapshot().unwrap();

        // 3. The sync, through the app's resolver.
        let connections = Arc::new(RemoteConnections::default());
        let resolver = connections.resolver(data_dir.clone(), &snapshot);
        let mut index = RemoteMediaIndex::new(resolver);
        let mut database = Database::open_in_memory().unwrap();
        database.save_library_root(&root).unwrap();
        let report = SyncEngine::sync(&root, &mut index, &mut database, 1_800_000_000_000).unwrap();
        println!("observed {} tracks, state {:?}", report.observed, report.state);
        assert!(report.observed > 0);
        assert!(report.metadata_errors.is_empty(), "{:?}", report.metadata_errors);

        // 4. The playback path of a track, and the reader the engine gets for it.
        let page = database
            .list_tracks_page(ListTracksQuery {
                offset: 0,
                limit: 3,
                query: None,
                field_filter: None,
                sort: Default::default(),
            })
            .unwrap();
        let track = &page.items[0];
        let path = database.resolve_playable_filesystem_locator(track.id).unwrap();
        let text = path.to_string_lossy().into_owned();
        assert!(text.starts_with("moeremote://"), "{text}");
        let opener = RemoteFiles { connections: Arc::clone(&connections), settings: Arc::clone(&settings), cache: Some(Arc::new(player_remote::cache::DiskCache::open(data_dir.join("remote-cache"), 200_000_000).unwrap())) };
        let Some(MediaLocator::Remote { source_id, path: relative }) =
            player_core::parse_remote_locator_uri(&text)
        else {
            panic!("a remote pseudo path");
        };
        let mut input = opener.open(source_id, &relative).expect("the shared opener");
        assert!(input.len > 0);
        let mut head = [0u8; 4];
        input.reader.read_exact(&mut head).unwrap();
        input.reader.seek(SeekFrom::End(-16)).unwrap();
        let mut tail = Vec::new();
        input.reader.read_to_end(&mut tail).unwrap();
        println!("{:?} {} bytes, head {:?}", track.title, input.len, head);
        assert_eq!(tail.len(), 16);

        // 5. The playback stream: read at roughly lossless-playback pace (about 1 MB/s) and see how long the slowest
        // read waits once it has started (the audio thread must never wait for the network).
        let mut stream = opener
            .open_streaming(source_id, &relative)
            .expect("the streaming opener");
        let mut chunk = vec![0u8; 32 * 1024];
        stream.reader.read_exact(&mut chunk).unwrap();
        let mut worst = std::time::Duration::ZERO;
        let mut total = 0usize;
        let started = std::time::Instant::now();
        while total < 12 * 1024 * 1024 && total + 2 * chunk.len() < stream.len as usize {
            std::thread::sleep(std::time::Duration::from_millis(8));
            let before = std::time::Instant::now();
            stream.reader.read_exact(&mut chunk).unwrap();
            worst = worst.max(before.elapsed());
            total += chunk.len();
        }
        println!(
            "streamed {} MiB in {:.1}s, slowest read {:?}",
            total / (1024 * 1024),
            started.elapsed().as_secs_f64(),
            worst
        );
        assert!(worst < std::time::Duration::from_millis(100), "a read waited {worst:?}");
        drop(stream);

        // 6. The disk cache: what was streamed is on disk now, and the same song played again is read from there.
        let used = opener.cache.as_ref().unwrap().used();
        println!("cache holds {} MB", used / 1_000_000);
        assert!(used >= 1_000_000, "nothing was cached");
        let started = std::time::Instant::now();
        let mut again = opener
            .open_streaming(source_id, &relative)
            .expect("the streaming opener again");
        let mut total_again = 0usize;
        while total_again < 6 * 1024 * 1024 {
            again.reader.read_exact(&mut chunk).unwrap();
            total_again += chunk.len();
        }
        println!("6 MiB again from the cache in {:?}", started.elapsed());

        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
