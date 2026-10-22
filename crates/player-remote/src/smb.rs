//! The SMB client (ADR 0026): SMB 2/3 through the pure-Rust `smb` crate (signing and encryption when the server offers
//! them), behind the same [`RemoteFs`] trait as WebDAV. SMB 1 is not supported.
//!
//! One [`SmbFs`] is one account on one share. The connection is made when the value is created and made again, once,
//! when an operation finds it broken (a server that was restarted, a link that dropped). The password lives only inside
//! this value (wiped when dropped) and is never put in an error.

use std::sync::{Mutex, PoisonError};

use smb::{
    Client, ClientConfig, ConnectionConfig, CreateOptions, DirAccessMask, FileAccessMask,
    FileAttributes, FileBasicInformation, FileCreateArgs, FileDispositionInformation,
    FileFullDirectoryInformation, FileRenameInformation, FileStandardInformation, ReadAt,
    Resource, UncPath, WriteAt,
};
use zeroize::Zeroizing;

use crate::fs::{
    replacement_temp_path, FileStamp, RangeReader, RemoteEntry, RemoteError, RemoteFile, RemoteFs,
    WRITE_CHUNK,
};

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
    /// The file was deleted a moment ago and its last handle has not closed yet: for a reader it is already gone.
    pub const DELETE_PENDING: u32 = 0xC000_0056;
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
            | status::DELETE_PENDING
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

/// The shares a server offers to this account (administrative shares such as `C$` and `IPC$` are left out), so the
/// user can pick one instead of typing its name.
pub fn list_shares(
    host: &str,
    port: Option<u16>,
    user: &str,
    password: &str,
) -> Result<Vec<String>, RemoteError> {
    list_shares_with_guest_access(host, port, user, password, false)
}

/// List shares with an explicit opt-in for servers that map a guest or null SMB session.
///
/// Guest sessions may be unsigned, so this remains disabled by default. Enabling it only permits the SMB client's
/// guest-session handling when the server identifies the session as guest; it does not disable signing for an
/// authenticated session.
pub fn list_shares_with_guest_access(
    host: &str,
    port: Option<u16>,
    user: &str,
    password: &str,
    allow_unsigned_guest_access: bool,
) -> Result<Vec<String>, RemoteError> {
    let client = Client::new(ClientConfig {
        connection: ConnectionConfig {
            port,
            // A phone on a mobile link reading a large tag (a DSF with a big cover) can wait longer than the 10 s default.
            timeout: Some(std::time::Duration::from_secs(20)),
            allow_unsigned_guest_access,
            ..Default::default()
        },
        ..Default::default()
    });
    let ipc = UncPath::ipc_share(host).map_err(|error| RemoteError::Protocol(error.to_string()))?;
    client
        .share_connect(&ipc, user, password.to_owned())
        .map_err(|error| classify(&error))?;
    let shares = client.list_shares(host).map_err(|error| classify(&error))?;
    let _ = client.close();
    let mut names: Vec<String> = shares
        .iter()
        .filter_map(|share| share.netname.as_ref().map(|name| name.to_string()))
        .map(|name| {
            name.trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_owned()
        })
        .filter(|name| !name.is_empty() && !name.ends_with('$'))
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    Ok(names)
}

