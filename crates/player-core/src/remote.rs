//! Reading files of remote sources from code that knows nothing about servers (ADR 0026): the audio engine, the cover and
//! lyrics lookups and the tag reader all ask the one [`RemoteContent`] the app installs. A file is named the way a
//! [`MediaLocator::Remote`](crate::MediaLocator::Remote) names it: the source it belongs to and its path under that
//! source's root.

use std::io::{self, Read, Seek};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use crate::SourceId;

/// A stream that can be moved to another thread, read and seeked.
pub trait RemoteStream: Read + Seek + Send {}
impl<T: Read + Seek + Send> RemoteStream for T {}

/// How the message of a read error starts when a remote stream gave up because the server could not be reached for
/// longer than the retries allow (ADR 0026, "link lost"). The players stop on it instead of moving on to the next track.
pub const REMOTE_LINK_LOST: &str = "伺服器無法連線";

/// Whether a read error (its message, possibly wrapped by a converter) means the server could not be reached and the
/// stream gave up.
pub fn is_remote_link_lost(message: &str) -> bool {
    message.contains(REMOTE_LINK_LOST)
}

/// The start of the message of a remote error whose cause is the network (`RemoteError::Network`).
pub const REMOTE_UNREACHABLE: &str = "無法連線到伺服器";

/// Whether an error message says the server could not be reached: either a failed attempt (an operation or a
/// connection) or a stream that gave up after its retries.
pub fn is_remote_unreachable(message: &str) -> bool {
    message.contains(REMOTE_UNREACHABLE) || is_remote_link_lost(message)
}

/// A newer choice of track replaces a load that is still opening a remote file (ADR 0026): a player numbers the loads
/// it is asked for ([`LoadCounter`]); its worker runs them one at a time, and a stream opened while load `n` runs gives
/// up its reads once a load newer than `n` is asked for, only while load `n` is still running (a track that is
/// playing is never cut off by the next choice).
#[derive(Clone, Default)]
pub struct LoadCounter {
    requested: Arc<AtomicU64>,
    finished: Arc<AtomicU64>,
}

impl LoadCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// A load of a track was asked for (on any thread); returns its number.
    pub fn note_requested(&self) -> u64 {
        self.requested.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The player's worker is done with one load that was asked for (it started, failed or was replaced).
    pub fn note_finished(&self) {
        self.finished.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether a load was asked for and has not been dealt with yet. A walk over the queue does not start while the
    /// user's own choice of a track is still on its way.
    pub fn in_flight(&self) -> bool {
        self.finished.load(Ordering::SeqCst) < self.requested.load(Ordering::SeqCst)
    }

    /// How many loads were asked for so far: a walk over the queue compares it before each entry, to stop once the
    /// user has chosen a track of their own.
    pub fn requested(&self) -> u64 {
        self.requested.load(Ordering::SeqCst)
    }
}

/// A read gave up because the load it belonged to was replaced by a newer one.
pub const LOAD_SUPERSEDED: &str = "load superseded";

/// Whether a stream is to give up now (see [`load_cancel`]).
pub type LoadCancel = Arc<dyn Fn() -> bool + Send + Sync>;

struct RunningLoad {
    number: u64,
    requested: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
    /// Set by whoever gives up waiting for the load (a deadline, a stop): its reads end like a replaced load's.
    abandoned: Arc<AtomicBool>,
}

impl RunningLoad {
    fn superseded(
        number: u64,
        requested: &AtomicU64,
        running: &AtomicBool,
        abandoned: &AtomicBool,
    ) -> bool {
        running.load(Ordering::SeqCst)
            && (abandoned.load(Ordering::SeqCst) || requested.load(Ordering::SeqCst) > number)
    }
}

thread_local! {
    /// The load the calling thread (a player's worker) is running.
    static RUNNING_LOAD: std::cell::RefCell<Option<RunningLoad>> = const { std::cell::RefCell::new(None) };
}

/// The calling thread (the worker) starts the load numbered `number` of `counter`'s player. `false` when a newer one
/// was already asked for: this one is not worth starting.
pub fn begin_load(counter: &LoadCounter, number: u64) -> bool {
    begin_load_abandonable(counter, number, Arc::new(AtomicBool::new(false)))
}

/// As [`begin_load`]; the streams of the load also give up once `abandoned` is set.
pub fn begin_load_abandonable(
    counter: &LoadCounter,
    number: u64,
    abandoned: Arc<AtomicBool>,
) -> bool {
    if counter.requested.load(Ordering::SeqCst) > number {
        return false;
    }
    RUNNING_LOAD.with(|slot| {
        *slot.borrow_mut() = Some(RunningLoad {
            number,
            requested: Arc::clone(&counter.requested),
            running: Arc::new(AtomicBool::new(true)),
            abandoned,
        });
    });
    true
}

/// The calling thread finished the load it began.
pub fn end_load() {
    RUNNING_LOAD.with(|slot| {
        if let Some(load) = slot.borrow_mut().take() {
            load.running.store(false, Ordering::SeqCst);
        }
    });
}

/// Whether the load the calling thread is running has been replaced by a newer request.
pub fn running_load_superseded() -> bool {
    RUNNING_LOAD.with(|slot| {
        slot.borrow().as_ref().is_some_and(|load| {
            RunningLoad::superseded(load.number, &load.requested, &load.running, &load.abandoned)
        })
    })
}

/// For a stream opened by the load the calling thread is running: `true` once that load is replaced (and only while it
/// still runs). `None` outside a load.
pub fn load_cancel() -> Option<LoadCancel> {
    RUNNING_LOAD.with(|slot| {
        slot.borrow().as_ref().map(|load| {
            let (number, requested, running, abandoned) = (
                load.number,
                Arc::clone(&load.requested),
                Arc::clone(&load.running),
                Arc::clone(&load.abandoned),
            );
            Arc::new(move || RunningLoad::superseded(number, &requested, &running, &abandoned))
                as LoadCancel
        })
    })
}

/// An open remote file.
pub struct RemoteFileHandle {
    pub reader: Box<dyn RemoteStream>,
    pub len: u64,
}

/// What a remote file looked like at one moment: its size, modified time (UTC Unix milliseconds), and optional
/// creation time (UTC Unix 100 ns ticks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteStamp {
    pub len: u64,
    pub modified_ms: Option<i64>,
    pub created_utc_100ns: Option<i64>,
}

