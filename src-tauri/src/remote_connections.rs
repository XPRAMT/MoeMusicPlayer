//! Open connections to remote servers (ADR 0026): one per account, made when first needed and kept while the account is
//! unchanged, so a library scan, the tag reads and playback share one signed-in connection instead of each logging in.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use player_core::{MediaLocator, SourceId};
use player_remote::account::{CredentialVault, RemoteAccount, RemoteProtocol};
use player_remote::fs::{FileStamp, RemoteEntry, RemoteError, RemoteFile, RemoteFs};
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

/// How long a server that could not be reached is left alone: every use in that time fails at once instead of waiting
/// for another connection attempt to time out.
pub(crate) const UNREACHABLE_BACKOFF: Duration = Duration::from_secs(15);

/// Whether an account's server is known to be unreachable: set when connecting or any operation on the open connection
/// ends in a network error (after the protocol client's own retry), cleared by the next success once the backoff is over.
/// Shared by the slot and every [`GuardedFs`] handed out for it, so the mark is made in this one place.
struct LinkHealth {
    backoff: Duration,
    down: Mutex<Option<(Instant, String)>>,
}

impl LinkHealth {
    fn new(backoff: Duration) -> Self {
        Self {
            backoff,
            down: Mutex::new(None),
        }
    }

    /// `Err` (the error that started the backoff) while the backoff lasts.
    fn check(&self) -> Result<(), RemoteError> {
        match &*self.down.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((since, message)) if since.elapsed() < self.backoff => {
                Err(RemoteError::Network(message.clone()))
            }
            _ => Ok(()),
        }
    }

    /// The backoff was started and has run out: the connection that failed should be replaced.
    fn expired(&self) -> bool {
        self.down
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|(since, _)| since.elapsed() >= self.backoff)
    }

    fn mark(&self, message: &str) {
        *self.down.lock().unwrap_or_else(PoisonError::into_inner) =
            Some((Instant::now(), message.to_owned()));
    }

    fn clear(&self) {
        *self.down.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// The connection every caller gets: it answers at once while the server is in its backoff, and a network error of any
/// operation starts the backoff, so one dead server costs one slow failure and then none. SMB and WebDAV both come
/// through here.
struct GuardedFs {
    inner: Arc<dyn RemoteFs>,
    health: Arc<LinkHealth>,
}

impl GuardedFs {
    fn run<T>(
        &self,
        operation: impl FnOnce(&dyn RemoteFs) -> Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        self.health.check()?;
        match operation(self.inner.as_ref()) {
            Ok(value) => {
                self.health.clear();
                Ok(value)
            }
            Err(RemoteError::Network(message)) => {
                self.health.mark(&message);
                Err(RemoteError::Network(message))
            }
            Err(error) => Err(error),
        }
    }
}

impl RemoteFs for GuardedFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.run(|fs| fs.list(dir))
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.run(|fs| fs.stat(path))
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        self.run(|fs| fs.open(path))
    }

    fn replace_file(
        &self,
        path: &str,
        source: &mut dyn std::io::Read,
        len: u64,
        expected: Option<FileStamp>,
    ) -> Result<RemoteEntry, RemoteError> {
        self.run(|fs| fs.replace_file(path, source, len, expected))
    }

    fn remove_file(&self, path: &str) -> Result<(), RemoteError> {
        self.run(|fs| fs.remove_file(path))
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        self.run(|fs| fs.rename_file(from, to))
    }

    fn replace_file_keeping_backup(
        &self,
        path: &str,
        source: &mut dyn std::io::Read,
        len: u64,
        expected: Option<FileStamp>,
        backup: &str,
    ) -> Result<RemoteEntry, RemoteError> {
        self.run(|fs| fs.replace_file_keeping_backup(path, source, len, expected, backup))
    }

    fn read_ahead_blocks(&self) -> u64 {
        self.inner.read_ahead_blocks()
    }
}

/// One account's connection: its own lock (so connecting to one server never waits on another, and nothing else in the
/// app waits on it), the open connection, and whether the server is known to be unreachable.
struct AccountSlot {
    print: String,
    health: Arc<LinkHealth>,
    state: Mutex<SlotState>,
}

#[derive(Default)]
struct SlotState {
    fs: Option<Arc<dyn RemoteFs>>,
}

#[cfg(test)]
type TestMaker = Arc<
    dyn Fn(&RemoteAccount) -> Result<Arc<dyn RemoteFs>, RemoteError>
        + Send
        + Sync
        + std::panic::RefUnwindSafe,
>;

#[derive(Default)]
pub(crate) struct RemoteConnections {
    open: Mutex<HashMap<String, Arc<AccountSlot>>>,
    /// Test hook: a backoff other than [`UNREACHABLE_BACKOFF`].
    backoff: Option<Duration>,
    /// Test hook: how a connection is made instead of [`connect_account`].
    #[cfg(test)]
    maker: Option<TestMaker>,
}

impl RemoteConnections {
    /// The (shared) connection of an account, made now when there is none or the account changed.
    pub(crate) fn connect(
        &self,
        data_dir: &Path,
        account: &RemoteAccount,
    ) -> Result<Arc<dyn RemoteFs>, RemoteError> {
        #[cfg(test)]
        if let Some(maker) = &self.maker {
            return self.connect_with(account, || maker(account));
        }
        self.connect_with(account, || connect_account(data_dir, account))
    }