/// How long a request that asks the server something (list, stat, open) may take before it is given up.
const OPERATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
/// The longest one read of file data (a megabyte) may take while data keeps arriving; a link on which nothing arrives
/// is given up much sooner (see [`STALL_DETECT`]). 1 MiB at 0.1 Mbit/s is about 84 s.
const READ_LIMIT: std::time::Duration = std::time::Duration::from_secs(120);
/// While a request waits, this long without a single byte from the server makes the client ask whether it is still
/// there (a small request standing in for an SMB2 ECHO, see [`Link::ping`]).
pub(crate) const STALL_DETECT: std::time::Duration = std::time::Duration::from_secs(1);
/// How long the server then has to send anything (the echo's answer or the data) before the link counts as dead and is
/// cut, failing every request on it so the caller connects again.
pub(crate) const ECHO_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
/// How often a waiting request looks at the link.
const WATCH_TICK: std::time::Duration = std::time::Duration::from_millis(100);
/// How long one write of file data (up to [`WRITE_CHUNK`]) may take.
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// How long connecting (negotiating and signing in) may take.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Run `work` on its own thread and wait at most `timeout` for it. The `smb` crate's own timeout does not always fire (a
/// connection that was dropped silently can leave a call waiting for good, and with it everything queued behind it), so
/// every call to the server is bounded from outside: past the time the call is abandoned (its thread ends whenever the
/// call does) and reported as a network failure, which the caller answers by connecting again.
fn bounded<T: Send + 'static>(
    timeout: std::time::Duration,
    work: impl FnOnce() -> Result<T, RemoteError> + Send + 'static,
) -> Result<T, RemoteError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("moe-smb-call".to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        });
    if spawned.is_err() {
        return Err(RemoteError::Network("could not start a worker for the request".to_owned()));
    }
    match receiver.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err(RemoteError::Network(format!(
            "the server did not answer within {} s",
            timeout.as_secs()
        ))),
    }
}

/// What a waiting request should do about its link, decided from when the last byte arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Watch {
    Wait,
    /// Nothing arrived for [`STALL_DETECT`]: ask the server (see [`Link::ping`]).
    SendEcho,
    /// Nothing arrived for [`ECHO_WAIT`] after asking either: the link is dead.
    Dead,
}

/// Tells a slow link (bytes keep coming, however few) from a dead one (none at all) for one request.
pub(crate) struct Watchdog {
    started: std::time::Instant,
    echo_sent: Option<std::time::Instant>,
    stall: std::time::Duration,
    echo_wait: std::time::Duration,
}

impl Watchdog {
    pub(crate) fn new(started: std::time::Instant, stall: std::time::Duration, echo_wait: std::time::Duration) -> Self {
        Self {
            started,
            echo_sent: None,
            stall,
            echo_wait,
        }
    }

    /// `last_received`: when the link's last byte arrived (any request's, the echo's answer included).
    pub(crate) fn check(
        &mut self,
        now: std::time::Instant,
        last_received: Option<std::time::Instant>,
    ) -> Watch {
        // Bytes from before this request do not count: the link may have been idle for minutes.
        let quiet_since = last_received.map_or(self.started, |at| at.max(self.started));
        if let Some(sent) = self.echo_sent {
            if quiet_since > sent {
                // Something arrived after asking: alive. Start over.
                self.echo_sent = None;
            } else if now.saturating_duration_since(sent) >= self.echo_wait {
                return Watch::Dead;
            } else {
                return Watch::Wait;
            }
        }
        if now.saturating_duration_since(quiet_since) >= self.stall {
            self.echo_sent = Some(now);
            return Watch::SendEcho;
        }
        Watch::Wait
    }
}

/// Run `work` on its own thread against `link` and wait for it as long as the link is alive: data arriving (however
/// slowly) keeps the wait going up to `limit`; nothing at all for [`STALL_DETECT`] asks the server ([`Link::ping`]), and nothing for
/// [`ECHO_WAIT`] after that cuts the link ([`LinkHealth::kill`](crate::smb_transport::LinkHealth::kill)), which fails
/// every request on it, and reports a network failure (the caller connects again).
fn bounded_on<T: Send + 'static>(
    link: &std::sync::Arc<Link>,
    limit: std::time::Duration,
    work: impl FnOnce() -> Result<T, RemoteError> + Send + 'static,
) -> Result<T, RemoteError> {
    let (sender, receiver) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("moe-smb-call".to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        });
    if spawned.is_err() {
        return Err(RemoteError::Network("could not start a worker for the request".to_owned()));
    }
    let started = std::time::Instant::now();
    let mut watchdog = Watchdog::new(started, STALL_DETECT, ECHO_WAIT);
    loop {
        match receiver.recv_timeout(WATCH_TICK) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(RemoteError::Network("the request's worker ended without an answer".to_owned()))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        let now = std::time::Instant::now();
        if now.saturating_duration_since(started) >= limit {
            return Err(RemoteError::Network(format!(
                "the server did not finish within {} s",
                limit.as_secs()
            )));
        }
        match watchdog.check(now, link.health.last_received()) {
            Watch::Wait => {}
            Watch::SendEcho => link.ping(),
            Watch::Dead => {
                eprintln!(
                    "[remote] smb link dead: no data for {} ms (echo unanswered); reconnecting",
                    now.saturating_duration_since(
                        link.health.last_received().map_or(started, |at| at.max(started))
                    )
                    .as_millis()
                );
                link.health.kill();
                return Err(RemoteError::Network("the server stopped answering".to_owned()));
            }
        }
    }
}