pub trait RemoteContent: Send + Sync {
    /// Open `path` (relative to the root of `source_id`, `/`-separated) for reading, cached and tuned for the
    /// protocol. `NotFound` when the file or the source does not exist.
    fn open(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle>;

    /// As [`open`](Self::open), for a small file that belongs with a song the way its audio does (a lyrics sidecar): what is
    /// read is kept in the disk cache like the audio of a played song, so once it was read whole it is there without its
    /// server. Defaults to `open`.
    fn open_cached(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle> {
        self.open(source_id, path)
    }

    /// As [`open`](Self::open), for a reader that plays the file from start to end (the audio engine): the provider
    /// keeps reading ahead in the background so the player never waits for the network. Defaults to `open`.
    fn open_streaming(&self, source_id: SourceId, path: &str) -> io::Result<RemoteFileHandle> {
        self.open(source_id, path)
    }

    /// Ask the server for a file's size and available timestamps.
    fn stat(&self, source_id: SourceId, path: &str) -> io::Result<RemoteStamp> {
        let _ = (source_id, path);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// Replace a file on the server with the local file `local`. Nothing is written when `expected` is given and the
    /// file on the server is not as it was then; the original is untouched by a failure. Returns the new stamp.
    fn replace_file(
        &self,
        source_id: SourceId,
        path: &str,
        local: &std::path::Path,
        expected: Option<RemoteStamp>,
    ) -> io::Result<RemoteStamp> {
        let _ = (source_id, path, local, expected);
        Err(io::Error::from(io::ErrorKind::Unsupported))
    }

    /// Names the server a source's files are on (its account), so tracks of one server can be told apart from the
    /// others. Defaults to the source itself.
    fn server_key(&self, source_id: SourceId) -> String {
        source_id.to_string()
    }

    /// Whether the whole file is in the provider's disk cache, so it plays without its server (online or not). Cheap and
    /// never touches the network. Defaults to `false`.
    fn available_offline(&self, source_id: SourceId, path: &str) -> bool {
        let _ = (source_id, path);
        false
    }

    /// The file was found changed on its server (a song checked when it started playing): what the disk cache holds of
    /// it is not served for it from now on (the next play fetches the new version), while a stream that is open keeps
    /// reading what it opened. Defaults to nothing.
    fn file_changed(&self, source_id: SourceId, path: &str, now: RemoteStamp) {
        let _ = (source_id, path, now);
    }

    /// Stop every reader that has this file open (the player's handle and read-ahead): the file is about to be replaced.
    /// Whoever was reading must open it again.
    fn release(&self, source_id: SourceId, path: &str) {
        let _ = (source_id, path);
    }
}

/// The server a remote track counts under when a natural walk skips the entries of a server that could not be reached:
/// `None` for a track that plays from the disk cache (it does not need its server, so a dead server never makes the walk
/// pass it over).
pub fn walk_server_key(
    content: &dyn RemoteContent,
    source_id: SourceId,
    path: &str,
) -> Option<String> {
    if content.available_offline(source_id, path) {
        None
    } else {
        Some(content.server_key(source_id))
    }
}

/// Whether the file is only readable from the disk cache right now: it is whole in the cache but its server does not
/// answer. Opening such a file succeeds, so a lookup that found nothing next to it (a cover or lyrics sidecar) must not
/// take its own failure for "there is none".
pub fn served_from_cache_only(
    content: &dyn RemoteContent,
    source_id: SourceId,
    path: &str,
) -> bool {
    content.available_offline(source_id, path)
        && content
            .stat(source_id, path)
            .is_err_and(|error| is_remote_network_error(&error))
}

static CONTENT: RwLock<Option<Arc<dyn RemoteContent>>> = RwLock::new(None);

/// Install the provider (once at start-up; a second call replaces it).
pub fn set_remote_content(provider: Arc<dyn RemoteContent>) {
    *CONTENT.write().unwrap_or_else(PoisonError::into_inner) = Some(provider);
}

/// The installed provider, if any.
pub fn remote_content() -> Option<Arc<dyn RemoteContent>> {
    CONTENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Whether a read error means the server could not be reached (a [`RemoteError::Network`] converted to an I/O error),
/// as opposed to the file being missing or unreadable. Lookups that try several files stop after the first such error.
pub fn is_remote_network_error(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::ConnectionAborted
}

/// Read at most `limit` bytes of a remote file, or `None` when it is missing or larger than `limit` (for sidecars such
/// as `cover.jpg` and `.lrc`, which must not be loaded without a bound).
pub fn read_remote_file_capped(source_id: SourceId, path: &str, limit: u64) -> Option<Vec<u8>> {
    read_remote_file_capped_checked(source_id, path, limit)
        .ok()
        .flatten()
}

/// As [`read_remote_file_capped`], but an error of opening the file is returned, so a caller can tell "not there" from
/// "the server cannot be reached" ([`is_remote_network_error`]).
pub fn read_remote_file_capped_checked(
    source_id: SourceId,
    path: &str,
    limit: u64,
) -> io::Result<Option<Vec<u8>>> {
    let Some(content) = remote_content() else {
        return Ok(None);
    };
    let handle = content.open(source_id, path)?;
    if handle.len == 0 || handle.len > limit {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(handle.len as usize);
    if let Err(error) = handle.reader.take(limit + 1).read_to_end(&mut bytes) {
        return if is_remote_network_error(&error) {
            Err(error)
        } else {
            Ok(None)
        };
    }
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// As [`read_remote_file_capped_checked`], through [`RemoteContent::open_cached`]: a lyrics sidecar, kept in the disk
/// cache with the songs so a song that plays without its server has its sidecar too.
pub fn read_remote_sidecar_checked(
    source_id: SourceId,
    path: &str,
    limit: u64,
) -> io::Result<Option<Vec<u8>>> {
    let Some(content) = remote_content() else {
        return Ok(None);
    };
    let handle = content.open_cached(source_id, path)?;
    if handle.len == 0 || handle.len > limit {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(handle.len as usize);
    if let Err(error) = handle.reader.take(limit + 1).read_to_end(&mut bytes) {
        return if is_remote_network_error(&error) {
            Err(error)
        } else {
            Ok(None)
        };
    }
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// How long the lyric texts seen in a remote file's tags are kept for the next lookup of the same file.
const TAG_LYRICS_KEPT_FOR: std::time::Duration = std::time::Duration::from_secs(60);
/// How many files' tag lyrics are kept at once.
const TAG_LYRICS_KEPT: usize = 8;

struct TagLyrics {
    source_id: SourceId,
    path: String,
    len: u64,
    read_at: std::time::Instant,
    texts: Vec<String>,
}

static TAG_LYRICS: std::sync::Mutex<Vec<TagLyrics>> = std::sync::Mutex::new(Vec::new());

/// Keep the lyric texts (raw, as the tags hold them; empty when there are none) that a read of a remote file's tags found.
/// A song that starts has its cover and its lyrics looked up at once, and both need the same tags: the one that reads
/// them first leaves the lyrics here, so the other does not read the tags over the network again. Kept briefly, for the
/// file of this size only; nothing of it is stored.
pub fn remember_remote_tag_lyrics(source_id: SourceId, path: &str, len: u64, texts: Vec<String>) {
    let mut kept = TAG_LYRICS.lock().unwrap_or_else(PoisonError::into_inner);
    kept.retain(|item| {
        !(item.source_id == source_id && item.path == path)
            && item.read_at.elapsed() < TAG_LYRICS_KEPT_FOR
    });
    if kept.len() >= TAG_LYRICS_KEPT {
        kept.remove(0);
    }
    kept.push(TagLyrics {
        source_id,
        path: path.to_owned(),
        len,
        read_at: std::time::Instant::now(),
        texts,
    });
}

/// Forget what a read of this remote file's tags found (the file changed).
pub fn forget_remote_tag_lyrics(source_id: SourceId, path: &str) {
    TAG_LYRICS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|item| !(item.source_id == source_id && item.path == path));
}

/// The lyric texts a recent read of this remote file's tags found ([`remember_remote_tag_lyrics`]), when the file still
/// has the size it had then.
pub fn remembered_remote_tag_lyrics(
    source_id: SourceId,
    path: &str,
    len: u64,
) -> Option<Vec<String>> {
    TAG_LYRICS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|item| {
            item.source_id == source_id
                && item.path == path
                && item.len == len
                && item.read_at.elapsed() < TAG_LYRICS_KEPT_FOR
        })
        .map(|item| item.texts.clone())
}

/// `a/b/c.flac` + `cover.jpg` → `a/b/cover.jpg`: a file beside a remote track.
pub fn remote_sibling(path: &str, name: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name.to_owned(),
    }
}

/// `a/b/c.flac` → `a/b/c.lrc`: the same name with another extension.
pub fn remote_with_extension(path: &str, extension: &str) -> String {
    let (directory, file) = match path.rsplit_once('/') {
        Some((parent, file)) => (Some(parent), file),
        None => (None, path),
    };
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    match directory {
        Some(parent) => format!("{parent}/{stem}.{extension}"),
        None => format!("{stem}.{extension}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_lyrics_are_kept_briefly_for_the_same_file_only() {
        let source = SourceId::new();
        remember_remote_tag_lyrics(source, "a/b.flac", 10, vec!["[00:01.00]x".to_owned()]);
        assert_eq!(
            remembered_remote_tag_lyrics(source, "a/b.flac", 10),
            Some(vec!["[00:01.00]x".to_owned()])
        );
        assert_eq!(
            remembered_remote_tag_lyrics(source, "a/b.flac", 11),
            None,
            "another size"
        );
        assert_eq!(remembered_remote_tag_lyrics(source, "a/c.flac", 10), None);
        assert_eq!(
            remembered_remote_tag_lyrics(SourceId::new(), "a/b.flac", 10),
            None
        );
        remember_remote_tag_lyrics(source, "a/b.flac", 12, Vec::new());
        assert_eq!(
            remembered_remote_tag_lyrics(source, "a/b.flac", 12),
            Some(Vec::new())
        );
        assert_eq!(
            remembered_remote_tag_lyrics(source, "a/b.flac", 10),
            None,
            "replaced"
        );
    }

    #[test]
    fn siblings_and_extensions_stay_in_the_same_remote_folder() {
        assert_eq!(remote_sibling("a/b/c.flac", "cover.jpg"), "a/b/cover.jpg");
        assert_eq!(remote_sibling("c.flac", "cover.jpg"), "cover.jpg");
        assert_eq!(remote_with_extension("a/b/c.flac", "lrc"), "a/b/c.lrc");
        assert_eq!(remote_with_extension("a/b.c/d", "lrc"), "a/b.c/d.lrc");
        assert_eq!(remote_with_extension("c.tar.gz", "lrc"), "c.tar.lrc");
        assert_eq!(remote_with_extension("noext", "lrc"), "noext.lrc");
    }

    #[test]
    fn a_newer_load_cancels_only_the_streams_of_the_load_that_is_running() {
        let player = LoadCounter::new();
        let other_player = LoadCounter::new();
        let first = player.note_requested();
        assert!(begin_load(&player, first));
        let stream_of_first = load_cancel().expect("a load is running");
        assert!(!stream_of_first(), "nothing newer was asked for yet");
        assert!(!running_load_superseded());

        // Another player's loads are none of its business.
        other_player.note_requested();
        assert!(!stream_of_first());

        // The user chooses another track while the first one is still opening its file.
        let second = player.note_requested();
        assert!(
            stream_of_first(),
            "the stream of the replaced load gives up"
        );
        assert!(running_load_superseded());
        end_load();
        assert!(
            !stream_of_first(),
            "a finished load is never cut off by the next choice"
        );
        assert!(!running_load_superseded());

        // A load already replaced before it starts is skipped; the newest starts normally.
        let third = player.note_requested();
        assert!(!begin_load(&player, second), "replaced before it started");
        assert!(begin_load(&player, third));
        end_load();
        assert!(load_cancel().is_none());
    }
}