    /// [`connect`](Self::connect) with the way to make a connection given (tests).
    fn connect_with(
        &self,
        account: &RemoteAccount,
        make: impl FnOnce() -> Result<Arc<dyn RemoteFs>, RemoteError>,
    ) -> Result<Arc<dyn RemoteFs>, RemoteError> {
        let print = fingerprint(account);
        // The map is locked only to find the account's slot, never while connecting.
        let slot = {
            let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            match open.get(&account.id) {
                Some(slot) if slot.print == print => Arc::clone(slot),
                _ => {
                    let slot = Arc::new(AccountSlot {
                        print,
                        health: Arc::new(LinkHealth::new(
                            self.backoff.unwrap_or(UNREACHABLE_BACKOFF),
                        )),
                        state: Mutex::new(SlotState::default()),
                    });
                    open.insert(account.id.clone(), Arc::clone(&slot));
                    slot
                }
            }
        };
        // The account's own lock is held while connecting: a second caller for the same account waits for this
        // connection instead of making another, which is what a scan's parallel readers need.
        let mut state = slot.state.lock().unwrap_or_else(PoisonError::into_inner);
        slot.health.check()?;
        if slot.health.expired() {
            // The connection that failed is replaced, not tried again; letting go of a dead link can take a while.
            if let Some(old) = state.fs.take() {
                let _ = std::thread::Builder::new()
                    .name("moe-remote-drop".to_owned())
                    .spawn(move || drop(old));
            }
        }
        if let Some(fs) = &state.fs {
            return Ok(Arc::clone(fs));
        }
        match make() {
            Ok(inner) => {
                slot.health.clear();
                let fs: Arc<dyn RemoteFs> = Arc::new(GuardedFs {
                    inner,
                    health: Arc::clone(&slot.health),
                });
                state.fs = Some(Arc::clone(&fs));
                Ok(fs)
            }
            Err(RemoteError::Network(message)) => {
                slot.health.mark(&message);
                Err(RemoteError::Network(message))
            }
            Err(error) => Err(error),
        }
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
            if let Some((account, base)) = remote_source_account(settings, source) {
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

/// The account a remote source is on and the folder it is rooted at (under the account's root); `None` for a source that
/// is not a remote one or whose account is gone. Nothing is connected.
fn remote_source_account<'a>(
    settings: &'a AppSettings,
    source: &crate::settings::SourceEntry,
) -> Option<(&'a RemoteAccount, String)> {
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
        _ => return None,
    };
    let base = match path.to_media_locator() {
        // The songs of a playlist are named from the account's root, whatever folder the list is in.
        Ok(MediaLocator::Remote { .. }) if is_playlist => String::new(),
        Ok(MediaLocator::Remote { path, .. }) => path,
        _ => return None,
    };
    let account = settings
        .remote_accounts
        .iter()
        .find(|account| &account.id == account_id)?;
    Some((account, base))
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

    fn account_id(&self, source_id: SourceId) -> String {
        self.sources
            .get(&source_id)
            .map_or_else(|| source_id.to_string(), |(account, _, _)| account.id.clone())
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
        RemoteProtocol::Unsupported => Err(RemoteError::Unsupported(
            "這個版本不支援此伺服器的通訊協定，請更新播放器".to_owned(),
        )),
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
    /// The streams that are open for playback, so they can be let go of before their file is replaced.
    pub(crate) streams: Mutex<Vec<OpenStream>>,
}

/// One file held open for playback: which one, and how to let go of it.
pub(crate) struct OpenStream {
    source_id: SourceId,
    full_path: String,
    release: player_remote::fs::FileRelease,
}

impl RemoteFiles {
    /// Remember a stream that was opened (and forget the ones that are finished).
    fn track_stream(&self, source_id: SourceId, full_path: &str, release: player_remote::fs::FileRelease) {
        let mut streams = self.streams.lock().unwrap_or_else(PoisonError::into_inner);
        streams.retain(|stream| !stream.release.is_finished());
        streams.push(OpenStream {
            source_id,
            full_path: full_path.to_owned(),
            release,
        });
    }
}

/// Where a remote file is, without a connection: the account that holds it and its full path under the account's root.
struct Located {
    data_dir: PathBuf,
    account: RemoteAccount,
    full: String,
}

/// Files being checked against their server in the background (source id and full path), so a song opened again and
/// again has one check under way, not a pile of them.
static VALIDATING: std::sync::LazyLock<Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// A connection that is only made when something asks the server: the stand-in under a file that is whole in the disk
/// cache. Reads that the cache answers never get here; when one does (a chunk was evicted meanwhile) it connects like
/// any other use of the account, so it fails at once while the server is known down and never waits for it otherwise.
struct DeferredFs {
    connections: Arc<RemoteConnections>,
    data_dir: PathBuf,
    account: RemoteAccount,
}

impl DeferredFs {
    fn fs(&self) -> Result<Arc<dyn RemoteFs>, RemoteError> {
        self.connections.connect(&self.data_dir, &self.account)
    }
}

impl RemoteFs for DeferredFs {
    fn list(&self, dir: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
        self.fs()?.list(dir)
    }

    fn stat(&self, path: &str) -> Result<RemoteEntry, RemoteError> {
        self.fs()?.stat(path)
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        self.fs()?.open(path)
    }
}

impl RemoteFiles {
    /// The file `path` of a source: where it is, found from the settings alone (nothing is connected).
    fn plan(&self, source_id: SourceId, path: &str) -> std::io::Result<Located> {
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
        let (account, base) = settings
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .and_then(|source| remote_source_account(&settings, source))
            .ok_or_else(|| Error::from(ErrorKind::NotFound))?;
        Ok(Located {
            data_dir,
            account: account.clone(),
            full: join_path(&base, path),
        })
    }

    /// The file `path` of a source: its server connection and the full path under the account's root.
    fn locate(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<(std::sync::Arc<dyn player_remote::fs::RemoteFs>, String)> {
        let located = self.plan(source_id, path)?;
        let fs = self.connections.connect(&located.data_dir, &located.account)?;
        Ok((fs, located.full))
    }

    /// The song as the disk cache holds it when the whole file is there: read from the cache without a connection (the
    /// server may be on, off or slow; none of it matters), with the server asked in the background whether the file is
    /// still the same (`store`: playback only).
    fn complete_cached_file(
        &self,
        source_id: SourceId,
        located: &Located,
        store: bool,
    ) -> Option<Box<dyn player_remote::fs::RemoteFile>> {
        let cache = self.cache.as_ref()?;
        let account_key = source_id.to_string();
        let meta = cache.complete_meta(&account_key, &located.full)?;
        let key = player_remote::cache::cache_key(&account_key, &located.full, meta.len, meta.modified_ms);
        let deferred: Arc<dyn RemoteFs> = Arc::new(DeferredFs {
            connections: Arc::clone(&self.connections),
            data_dir: located.data_dir.clone(),
            account: located.account.clone(),
        });
        let (lazy, release) = player_remote::fs::releasable(Box::new(
            player_remote::fs::ReopeningFile::lazy(deferred, located.full.clone(), meta.len),
        ));
        self.track_stream(source_id, &located.full, release);
        if store {
            self.validate_in_background(source_id, located, meta);
            Some(Box::new(player_remote::cache::CachedFile::new(
                Box::new(lazy),
                Arc::clone(cache),
                key,
            )))
        } else {
            Some(Box::new(player_remote::cache::CachedFile::read_only(
                Box::new(lazy),
                Arc::clone(cache),
                key,
            )))
        }
    }

    /// Ask the server, on a thread of its own and never on the playback path, whether the file is still the one the
    /// cache remembers. When its size or time differ the remembered ones are replaced, so the next play uses (and
    /// fetches) the new version; what plays now keeps reading its cached bytes. A server that cannot be reached is not
    /// reported to anyone (the connection's own backoff notes it, as it does for any failed connect).
    fn validate_in_background(&self, source_id: SourceId, located: &Located, known: player_remote::cache::FileMeta) {
        let Some(cache) = self.cache.as_ref().map(Arc::clone) else {
            return;
        };
        let id = format!("{source_id}\n{}", located.full);
        if !VALIDATING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone())
        {
            return;
        }
        let connections = Arc::clone(&self.connections);
        let (data_dir, account, full) = (located.data_dir.clone(), located.account.clone(), located.full.clone());
        let account_key = source_id.to_string();
        let started = std::thread::Builder::new()
            .name("moe-remote-validate".to_owned())
            .spawn({
                let id = id.clone();
                move || {
                    if let Ok(entry) = connections
                        .connect(&data_dir, &account)
                        .and_then(|fs| fs.stat(&full))
                    {
                        let now = player_remote::cache::FileMeta {
                            len: entry.size,
                            modified_ms: entry.modified_ms,
                        };
                        // Only when the cache still remembers what it was asked about: another check or a save may have
                        // replaced it meanwhile.
                        if now != known && cache.meta(&account_key, &full) == Some(known) {
                            cache.put_meta(&account_key, &full, now);
                        }
                    }
                    VALIDATING
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&id);
                }
            });
        if started.is_err() {
            VALIDATING
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
        }
    }
}

impl player_core::RemoteContent for RemoteFiles {
    fn open(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<player_core::RemoteFileHandle> {
        // A song whose size and time were learned when it was played: the disk cache answers what it holds, so a cover or a
        // tag of a played song needs no network. What it lacks is fetched but not stored (the cache is for playing).
        // A song that is whole in the cache needs no connection at all.
        if let Some(cache) = self.cache.as_ref() {
            let located = self.plan(source_id, path)?;
            if cache.meta(&source_id.to_string(), &located.full).is_some() {
                let reader = player_remote::fs::RangeReader::new(self.cached_file(source_id, path, false)?);
                let len = reader.len();
                return Ok(player_core::RemoteFileHandle {
                    reader: Box::new(reader),
                    len,
                });
            }
        }
        let (fs, full) = self.locate(source_id, path)?;
        // Tags, covers and lyrics need the start of a file (and a little more): the default read-ahead of a few blocks is
        // plenty. The SMB-tuned read-ahead (8 MiB per request) is for playing a file from start to end; with it, a screenful of
        // album covers asked for at once is dozens of 8 MiB requests on one connection, which the server does not answer in time.
        let reader = player_remote::fs::RangeReader::new(fs.open(&full)?);
        let len = reader.len();
        Ok(player_core::RemoteFileHandle {
            reader: Box::new(reader),
            len,
        })
    }

    fn open_cached(
        &self,
        source_id: SourceId,
        path: &str,
    ) -> std::io::Result<player_core::RemoteFileHandle> {
        // A lyrics sidecar: kept in the disk cache like the audio of a played song (cache first, the server asked in the
        // background), so a song that plays without its server has its sidecar too. No read-ahead: it is small.
        let reader = player_remote::fs::RangeReader::new(self.cached_file(source_id, path, true)?);
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
        let file = self.cached_file(source_id, path, true)?;
        // Opened by a load that a newer choice of track replaces: its reads end at once (a stream that is playing is not cut).
        let reader = player_remote::prefetch::PrefetchReader::new(file)
            .with_cancel(player_core::load_cancel());
        let len = reader.len();
        Ok(player_core::RemoteFileHandle {
            reader: Box::new(reader),
            len,
        })
    }

    fn stat(&self, source_id: SourceId, path: &str) -> std::io::Result<player_core::RemoteStamp> {
        let (fs, full) = self.locate(source_id, path)?;
        let entry = fs.stat(&full)?;
        Ok(player_core::RemoteStamp {
            len: entry.size,
            modified_ms: entry.modified_ms,
            created_utc_100ns: entry.created_utc_100ns,
        })
    }

    fn replace_file(
        &self,
        source_id: SourceId,
        path: &str,
        local: &Path,
        expected: Option<player_core::RemoteStamp>,
    ) -> std::io::Result<player_core::RemoteStamp> {
        self.replace_remote_file(source_id, path, local, expected)
    }

    fn release(&self, source_id: SourceId, path: &str) {
        self.release_streams(source_id, path);
    }

    fn file_changed(&self, source_id: SourceId, path: &str, now: player_core::RemoteStamp) {
        // As the background check of a played song does (`validate_in_background`): the remembered size and time are
        // replaced, so the cache key of the next open names the new version; an open stream keeps its own key.
        let (Some(cache), Ok(located)) = (self.cache.as_ref(), self.plan(source_id, path)) else {
            return;
        };
        let account_key = source_id.to_string();
        let now = player_remote::cache::FileMeta {
            len: now.len,
            modified_ms: now.modified_ms,
        };
        if cache
            .meta(&account_key, &located.full)
            .is_some_and(|known| known != now)
        {
            cache.put_meta(&account_key, &located.full, now);
        }
    }

    fn available_offline(&self, source_id: SourceId, path: &str) -> bool {
        let Some(cache) = self.cache.as_ref() else {
            return false;
        };
        self.plan(source_id, path)
            .is_ok_and(|located| cache.complete_meta(&source_id.to_string(), &located.full).is_some())
    }

    fn server_key(&self, source_id: SourceId) -> String {
        // The account is the server: every source of one account is down together (`RemoteConnections`).
        let data_dir = self.settings.data_directory().map(Path::to_path_buf);
        match (data_dir, self.settings.snapshot()) {
            (Some(data_dir), Ok(settings)) => self
                .connections
                .resolver(data_dir, &settings)
                .account_id(source_id),
            _ => source_id.to_string(),
        }
    }
}

impl RemoteFiles {
    /// The file as playback reads it, without the read-ahead thread: the disk cache answers what it holds (a file whose
    /// size and time were learned before is opened without asking the server anything), and what it fetches is kept.
    fn cached_file(
        &self,
        source_id: SourceId,
        path: &str,
        store: bool,
    ) -> std::io::Result<Box<dyn player_remote::fs::RemoteFile>> {
        let located = self.plan(source_id, path)?;
        // Cache first: a song that is whole on disk is read from there without connecting, server on or off.
        if let Some(file) = self.complete_cached_file(source_id, &located, store) {
            return Ok(file);
        }
        let fs = self
            .connections
            .connect(&located.data_dir, &located.account)?;
        let full = located.full.clone();
        let account_key = source_id.to_string();
        // A file whose size and modified time were learned before (and whose chunks may be on disk) is opened without
        // asking the server about the file: reads that the disk cache can answer never touch the network, and the server
        // is asked in the background whether the file is still the same. This is what makes a song played before start
        // at once.
        let known = self
            .cache
            .as_ref()
            .and_then(|cache| cache.meta(&account_key, &full));
        let file: Box<dyn player_remote::fs::RemoteFile> = match (&self.cache, known) {
            (Some(cache), Some(meta)) => {
                let key = player_remote::cache::cache_key(
                    &account_key,
                    &full,
                    meta.len,
                    meta.modified_ms,
                );
                if store {
                    self.validate_in_background(source_id, &located, meta);
                }
                let (lazy, release) = player_remote::fs::releasable(Box::new(
                    player_remote::fs::ReopeningFile::lazy(Arc::clone(&fs), full.clone(), meta.len),
                ));
                self.track_stream(source_id, &full, release);
                if store {
                    Box::new(player_remote::cache::CachedFile::new(
                        Box::new(lazy),
                        Arc::clone(cache),
                        key,
                    ))
                } else {
                    Box::new(player_remote::cache::CachedFile::read_only(
                        Box::new(lazy),
                        Arc::clone(cache),
                        key,
                    ))
                }
            }
            (Some(cache), None) => {
                let (opened, release) = player_remote::fs::releasable(Box::new(
                    player_remote::fs::ReopeningFile::new(Arc::clone(&fs), full.clone(), fs.open(&full)?),
                ));
                self.track_stream(source_id, &full, release);
                let opened: Box<dyn player_remote::fs::RemoteFile> = Box::new(opened);
                // The key includes the size and modified time, so an edited file is not served stale.
                let modified = fs.stat(&full).ok().and_then(|entry| entry.modified_ms);
                let meta = player_remote::cache::FileMeta {
                    len: opened.len(),
                    modified_ms: modified,
                };
                cache.put_meta(&account_key, &full, meta);
                let key = player_remote::cache::cache_key(&account_key, &full, meta.len, meta.modified_ms);
                Box::new(player_remote::cache::CachedFile::new(
                    opened,
                    Arc::clone(cache),
                    key,
                ))
            }
            (None, _) => {
                let (opened, release) = player_remote::fs::releasable(Box::new(
                    player_remote::fs::ReopeningFile::new(Arc::clone(&fs), full.clone(), fs.open(&full)?),
                ));
                self.track_stream(source_id, &full, release);
                Box::new(opened)
            }
        };
        Ok(file)
    }