/// One signed-in connection to one share: the connection (over [`NoDelayTransport`]), its session and the tree.
/// `smb::Client` would do the same but always builds its own transport, which stalls every request (see
/// [`crate::smb_transport`]); the calls this file makes need only `create_file`.
pub struct Link {
    connection: std::sync::Arc<smb::Connection>,
    session: smb::Session,
    tree: smb::Tree,
    health: std::sync::Arc<crate::smb_transport::LinkHealth>,
    /// An echo is on its way (one at a time per link).
    echo_in_flight: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Link {
    fn connect(settings: &SmbSettings) -> Result<Self, smb::Error> {
        use std::net::ToSocketAddrs;
        let timeout = std::time::Duration::from_secs(20);
        let port = settings.port.unwrap_or(crate::smb_transport::NoDelayTransport::DEFAULT_PORT);
        let address = (settings.host.as_str(), port)
            .to_socket_addrs()
            .map_err(|error| smb::Error::InvalidArgument(error.to_string()))?
            .next()
            .ok_or_else(|| smb::Error::InvalidArgument("the server name has no address".to_owned()))?;
        let health = crate::smb_transport::LinkHealth::new();
        let mut transport =
            crate::smb_transport::NoDelayTransport::with_health(CONNECT_TIMEOUT, std::sync::Arc::clone(&health));
        smb::transport::SmbTransport::connect(&mut transport, &settings.host, address)?;
        let connection = smb::Connection::from_transport(
            Box::new(transport),
            &settings.host,
            smb::Guid::generate(),
            ConnectionConfig {
                port: settings.port,
                // A phone on a mobile link reading a large tag (a DSF with a big cover) can wait longer than the 10 s default.
                timeout: Some(timeout),
                ..Default::default()
            },
        )?;
        let identity = sspi::AuthIdentity {
            username: sspi::Username::parse(&settings.user)
                .map_err(|error| smb::Error::SspiError(error.into()))?,
            password: sspi::Secret::from(settings.password.to_string()),
        };
        let session = connection.authenticate(identity)?;
        let tree = session.tree_connect(&SmbFs::share_path(settings).map_err(|error| {
            smb::Error::InvalidArgument(error.to_string())
        })?)?;
        Ok(Self {
            connection: std::sync::Arc::new(connection),
            session,
            tree,
            health,
            echo_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    /// Ask the server something small on its own thread, as an SMB2 ECHO would (the `smb` crate 0.12 keeps its
    /// message handlers private, so an ECHO cannot be sent through it, and a hand-made one would bypass its message ids,
    /// credits and signing): open the share's root for its attributes and close it. Its answer is not looked at: any
    /// byte it brings back counts as the link being alive (see [`bounded_on`]).
    fn ping(self: &std::sync::Arc<Self>) {
        use std::sync::atomic::Ordering;
        if self.echo_in_flight.swap(true, Ordering::SeqCst) {
            return;
        }
        let link = std::sync::Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("moe-smb-echo".to_owned())
            .spawn(move || {
                let args = FileCreateArgs::make_open_existing(
                    FileAccessMask::new().with_file_read_attributes(true),
                );
                match link.tree.create("", &args) {
                    Ok(Resource::File(file)) => {
                        let _ = file.close();
                    }
                    Ok(Resource::Directory(dir)) => {
                        let _ = dir.close();
                    }
                    _ => {}
                }
                link.echo_in_flight.store(false, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.echo_in_flight.store(false, Ordering::SeqCst);
        }
    }

    fn create_file(&self, path: &UncPath, args: &FileCreateArgs) -> Result<Resource, smb::Error> {
        self.tree.create(path.path().unwrap_or(""), args)
    }

    fn close(&self) {
        let _ = self.tree.disconnect();
        let _ = self.session.logoff();
        let _ = self.connection.close();
    }
}

pub struct SmbFs {
    settings: SmbSettings,
    client: Mutex<std::sync::Arc<Link>>,
}

impl SmbFs {
    /// Connect and sign in. Fails with [`RemoteError::Auth`] for a wrong account, name or password.
    pub fn connect(settings: SmbSettings) -> Result<Self, RemoteError> {
        let client = Self::new_client_bounded(&settings)?;
        Ok(Self {
            settings,
            client: Mutex::new(std::sync::Arc::new(client)),
        })
    }

    fn new_client(settings: &SmbSettings) -> Result<Link, RemoteError> {
        Link::connect(settings).map_err(|error| classify(&error))
    }

    /// Connect, giving up after [`CONNECT_TIMEOUT`].
    fn new_client_bounded(settings: &SmbSettings) -> Result<Link, RemoteError> {
        let settings = settings.clone();
        bounded(CONNECT_TIMEOUT, move || Self::new_client(&settings))
    }

    fn share_path(settings: &SmbSettings) -> Result<UncPath, RemoteError> {
        UncPath::new(&settings.host)
            .and_then(|path| path.with_share(&settings.share))
            .map_err(|error| RemoteError::Protocol(error.to_string()))
    }

    fn client(&self) -> std::sync::Arc<Link> {
        self.client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Run `operation` against the server, at most [`OPERATION_TIMEOUT`]; when it fails because the link is gone (or
    /// did not answer), connect again (the lock is not held while connecting) and try once more.
    fn with_retry<T: Send + 'static>(
        &self,
        operation: impl Fn(&Link) -> Result<T, smb::Error> + Send + Sync + 'static,
    ) -> Result<T, RemoteError> {
        let operation = std::sync::Arc::new(operation);
        let attempt = |client: std::sync::Arc<Link>| {
            let operation = std::sync::Arc::clone(&operation);
            let worker_client = std::sync::Arc::clone(&client);
            bounded_on(&client, OPERATION_TIMEOUT, move || {
                operation(&worker_client).map_err(|error| classify(&error))
            })
        };
        let current = self.client();
        // A link already found dead is not tried again.
        let first = if current.health.is_killed() {
            Err(RemoteError::Network("the link was cut".to_owned()))
        } else {
            attempt(current)
        };
        match first {
            Err(RemoteError::Network(_)) => {
                let fresh = std::sync::Arc::new(Self::new_client_bounded(&self.settings)?);
                *self.client.lock().unwrap_or_else(PoisonError::into_inner) =
                    std::sync::Arc::clone(&fresh);
                attempt(fresh)
            }
            other => other,
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
        crate::fs::open_reader(self, path)
    }
}

impl Drop for SmbFs {
    fn drop(&mut self) {
        if let Ok(client) = self.client.get_mut() {
            client.close();
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
        self.with_retry(move |client| {
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
        self.with_retry(move |client| {
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

    fn read_ahead_blocks(&self) -> u64 {
        SMB_READ_AHEAD_BLOCKS
    }

    fn replace_file(
        &self,
        path: &str,
        source: &mut dyn std::io::Read,
        len: u64,
        expected: Option<FileStamp>,
    ) -> Result<RemoteEntry, RemoteError> {
        if let Some(expected) = expected {
            let now = self.stat(path)?;
            if FileStamp::of(&now) != expected {
                return Err(RemoteError::Changed);
            }
        }
        let temp_unc = self.unc(&replacement_temp_path(path))?;
        // The name the temporary file is renamed to, the way the protocol wants it: from the root of the share.
        let target = windows_path(&[&self.settings.base_path, path])?;
        let created = self.with_retry(move |client| {
            let args = FileCreateArgs::make_overwrite(FileAttributes::new(), CreateOptions::new());
            match client.create_file(&temp_unc, &args)? {
                Resource::File(file) => Ok(file),
                Resource::Directory(dir) => {
                    let _ = dir.close();
                    Err(smb::Error::NotFound("a folder is in the way".to_owned()))
                }
                Resource::Pipe(_) => Err(smb::Error::NotFound("not a file".to_owned())),
            }
        })?;
        let file = std::sync::Arc::new(Mutex::new(created));
        let outcome = (|| -> Result<(), RemoteError> {
            let mut buffer = vec![0_u8; WRITE_CHUNK];
            let mut offset = 0_u64;
            while offset < len {
                let wanted = (len - offset).min(WRITE_CHUNK as u64) as usize;
                source
                    .read_exact(&mut buffer[..wanted])
                    .map_err(|error| RemoteError::Protocol(format!("could not read the file to send: {error}")))?;
                let chunk = buffer[..wanted].to_vec();
                let handle = std::sync::Arc::clone(&file);
                let start = offset;
                bounded(WRITE_TIMEOUT, move || {
                    let handle = handle.lock().unwrap_or_else(PoisonError::into_inner);
                    let mut done = 0;
                    while done < chunk.len() {
                        let written = handle
                            .write_at(&chunk[done..], start + done as u64)
                            .map_err(|error| classify(&error))?;
                        if written == 0 {
                            return Err(RemoteError::Protocol("the server stopped accepting data".to_owned()));
                        }
                        done += written;
                    }
                    Ok(())
                })?;
                offset += wanted as u64;
            }
            // All of it must have arrived before it takes the place of the original.
            let handle = std::sync::Arc::clone(&file);
            let size = bounded(OPERATION_TIMEOUT, move || {
                let handle = handle.lock().unwrap_or_else(PoisonError::into_inner);
                handle
                    .query_info::<FileStandardInformation>()
                    .map(|info| info.end_of_file)
                    .map_err(|error| classify(&error))
            })?;
            if size != len {
                return Err(RemoteError::Protocol(format!("the server holds {size} of {len} bytes")));
            }
            let handle = std::sync::Arc::clone(&file);
            bounded(OPERATION_TIMEOUT, move || {
                let handle = handle.lock().unwrap_or_else(PoisonError::into_inner);
                handle
                    .set_info(FileRenameInformation {
                        replace_if_exists: true.into(),
                        root_directory: 0,
                        file_name: target.as_str().into(),
                    })
                    .map_err(|error| classify(&error))
            })
        })();
        if outcome.is_err() {
            // The temporary file does not stay behind: marked for deletion, it goes when its handle closes.
            let handle = std::sync::Arc::clone(&file);
            let _ = bounded(OPERATION_TIMEOUT, move || {
                let handle = handle.lock().unwrap_or_else(PoisonError::into_inner);
                handle
                    .set_info(FileDispositionInformation::default())
                    .map_err(|error| classify(&error))
            });
        }
        if let Ok(handle) = file.try_lock() {
            let _ = handle.close();
        }
        outcome?;
        self.stat(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), RemoteError> {
        let target = self.unc(path)?;
        self.with_retry(move |client| {
            let args = FileCreateArgs::make_open_existing(FileAccessMask::new().with_delete(true));
            match client.create_file(&target, &args)? {
                Resource::File(file) => {
                    let result = file.set_info(FileDispositionInformation::default());
                    let _ = file.close();
                    result
                }
                Resource::Directory(dir) => {
                    let _ = dir.close();
                    Err(smb::Error::NotFound("not a file".to_owned()))
                }
                Resource::Pipe(_) => Err(smb::Error::NotFound("not a file".to_owned())),
            }
        })
    }

    fn rename_file(&self, from: &str, to: &str) -> Result<(), RemoteError> {
        let source = self.unc(from)?;
        // The new name, the way the protocol wants it: from the root of the share.
        let target = windows_path(&[&self.settings.base_path, to])?;
        self.with_retry(move |client| {
            let args = FileCreateArgs::make_open_existing(FileAccessMask::new().with_delete(true));
            match client.create_file(&source, &args)? {
                Resource::File(file) => {
                    let result = file.set_info(FileRenameInformation {
                        replace_if_exists: false.into(),
                        root_directory: 0,
                        file_name: target.as_str().into(),
                    });
                    let _ = file.close();
                    result
                }
                Resource::Directory(dir) => {
                    let _ = dir.close();
                    Err(smb::Error::NotFound("not a file".to_owned()))
                }
                Resource::Pipe(_) => Err(smb::Error::NotFound("not a file".to_owned())),
            }
        })
    }

    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        let target = self.unc(path)?;
        let missing = path.to_owned();
        let (file, len) = self.with_retry(move |client| {
            let resource = client.create_file(&target, &open_file_args())?;
            let Resource::File(file) = resource else {
                if let Resource::Directory(dir) = resource {
                    let _ = dir.close();
                }
                return Err(smb::Error::NotFound(missing.clone()));
            };
            let len = file.query_info::<FileStandardInformation>()?.end_of_file;
            Ok((file, len))
        })?;
        Ok(Box::new(SmbFile {
            file: std::sync::Arc::new(Mutex::new(file)),
            len,
            link: self.client(),
        }))
    }
}

/// An open SMB file. Reading is `read_at`, which the `smb` crate splits into as many requests as the negotiated
/// maximum read size needs.
pub struct SmbFile {
    /// Shared with the worker thread of the read in flight (see [`bounded`]).
    file: std::sync::Arc<Mutex<smb::File>>,
    len: u64,
    /// The connection the file was opened on, watched while a read waits.
    link: std::sync::Arc<Link>,
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
        let file = std::sync::Arc::clone(&self.file);
        let data = bounded_on(&self.link, READ_LIMIT, move || {
            let file = file.lock().unwrap_or_else(PoisonError::into_inner);
            let mut data = vec![0_u8; wanted];
            let mut done = 0;
            while done < wanted {
                let read = file
                    .read_at(&mut data[done..wanted], offset + done as u64)
                    .map_err(|error| classify(&error))?;
                if read == 0 {
                    break;
                }
                done += read;
            }
            data.truncate(done);
            Ok(data)
        })?;
        buf[..data.len()].copy_from_slice(&data);
        Ok(data.len())
    }
}

impl Drop for SmbFile {
    fn drop(&mut self) {
        // A read that never returned may still hold the handle; closing then is left to the process.
        if let Ok(file) = self.file.try_lock() {
            let _ = file.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watchdog_asks_after_a_second_of_silence_and_gives_up_a_second_later() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut watchdog = Watchdog::new(start, STALL_DETECT, ECHO_WAIT);
        // Bytes from long before the request do not make it look silent from the start.
        assert_eq!(watchdog.check(at(500), None), Watch::Wait);
        assert_eq!(watchdog.check(at(1000), None), Watch::SendEcho);
        assert_eq!(watchdog.check(at(1500), None), Watch::Wait);
        // An answer (any byte) after asking: alive, the count starts over from that byte.
        assert_eq!(watchdog.check(at(1600), Some(at(1550))), Watch::Wait);
        assert_eq!(watchdog.check(at(2500), Some(at(1550))), Watch::Wait);
        assert_eq!(watchdog.check(at(2550), Some(at(1550))), Watch::SendEcho);
        assert_eq!(watchdog.check(at(3500), Some(at(1550))), Watch::Wait);
        assert_eq!(watchdog.check(at(3550), Some(at(1550))), Watch::Dead);
        // A link idle before the request began starts counting at the request.
        let mut fresh = Watchdog::new(at(10_000), STALL_DETECT, ECHO_WAIT);
        assert_eq!(fresh.check(at(10_500), Some(at(0))), Watch::Wait);
        assert_eq!(STALL_DETECT, Duration::from_secs(1));
        assert_eq!(ECHO_WAIT, Duration::from_secs(1));
    }

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
    fn share_names_lose_the_terminating_nul_and_admin_shares_are_hidden() {
        let clean = |name: &str| {
            name.trim_matches(|c: char| c == '\0' || c.is_whitespace())
                .to_owned()
        };
        assert_eq!(clean("DATA1\0"), "DATA1");
        assert!(clean("C$\0").ends_with('$'));
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