    fn replace_remote_file(
        &self,
        source_id: SourceId,
        path: &str,
        local: &Path,
        expected: Option<player_core::RemoteStamp>,
    ) -> std::io::Result<player_core::RemoteStamp> {
        let (fs, full) = self.locate(source_id, path)?;
        let mut file = std::fs::File::open(local)?;
        let len = file.metadata()?.len();
        let expected = expected.map(|stamp| player_remote::fs::FileStamp {
            len: stamp.len,
            modified_ms: stamp.modified_ms,
            created_utc_100ns: stamp.created_utc_100ns,
        });
        let entry = fs.replace_file(&full, &mut file, len, expected)?;
        // The song on the server is a new file now: what the cache holds of the old one is not served for it (its key
        // names the size and time), and the next play learns the new ones at once.
        if let Some(cache) = self.cache.as_ref() {
            cache.put_meta(
                &source_id.to_string(),
                &full,
                player_remote::cache::FileMeta {
                    len: entry.size,
                    modified_ms: entry.modified_ms,
                },
            );
        }
        Ok(player_core::RemoteStamp {
            len: entry.size,
            modified_ms: entry.modified_ms,
            created_utc_100ns: entry.created_utc_100ns,
        })
    }

    fn release_streams(&self, source_id: SourceId, path: &str) {
        // Nothing is connected to find the name: a stream of a song in the cache is let go of while its server is away too.
        let Ok(Located { full, .. }) = self.plan(source_id, path) else {
            return;
        };
        let streams = self.streams.lock().unwrap_or_else(PoisonError::into_inner);
        for stream in streams
            .iter()
            .filter(|stream| stream.source_id == source_id && stream.full_path == full)
        {
            stream.release.release();
        }
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
            .list_tracks_page(
                player_db::LibrarySourceScope::All,
                ListTracksQuery {
                    offset: 0,
                    limit: 3,
                    query: None,
                    field_filter: None,
                    sort: Default::default(),
                },
            )
            .unwrap();
        let track = &page.items[0];
        let path = database.resolve_playable_filesystem_locator(track.id).unwrap();
        let text = path.to_string_lossy().into_owned();
        assert!(text.starts_with("moeremote://"), "{text}");
        let opener = RemoteFiles { connections: Arc::clone(&connections), settings: Arc::clone(&settings), cache: Some(Arc::new(player_remote::cache::DiskCache::open(data_dir.join("remote-cache"), 200_000_000).unwrap())), streams: Default::default() };
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

    /// Edits a scratch song on a real server end to end (writes to it; set `MOE_SMB_WRITE_TEST=1`): a small MP3 is sent
    /// to the server, a stream of it is open for playback, its title is changed through the same function the panel uses
    /// (which lets go of the stream, fetches the file, edits it and sends it back), and the new title is read back from
    /// the server. The scratch file is deleted at the end; no other file is touched.
    #[test]
    fn a_song_on_a_real_server_is_edited_while_a_stream_of_it_is_open() {
        let (Ok(host), Ok(share), Ok(file)) = (
            std::env::var("MOE_SMB_HOST"),
            std::env::var("MOE_SMB_SHARE"),
            std::env::var("MOE_SMB_FILE"),
        ) else {
            eprintln!("skipped: MOE_SMB_* is not set");
            return;
        };
        if std::env::var("MOE_SMB_WRITE_TEST").as_deref() != Ok("1") {
            eprintln!("skipped: MOE_SMB_WRITE_TEST is not 1 (the test writes to the server)");
            return;
        }
        let credentials = std::fs::read_to_string(file).unwrap();
        let mut lines = credentials.lines();
        let user = lines.next().unwrap().trim().to_owned();
        let password = lines.next().unwrap().trim().to_owned();
        let base = std::env::var("MOE_SMB_DIR").unwrap_or_default();

        let data_dir = std::env::temp_dir().join(format!("moe-remote-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).unwrap();
        let settings = Arc::new(
            SettingsStore::open(data_dir.join("settings.json"), AppSettings::default()).unwrap(),
        );
        let vault = vault(&data_dir).unwrap();
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
                password: Some(password),
                tls: TlsMode::Verify,
                only_on_wifi: false,
            },
        )
        .unwrap();
        let id = SourceId::new();
        let locator = MediaLocator::Remote { source_id: id, path: base };
        settings
            .upsert_source(SourceEntry {
                id,
                display_name: "remote".to_owned(),
                enabled: true,
                kind: SourceEntryKind::Folder {
                    media_kind: MediaSourceKind::Remote,
                    path: StoredPath::from_locator(&locator).unwrap(),
                    remote_account: Some(view.id.clone()),
                },
            })
            .unwrap();
        let connections = Arc::new(RemoteConnections::default());
        let opener = Arc::new(RemoteFiles {
            connections,
            settings,
            cache: Some(Arc::new(
                player_remote::cache::DiskCache::open(data_dir.join("remote-cache"), 200_000_000).unwrap(),
            )),
            streams: Default::default(),
        });
        player_core::set_remote_content(opener.clone());

        let name = format!("moe-edit-test-{}.mp3", std::process::id());
        let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/player-audio-windows/tests/fixtures/seek-tone.mp3");
        let original_size = std::fs::metadata(&local).unwrap().len();
        let cleanup = || {
            if let Ok((fs, full)) = opener.locate(id, &name) {
                let _ = fs.remove_file(&full);
            }
        };
        cleanup();
        let result = std::panic::catch_unwind(|| {
            opener.replace_file(id, &name, &local, None).expect("upload the scratch song");

            // A stream of it is open, as playback would hold it, and has read-ahead running.
            let mut playing = opener.open_streaming(id, &name).expect("open for playback");
            let mut head = [0u8; 64];
            playing.reader.read_exact(&mut head).unwrap();

            // The title is changed the way the panel changes it.
            let before = crate::track_details::read_remote_details(id, &name).expect("read the details");
            assert_eq!(before.file_size, original_size);
            assert!(before.remote);
            assert!(!before.read_only);
            let new_title = format!("edited {}", std::process::id());
            let mut fields = before.fields.clone();
            fields.title = new_title.clone();
            let edit = player_tags::TagEdit {
                fields,
                replace_cover: None,
                remove_cover: false,
                cover_options: Default::default(),
            };
            let saved = crate::track_details::save_remote(id, &name, &edit).expect("save on the server");
            assert_eq!(saved.fields.title, new_title);

            // The handle of the stream that was open was closed (what it had already buffered is still its own).
            assert!(
                opener
                    .streams
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|stream| stream.release.is_finished()),
                "an open stream still holds the file"
            );
            drop(playing);

            // The server's file holds the new title and the audio of the old one.
            let after = crate::track_details::read_remote_details(id, &name).expect("read it back");
            assert_eq!(after.fields.title, new_title);
            assert_eq!(after.properties.duration_ms, before.properties.duration_ms);
            assert_ne!(after.file_size, 0);
        });
        cleanup();
        let _ = std::fs::remove_dir_all(&data_dir);
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

#[cfg(test)]
mod unreachable_server_tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use player_remote::account::{RemoteAccount, RemoteProtocol};
    use player_remote::fs::{RemoteError, RemoteFs};
    use player_remote::local::DirFs;

    use super::RemoteConnections;

    fn account(id: &str, host: &str) -> RemoteAccount {
        RemoteAccount {
            id: id.to_owned(),
            name: id.to_owned(),
            protocol: RemoteProtocol::Smb,
            host: host.to_owned(),
            port: None,
            path: "share".to_owned(),
            username: String::new(),
            password: None,
            tls: Default::default(),
            only_on_wifi: false,
        }
    }

    /// A server that does not answer: connecting to it hangs, then fails on the network.
    fn hanging_connect() -> Result<Arc<dyn RemoteFs>, RemoteError> {
        std::thread::sleep(Duration::from_millis(800));
        Err(RemoteError::Network("timed out".to_owned()))
    }

    #[test]
    fn an_unreachable_server_neither_blocks_another_account_nor_is_waited_for_twice() {
        let connections = Arc::new(RemoteConnections::default());
        let down = account("down", "10.255.255.1");
        let up = account("up", "127.0.0.1");

        let hanging = {
            let connections = Arc::clone(&connections);
            let down = down.clone();
            std::thread::spawn(move || connections.connect_with(&down, hanging_connect))
        };
        std::thread::sleep(Duration::from_millis(50));

        // Another account connects while the first one hangs: it does not wait for it.
        let started = Instant::now();
        let dir = std::env::temp_dir();
        let fs = connections.connect_with(&up, || Ok(Arc::new(DirFs::new(dir)) as Arc<dyn RemoteFs>));
        assert!(fs.is_ok());
        assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());

        assert!(matches!(hanging.join().unwrap(), Err(RemoteError::Network(_))));

        // The server that just failed is not tried again for a while: callers are told at once.
        let started = Instant::now();
        let again = connections.connect_with(&down, || panic!("must not reconnect while the server is known down"));
        assert!(matches!(again, Err(RemoteError::Network(_))));
        assert!(started.elapsed() < Duration::from_millis(50), "{:?}", started.elapsed());

        // Forgetting the account (it was edited) lets the next use try again.
        connections.forget("down");
        let retried = connections.connect_with(&down, || Err(RemoteError::Network("still down".to_owned())));
        assert!(matches!(retried, Err(RemoteError::Network(message)) if message == "still down"));
    }

    /// A server that answered when connected, then went away: every operation fails on the network, and counts how
    /// often it was really asked.
    struct Switch {
        up: AtomicBool,
        calls: AtomicUsize,
    }

    struct SwitchFs(Arc<Switch>);

    impl SwitchFs {
        fn answer(&self) -> Result<(), RemoteError> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            if self.0.up.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(RemoteError::Network("No route to host".to_owned()))
            }
        }
    }

    impl RemoteFs for SwitchFs {
        fn list(&self, _: &str) -> Result<Vec<player_remote::fs::RemoteEntry>, RemoteError> {
            self.answer().map(|()| Vec::new())
        }
        fn stat(&self, _: &str) -> Result<player_remote::fs::RemoteEntry, RemoteError> {
            self.answer()?;
            Err(RemoteError::NotFound)
        }
        fn open(&self, _: &str) -> Result<Box<dyn player_remote::fs::RemoteFile>, RemoteError> {
            self.answer()?;
            Err(RemoteError::NotFound)
        }
    }

    fn connected(
        connections: &RemoteConnections,
        account: &RemoteAccount,
        switch: &Arc<Switch>,
    ) -> Arc<dyn RemoteFs> {
        let switch = Arc::clone(switch);
        connections
            .connect_with(account, move || Ok(Arc::new(SwitchFs(switch)) as Arc<dyn RemoteFs>))
            .unwrap()
    }

    #[test]
    fn a_network_error_of_an_operation_starts_the_backoff_for_every_later_use() {
        let connections = RemoteConnections::default();
        let account = account("cached", "192.0.2.1");
        let switch = Arc::new(Switch { up: AtomicBool::new(true), calls: AtomicUsize::new(0) });
        let fs = connected(&connections, &account, &switch);
        assert!(fs.list("").is_ok());
        let before = switch.calls.load(Ordering::SeqCst);

        // The server goes away: the first operation fails on the network and is the last one that asks the server.
        switch.up.store(false, Ordering::SeqCst);
        assert!(matches!(fs.list(""), Err(RemoteError::Network(_))));
        let after_first = switch.calls.load(Ordering::SeqCst);
        assert_eq!(after_first, before + 1);

        // Every kind of use now fails at once with the same error, on the connection already held and on a new request
        // for the account, without asking the server.
        assert!(matches!(fs.stat("a"), Err(RemoteError::Network(m)) if m == "No route to host"));
        assert!(matches!(fs.open("a"), Err(RemoteError::Network(_))));
        let again = connections.connect_with(&account, || panic!("no new connection while the server is known down"));
        assert!(matches!(again, Err(RemoteError::Network(_))));
        assert_eq!(switch.calls.load(Ordering::SeqCst), after_first);
    }

    #[test]
    fn a_connection_is_replaced_and_the_backoff_cleared_once_it_is_over() {
        let connections = RemoteConnections { backoff: Some(Duration::from_millis(80)), ..Default::default() };
        let account = account("recovering", "192.0.2.2");
        let switch = Arc::new(Switch { up: AtomicBool::new(true), calls: AtomicUsize::new(0) });
        let fs = connected(&connections, &account, &switch);
        switch.up.store(false, Ordering::SeqCst);
        assert!(fs.list("").is_err());
        assert!(fs.list("").is_err());

        std::thread::sleep(Duration::from_millis(120));
        // The server is back: the backoff is over, a new connection is made, and its first success clears the mark.
        switch.up.store(true, Ordering::SeqCst);
        let fresh = connected(&connections, &account, &switch);
        assert!(fresh.list("").is_ok());
        assert!(fs.list("").is_ok(), "the old handle works again too");

        // A failure of the new connection starts the backoff again.
        switch.up.store(false, Ordering::SeqCst);
        assert!(fresh.list("").is_err());
        let calls = switch.calls.load(Ordering::SeqCst);
        assert!(fresh.list("").is_err());
        assert_eq!(switch.calls.load(Ordering::SeqCst), calls);
    }

    #[test]
    fn errors_that_are_not_the_network_do_not_start_the_backoff() {
        let connections = RemoteConnections::default();
        let account = account("plain", "192.0.2.3");
        let switch = Arc::new(Switch { up: AtomicBool::new(true), calls: AtomicUsize::new(0) });
        let fs = connected(&connections, &account, &switch);
        assert!(matches!(fs.stat("missing"), Err(RemoteError::NotFound)));
        assert!(matches!(fs.stat("missing"), Err(RemoteError::NotFound)));
        assert_eq!(switch.calls.load(Ordering::SeqCst), 2);
    }

    /// The real protocol clients against a port nothing listens on (as for a host that is switched off): the failure
    /// is a network error, so the first use pays for one attempt and the backoff answers every later one at once.
    #[test]
    fn a_closed_port_is_a_network_error_that_trips_the_guard_for_smb_and_webdav() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let data_dir = std::env::temp_dir().join(format!("moe-closed-port-{}", std::process::id()));
        std::fs::create_dir_all(&data_dir).unwrap();
        for protocol in [RemoteProtocol::Smb, RemoteProtocol::Webdav] {
            let mut down = account("closed", "127.0.0.1");
            down.protocol = protocol;
            down.port = Some(port);
            let connections = RemoteConnections::default();
            let attempt = |connections: &RemoteConnections| {
                connections
                    .connect_with(&down, || super::connect_account(&data_dir, &down))
                    .and_then(|fs| fs.list("").map(|_| ()))
            };
            let first = attempt(&connections);
            assert!(matches!(first, Err(RemoteError::Network(_))), "{protocol:?}: {first:?}");
            let started = Instant::now();
            let second = connections.connect_with(&down, || panic!("must not connect again"));
            assert!(matches!(second, Err(RemoteError::Network(_))), "{protocol:?}");
            assert!(started.elapsed() < Duration::from_millis(50), "{protocol:?}: {:?}", started.elapsed());
        }
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}

/// Cache-first playback: a song that is whole in the disk cache plays without its server (ADR 0026).
#[cfg(test)]
mod offline_cache_tests {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use player_core::{MediaSourceKind, RemoteContent, SourceId};
    use player_remote::account::{RemoteAccount, RemoteProtocol};
    use player_remote::cache::{cache_key, DiskCache, FileMeta, CACHE_CHUNK};
    use player_remote::fs::{RemoteEntry, RemoteError, RemoteFile, RemoteFs};

    use super::{RemoteConnections, RemoteFiles};
    use crate::settings::{AppSettings, SettingsStore, SourceEntry, SourceEntryKind, StoredPath};

    const LEN: usize = 2 * CACHE_CHUNK + 1_234;

    fn bytes() -> Vec<u8> {
        (0..LEN).map(|index| (index % 251) as u8).collect()
    }

    /// What the fake server is doing, and how often it was really asked.
    struct Server {
        up: AtomicBool,
        /// Operations that reached the server (connect attempts, stat, open, reads).
        calls: AtomicUsize,
        /// How long a stat takes.
        stat_delay: Mutex<Duration>,
        /// The file as the server has it now.
        modified_ms: Mutex<i64>,
    }

    impl Server {
        fn new(up: bool) -> Arc<Self> {
            Arc::new(Self {
                up: AtomicBool::new(up),
                calls: AtomicUsize::new(0),
                stat_delay: Mutex::new(Duration::ZERO),
                modified_ms: Mutex::new(1_000),
            })
        }

        fn ask(&self) -> Result<(), RemoteError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.up.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(RemoteError::Network("No route to host".to_owned()))
            }
        }
    }

    struct FakeFs(Arc<Server>);
    struct FakeFile(Arc<Server>, Vec<u8>);

    impl RemoteFile for FakeFile {
        fn len(&self) -> u64 {
            self.1.len() as u64
        }

        fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
            self.0.ask()?;
            let start = (offset as usize).min(self.1.len());
            let count = (self.1.len() - start).min(buf.len());
            buf[..count].copy_from_slice(&self.1[start..start + count]);
            Ok(count)
        }
    }

    impl RemoteFs for FakeFs {
        fn list(&self, _: &str) -> Result<Vec<RemoteEntry>, RemoteError> {
            self.0.ask().map(|()| Vec::new())
        }

        fn stat(&self, _: &str) -> Result<RemoteEntry, RemoteError> {
            self.0.ask()?;
            std::thread::sleep(*self.0.stat_delay.lock().unwrap());
            Ok(RemoteEntry {
                name: "a.flac".to_owned(),
                is_dir: false,
                size: LEN as u64,
                modified_ms: Some(*self.0.modified_ms.lock().unwrap()),
                created_utc_100ns: None,
            })
        }

        fn open(&self, _: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
            self.0.ask()?;
            Ok(Box::new(FakeFile(Arc::clone(&self.0), bytes())))
        }
    }

    struct Fixture {
        files: RemoteFiles,
        source: SourceId,
        cache: Arc<DiskCache>,
        dir: std::path::PathBuf,
    }

    /// The account key and full path the cache knows the song by.
    fn names(source: SourceId) -> (String, String) {
        (source.to_string(), "Music/a.flac".to_owned())
    }

    /// A player over `dir` whose one remote source is served by `server`; `connect_delay` is how long making a connection
    /// takes. `dir` keeps the disk cache and the settings between fixtures, as a restart of the player would.
    fn fixture(dir: &std::path::Path, server: &Arc<Server>, connect_delay: Duration) -> Fixture {
        std::fs::create_dir_all(dir).unwrap();
        let settings =
            Arc::new(SettingsStore::open(dir.join("settings.json"), AppSettings::default()).unwrap());
        settings
            .update(|settings| {
                if settings.remote_accounts.is_empty() {
                    settings.remote_accounts.push(RemoteAccount {
                        id: "acct".to_owned(),
                        name: "server".to_owned(),
                        protocol: RemoteProtocol::Smb,
                        host: "192.0.2.9".to_owned(),
                        port: None,
                        path: "share".to_owned(),
                        username: String::new(),
                        password: None,
                        tls: Default::default(),
                        only_on_wifi: false,
                    });
                }
                Ok(())
            })
            .unwrap();
        let existing = settings.snapshot().unwrap().sources.first().map(|source| source.id);
        let source = existing.unwrap_or_else(|| {
            let id = SourceId::new();
            let locator = player_core::MediaLocator::Remote { source_id: id, path: "Music".to_owned() };
            settings
                .upsert_source(SourceEntry {
                    id,
                    display_name: "remote".to_owned(),
                    enabled: true,
                    kind: SourceEntryKind::Folder {
                        media_kind: MediaSourceKind::Remote,
                        path: StoredPath::from_locator(&locator).unwrap(),
                        remote_account: Some("acct".to_owned()),
                    },
                })
                .unwrap();
            id
        });
        let for_maker = Arc::clone(server);
        let connections = Arc::new(RemoteConnections {
            maker: Some(Arc::new(move |_| {
                std::thread::sleep(connect_delay);
                for_maker.ask()?;
                Ok(Arc::new(FakeFs(Arc::clone(&for_maker))) as Arc<dyn RemoteFs>)
            })),
            ..Default::default()
        });
        let cache = Arc::new(DiskCache::open(dir.join("remote-cache"), 500_000_000).unwrap());
        Fixture {
            files: RemoteFiles {
                connections,
                settings,
                cache: Some(Arc::clone(&cache)),
                streams: Default::default(),
            },
            source,
            cache,
            dir: dir.to_path_buf(),
        }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("moe-offline-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// Play the whole song once with the server on, so it is cached.
    fn play_once(fixture: &Fixture) {
        let mut handle = fixture.files.open_streaming(fixture.source, "a.flac").unwrap();
        let mut all = Vec::new();
        handle.reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes());
    }

    #[test]
    fn a_song_that_is_whole_in_the_cache_plays_with_the_server_off_and_never_waits_for_a_connection() {
        let dir = temp("whole");
        let online = Server::new(true);
        let first = fixture(&dir, &online, Duration::ZERO);
        assert!(!first.files.available_offline(first.source, "a.flac"));
        play_once(&first);
        assert!(first.files.available_offline(first.source, "a.flac"));
        let source = first.source;
        drop(first);

        // The player is started again with the server off, and connecting would take seconds if it were tried.
        let off = Server::new(false);
        let second = fixture(&dir, &off, Duration::from_secs(3));
        assert_eq!(second.source, source);
        assert!(second.files.available_offline(second.source, "a.flac"));
        let started = Instant::now();
        let mut handle = second.files.open_streaming(second.source, "a.flac").unwrap();
        assert!(started.elapsed() < Duration::from_millis(900), "{:?}", started.elapsed());
        assert_eq!(handle.len, LEN as u64);
        let mut all = Vec::new();
        handle.reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes(), "the cached bytes are what plays");
        // A tag or cover read of the same song needs no server either.
        let mut small = second.files.open(second.source, "a.flac").unwrap();
        let mut head = [0_u8; 16];
        small.reader.read_exact(&mut head).unwrap();
        assert_eq!(&head[..], &bytes()[..16]);
        // Nothing but the background check (one refused connection attempt) asked the off server.
        std::thread::sleep(Duration::from_millis(100));
        assert!(off.calls.load(Ordering::SeqCst) <= 1, "{}", off.calls.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&second.dir);
    }

    #[test]
    fn a_lyrics_sidecar_read_once_is_kept_in_the_disk_cache_and_read_with_the_server_off() {
        let dir = temp("sidecar");
        let online = Server::new(true);
        let first = fixture(&dir, &online, Duration::ZERO);
        let mut handle = first.files.open_cached(first.source, "a.lrc").unwrap();
        let mut all = Vec::new();
        handle.reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes());
        assert!(first.files.available_offline(first.source, "a.lrc"));
        drop(first);

        let off = Server::new(false);
        let second = fixture(&dir, &off, Duration::from_secs(3));
        let started = Instant::now();
        let mut handle = second.files.open_cached(second.source, "a.lrc").unwrap();
        let mut all = Vec::new();
        handle.reader.read_to_end(&mut all).unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(900),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(all, bytes(), "the sidecar comes from the disk cache");
        let _ = std::fs::remove_dir_all(&second.dir);
    }

    #[test]
    fn a_song_missing_one_chunk_needs_the_server_and_fails_as_unreachable_when_it_is_off() {
        let dir = temp("partial");
        let online = Server::new(true);
        let first = fixture(&dir, &online, Duration::ZERO);
        play_once(&first);
        let (account_key, full) = names(first.source);
        let meta = first.cache.meta(&account_key, &full).unwrap();
        assert_eq!(meta, FileMeta { len: LEN as u64, modified_ms: Some(1_000) });
        let key = cache_key(&account_key, &full, meta.len, meta.modified_ms);
        let source = first.source;
        drop(first);

        // One chunk is gone (pushed out by other songs, say): the cache keeps the rest.
        let cache = DiskCache::open(dir.join("remote-cache"), 500_000_000).unwrap();
        let (one, two) = (cache.get(&key, 1).unwrap(), cache.get(&key, 2).unwrap());
        cache.clear();
        cache.put_meta(&account_key, &full, meta);
        cache.put(&key, 1, &one);
        cache.put(&key, 2, &two);
        drop(cache);

        let off = Server::new(false);
        let second = fixture(&dir, &off, Duration::ZERO);
        assert_eq!(second.source, source);
        assert!(!second.files.available_offline(second.source, "a.flac"));
        let error = second
            .files
            .open_streaming(second.source, "a.flac")
            .err()
            .expect("a partial cache is not played offline");
        assert_eq!(error.kind(), std::io::ErrorKind::ConnectionAborted);
        assert!(player_core::is_remote_unreachable(&error.to_string()), "{error}");

        // With the server on it streams again, fetching what is missing, and the song is whole afterwards.
        off.up.store(true, Ordering::SeqCst);
        second.files.connections.forget("acct");
        let mut handle = second.files.open_streaming(second.source, "a.flac").unwrap();
        let mut all = Vec::new();
        handle.reader.read_to_end(&mut all).unwrap();
        assert_eq!(all, bytes());
        assert!(second.files.available_offline(second.source, "a.flac"));
        let _ = std::fs::remove_dir_all(&second.dir);
    }

    #[test]
    fn a_change_on_the_server_is_noticed_in_the_background_and_used_by_the_next_play_only() {
        let dir = temp("changed");
        let online = Server::new(true);
        let first = fixture(&dir, &online, Duration::ZERO);
        play_once(&first);
        let (account_key, full) = names(first.source);
        // The song was edited on the server, and its answers are slow.
        *online.modified_ms.lock().unwrap() = 2_000;
        *online.stat_delay.lock().unwrap() = Duration::from_millis(600);
        let started = Instant::now();
        let mut handle = first.files.open_streaming(first.source, "a.flac").unwrap();
        assert!(started.elapsed() < Duration::from_millis(400), "the check blocked the open: {:?}", started.elapsed());
        // This play still reads the bytes it opened, while the check is running and after it has finished.
        let mut head = vec![0_u8; 1000];
        handle.reader.read_exact(&mut head).unwrap();
        assert_eq!(head, bytes()[..1000]);
        assert_eq!(first.cache.meta(&account_key, &full).unwrap().modified_ms, Some(1_000));
        for _ in 0..100 {
            if first.cache.meta(&account_key, &full).unwrap().modified_ms == Some(2_000) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(first.cache.meta(&account_key, &full).unwrap().modified_ms, Some(2_000));
        let mut rest = Vec::new();
        handle.reader.read_to_end(&mut rest).unwrap();
        assert_eq!([head, rest].concat(), bytes(), "the playing session kept its cached bytes");
        // The next play uses the new version: it is not cached yet, so it is not available offline.
        assert!(!first.files.available_offline(first.source, "a.flac"));
        let _ = std::fs::remove_dir_all(&first.dir);
    }

    #[test]
    fn the_check_of_a_song_in_the_cache_is_silent_when_the_server_is_off_and_never_stacks() {
        let dir = temp("silent");
        let online = Server::new(true);
        let first = fixture(&dir, &online, Duration::ZERO);
        play_once(&first);
        drop(first);
        let off = Server::new(false);
        let second = fixture(&dir, &off, Duration::from_millis(300));
        // Many opens in a row: every one works, and the off server is asked about connecting only a few times.
        for _ in 0..5 {
            let mut handle = second.files.open_streaming(second.source, "a.flac").unwrap();
            let mut head = [0_u8; 8];
            handle.reader.read_exact(&mut head).unwrap();
            assert_eq!(&head[..], &bytes()[..8]);
        }
        std::thread::sleep(Duration::from_millis(900));
        assert!(off.calls.load(Ordering::SeqCst) <= 2, "{}", off.calls.load(Ordering::SeqCst));
        let _ = std::fs::remove_dir_all(&second.dir);
    }
}
