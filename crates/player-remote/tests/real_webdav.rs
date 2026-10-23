//! The WebDAV client against a real server (ADR 0035 P1): Apache mod_dav in WSL, set up as in ADR 0035 §5A. All
//! tests are ignored and skip themselves when the environment is missing. Nothing secret is printed.
//!
//! - `MOE_WEBDAV_HOST`, `MOE_WEBDAV_PORT` (https only).
//! - `MOE_WEBDAV_PATH`: the read-only music folder (`/music`); `MOE_WEBDAV_SCRATCH_PATH`: a writable folder
//!   (`/scratch`); `MOE_WEBDAV_DIGEST_PATH`: the music folder behind Digest authentication (`/digest`, optional).
//! - `MOE_WEBDAV_CRED_FILE`: a file outside the repository with `MOE_WEBDAV_USER=` and `MOE_WEBDAV_PASS=` lines.
//! - `MOE_WEBDAV_CERT_SHA256`: the certificate to pin (optional; otherwise the one the server presents).
//! - `MOE_WEBDAV_WRITE_TEST=1`: run the tests that write (only files they create in the scratch folder).
//! - `MOE_WEBDAV_RESTART_CMD`: a command that restarts the server (reconnect test);
//!   `MOE_WEBDAV_STALL_CMD` / `MOE_WEBDAV_RESUME_CMD`: freeze and thaw it (dead-link test).
//!
//! Run: `cargo test -p player-remote --test real_webdav --release -- --ignored --nocapture --test-threads=1`.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lofty::file::AudioFile as _;
use player_core::{
    LibraryRoot, ListTracksQuery, MediaLocator, MediaSourceKind, SourceId, SourceScanState,
    SyncEngine,
};
use player_db::Database;
use player_remote::account::{RemoteAccount, RemoteProtocol, TlsMode};
use player_remote::fs::{FileStamp, RangeReader, RemoteError, RemoteFile, RemoteFs};
use player_remote::index::{RemoteMediaIndex, RemoteResolver, ResolvedSource};
use player_remote::prefetch::PrefetchReader;
use player_remote::webdav::WebDavFs;
use zeroize::Zeroizing;

struct Env {
    host: String,
    port: u16,
    user: String,
    password: Zeroizing<String>,
    pin: String,
}

fn env() -> Option<Env> {
    let host = std::env::var("MOE_WEBDAV_HOST").ok()?;
    let port = std::env::var("MOE_WEBDAV_PORT").ok()?.parse().ok()?;
    let creds = std::fs::read_to_string(std::env::var("MOE_WEBDAV_CRED_FILE").ok()?)
        .expect("MOE_WEBDAV_CRED_FILE is readable");
    let value = |key: &str| {
        creds.lines().find_map(|line| {
            line.trim()
                .strip_prefix(key)
                .map(|rest| rest.trim().to_owned())
        })
    };
    let pin = match std::env::var("MOE_WEBDAV_CERT_SHA256") {
        Ok(pin) => pin.replace(':', "").to_ascii_lowercase(),
        // Configured but unreachable is a failure, not a skip.
        Err(_) => player_remote::tls::probe_certificate_sha256(&host, port)
            .unwrap_or_else(|error| panic!("cannot reach {host}:{port}: {error}")),
    };
    Some(Env {
        host,
        port,
        user: value("MOE_WEBDAV_USER=")?,
        password: Zeroizing::new(value("MOE_WEBDAV_PASS=")?),
        pin,
    })
}

fn path_var(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn account(env: &Env, path: &str, tls: TlsMode) -> RemoteAccount {
    RemoteAccount {
        id: "real-webdav".to_owned(),
        name: "real".to_owned(),
        protocol: RemoteProtocol::Webdav,
        host: env.host.clone(),
        port: Some(env.port),
        path: path.to_owned(),
        username: env.user.clone(),
        password: None,
        tls,
        only_on_wifi: false,
    }
}

fn fs_at(env: &Env, path: &str) -> WebDavFs {
    WebDavFs::new(
        &account(
            env,
            path,
            TlsMode::Pinned {
                sha256: env.pin.clone(),
            },
        ),
        Some(env.password.clone()),
    )
    .expect("client")
}

fn music(env: &Env) -> WebDavFs {
    fs_at(env, &path_var("MOE_WEBDAV_PATH", "/music"))
}

fn audio_files(fs: &dyn RemoteFs) -> Vec<(String, u64)> {
    let mut files: Vec<_> = fs
        .list("")
        .expect("list")
        .into_iter()
        .filter(|entry| !entry.is_dir && player_remote::index::is_remote_library_audio(&entry.name))
        .map(|entry| (entry.name, entry.size))
        .collect();
    files.sort();
    files
}

fn run(command: &str) {
    let status = std::process::Command::new("cmd")
        .args(["/C", command])
        .status()
        .expect("run the command");
    assert!(status.success(), "the command failed");
}

#[test]
#[ignore]
fn lists_stats_and_reads_ranges_including_cjk_and_space_names() {
    let Some(env) = env() else { return };
    let fs = music(&env);
    let files = audio_files(&fs);
    assert!(!files.is_empty(), "the music folder has audio files");
    for (name, size) in &files {
        let entry = fs.stat(name).expect("stat");
        assert_eq!(entry.size, *size);
        assert!(entry.modified_ms.is_some());
        assert_eq!(
            entry.created_utc_100ns, None,
            "WebDAV creationdate is not stored as a creation time"
        );
        let mut file = fs.open(name).expect("open");
        let mut head = [0u8; 64];
        assert_eq!(file.read_at(0, &mut head).unwrap(), 64);
        let mut tail = vec![0u8; 100];
        assert_eq!(file.read_at(size - 100, &mut tail).unwrap(), 100);
        assert_eq!(file.read_at(*size, &mut tail).unwrap(), 0, "past the end");
        println!("{} bytes, mtime ok: {}", size, name.chars().count());
    }
    assert_eq!(fs.stat("沒有這個檔案 x.flac"), Err(RemoteError::NotFound));
    assert_eq!(fs.list("沒有這個資料夾"), Err(RemoteError::NotFound));
}

#[test]
#[ignore]
fn digest_authentication_lists_the_same_files() {
    let Some(env) = env() else { return };
    let Ok(digest) = std::env::var("MOE_WEBDAV_DIGEST_PATH") else {
        return;
    };
    assert_eq!(
        audio_files(&fs_at(&env, &digest)),
        audio_files(&music(&env))
    );
    let wrong = WebDavFs::new(
        &account(
            &env,
            &digest,
            TlsMode::Pinned {
                sha256: env.pin.clone(),
            },
        ),
        Some(Zeroizing::new("wrong-password".to_owned())),
    )
    .unwrap();
    assert_eq!(wrong.list(""), Err(RemoteError::Auth));
}

#[test]
#[ignore]
fn streaming_gives_the_same_bytes_and_tags_as_the_downloaded_file() {
    let Some(env) = env() else { return };
    let fs: Arc<dyn RemoteFs> = Arc::new(music(&env));
    let dir = std::env::temp_dir().join(format!("moe-real-webdav-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (index, (name, size)) in audio_files(fs.as_ref()).into_iter().enumerate() {
        // "The local file": the whole file through the block reader.
        let mut whole = Vec::new();
        RangeReader::new(fs.open(&name).unwrap())
            .read_to_end(&mut whole)
            .unwrap();
        assert_eq!(whole.len() as u64, size);
        let extension = name.rsplit('.').next().unwrap();
        let local = dir.join(format!("{index}.{extension}"));
        std::fs::write(&local, &whole).unwrap();
        // The playback path: the prefetching reader, read through and then with seeks.
        let started = Instant::now();
        let mut stream = PrefetchReader::new(fs.open(&name).unwrap());
        let mut streamed = Vec::new();
        stream.read_to_end(&mut streamed).unwrap();
        let seconds = started.elapsed().as_secs_f64();
        assert!(streamed == whole, "streamed bytes differ");
        for offset in [size / 3, 17, size - 4096, size / 2] {
            let mut part = vec![0u8; 4096];
            stream.seek(SeekFrom::Start(offset)).unwrap();
            stream.read_exact(&mut part).unwrap();
            assert_eq!(part[..], whole[offset as usize..offset as usize + 4096]);
        }
        // The same tags from the streamed bytes and the local file.
        let from_local = lofty::read_from_path(&local).expect("tags of the local copy");
        let mut cursor = std::io::Cursor::new(streamed);
        let from_stream = lofty::probe::Probe::new(&mut cursor)
            .guess_file_type()
            .unwrap()
            .read()
            .expect("tags of the stream");
        assert_eq!(
            from_local.properties().duration(),
            from_stream.properties().duration()
        );
        println!(
            "file {index}: {:.1} MiB streamed at {:.1} MiB/s (through the stdio relay)",
            size as f64 / 1048576.0,
            size as f64 / 1048576.0 / seconds
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}

struct Fixed(Arc<dyn RemoteFs>);

impl RemoteResolver for Fixed {
    fn resolve(&self, _: SourceId) -> Result<ResolvedSource, RemoteError> {
        Ok(ResolvedSource {
            fs: Arc::clone(&self.0),
            base_path: String::new(),
            excluded: Vec::new(),
        })
    }
}

/// Counts every request and byte of the files it opens.
struct Counting {
    inner: Arc<dyn RemoteFs>,
    requests: Arc<std::sync::atomic::AtomicU64>,
    bytes: Arc<std::sync::atomic::AtomicU64>,
}

struct CountingFile {
    inner: Box<dyn RemoteFile>,
    requests: Arc<std::sync::atomic::AtomicU64>,
    bytes: Arc<std::sync::atomic::AtomicU64>,
}

impl RemoteFile for CountingFile {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, RemoteError> {
        let read = self.inner.read_at(offset, buf)?;
        self.requests
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.bytes
            .fetch_add(read as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(read)
    }
}

impl RemoteFs for Counting {
    fn list(&self, dir: &str) -> Result<Vec<player_remote::fs::RemoteEntry>, RemoteError> {
        self.inner.list(dir)
    }
    fn stat(&self, path: &str) -> Result<player_remote::fs::RemoteEntry, RemoteError> {
        self.inner.stat(path)
    }
    fn open(&self, path: &str) -> Result<Box<dyn RemoteFile>, RemoteError> {
        Ok(Box::new(CountingFile {
            inner: self.inner.open(path)?,
            requests: Arc::clone(&self.requests),
            bytes: Arc::clone(&self.bytes),
        }))
    }
}

#[test]
#[ignore]
fn the_sync_engine_scans_the_server_into_a_library_with_its_transfer() {
    let Some(env) = env() else { return };
    let inner: Arc<dyn RemoteFs> = Arc::new(music(&env));
    let expected = audio_files(inner.as_ref()).len() as u64;
    let requests = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let bytes = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let fs: Arc<dyn RemoteFs> = Arc::new(Counting {
        inner,
        requests: Arc::clone(&requests),
        bytes: Arc::clone(&bytes),
    });
    let mut db = Database::open_in_memory().unwrap();
    let id = SourceId::new();
    let root = LibraryRoot {
        id,
        kind: MediaSourceKind::Remote,
        display_name: "webdav".to_owned(),
        locator: MediaLocator::Remote {
            source_id: id,
            path: String::new(),
        },
        enabled: true,
    };
    db.save_library_root(&root).unwrap();
    let mut index = RemoteMediaIndex::new(Arc::new(Fixed(fs)));
    let started = Instant::now();
    let report = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_000_000).expect("sync");
    let elapsed = started.elapsed();
    assert_eq!(report.state, Some(SourceScanState::Complete));
    assert_eq!(report.observed, expected);
    assert!(
        report.metadata_errors.is_empty(),
        "{:?}",
        report.metadata_errors
    );
    let tracks = db
        .list_tracks_page(ListTracksQuery {
            offset: 0,
            limit: 100,
            query: None,
            field_filter: None,
            sort: Default::default(),
        })
        .unwrap()
        .items;
    assert_eq!(tracks.len() as u64, expected);
    for track in &tracks {
        println!(
            "  {:?} / {:?} / {:?} ms",
            track.title, track.artist, track.duration_ms
        );
    }
    let requests = requests.load(std::sync::atomic::Ordering::Relaxed);
    let bytes = bytes.load(std::sync::atomic::Ordering::Relaxed);
    println!(
        "scan: {expected} tracks in {} ms; per file {:.1} reads, {:.1} KiB (through the stdio relay)",
        elapsed.as_millis(),
        requests as f64 / expected as f64,
        bytes as f64 / 1024.0 / expected as f64
    );
    let again = SyncEngine::sync(&root, &mut index, &mut db, 1_800_000_100_000).expect("again");
    assert_eq!(again.unchanged, expected);
}

#[test]
#[ignore]
fn latency_of_small_requests_on_kept_alive_connections() {
    let Some(env) = env() else { return };
    let fs = music(&env);
    let (name, _) = audio_files(&fs).into_iter().next().unwrap();
    let mut file = fs.open(&name).unwrap();
    let mut times = Vec::new();
    for _ in 0..30 {
        let started = Instant::now();
        fs.stat(&name).unwrap();
        let mut buf = [0u8; 64];
        file.read_at(0, &mut buf).unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "stat + 64-byte read: median {:.1} ms, worst {:.1} ms (through the stdio relay; the first opens a connection)",
        times[times.len() / 2],
        times[times.len() - 1]
    );
    // Sixteen threads at once, as the tag readers do.
    let fs = Arc::new(fs);
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..16 {
            let fs = Arc::clone(&fs);
            let name = name.clone();
            scope.spawn(move || {
                for _ in 0..10 {
                    fs.stat(&name).unwrap();
                }
            });
        }
    });
    println!(
        "160 stats on 16 threads: {} ms",
        started.elapsed().as_millis()
    );
}

#[test]
#[ignore]
fn the_read_only_folder_refuses_writes_and_deletes() {
    let Some(env) = env() else { return };
    let fs = music(&env);
    let (name, _) = audio_files(&fs).into_iter().next().unwrap();
    assert_eq!(fs.remove_file(&name), Err(RemoteError::Auth));
    let new = b"x".to_vec();
    assert_eq!(
        fs.replace_file("moe-should-not-exist.txt", &mut new.as_slice(), 1, None),
        Err(RemoteError::Auth)
    );
    assert!(fs.stat("moe-should-not-exist.txt").is_err());
    assert!(fs.stat(&name).is_ok(), "still there");
}

#[test]
#[ignore]
fn a_pin_is_offered_once_and_a_changed_certificate_is_refused() {
    let Some(env) = env() else { return };
    let path = path_var("MOE_WEBDAV_PATH", "/music");
    // Verified normally, a self-signed certificate is refused as untrusted: the case the form offers to pin.
    let verify = WebDavFs::new(
        &account(&env, &path, TlsMode::Verify),
        Some(env.password.clone()),
    )
    .unwrap();
    let error = verify.list("").unwrap_err();
    assert!(matches!(error, RemoteError::Tls(_)), "{error:?}");
    assert!(error.to_string().starts_with("憑證不受信任"));
    // What the form shows is what the pinned client accepts.
    let probed = player_remote::tls::probe_certificate_sha256(&env.host, env.port).unwrap();
    assert!(probed.eq_ignore_ascii_case(&env.pin));
    assert!(music(&env).list("").is_ok());
    // Another certificate (as after the server's certificate changed): refused, with the change named.
    let other = WebDavFs::new(
        &account(
            &env,
            &path,
            TlsMode::Pinned {
                sha256: "00".repeat(32),
            },
        ),
        Some(env.password.clone()),
    )
    .unwrap();
    let error = other.list("").unwrap_err();
    assert!(error.to_string().contains("與你先前接受的不同"), "{error}");
}

fn scratch(env: &Env) -> Option<WebDavFs> {
    if std::env::var("MOE_WEBDAV_WRITE_TEST").ok().as_deref() != Some("1") {
        return None;
    }
    Some(fs_at(env, &path_var("MOE_WEBDAV_SCRATCH_PATH", "/scratch")))
}

#[test]
#[ignore]
fn replace_refuses_a_changed_file_and_keeps_one_bak() {
    let Some(env) = env() else { return };
    let Some(fs) = scratch(&env) else { return };
    let name = format!("moe 測試 {}.m3u8", std::process::id());
    let bak = format!("{name}.bak");
    let first = "#EXTM3U\n../music/a.flac\n".as_bytes().to_vec();
    let entry = fs
        .replace_file(&name, &mut first.as_slice(), first.len() as u64, None)
        .expect("create");
    assert_eq!(entry.size, first.len() as u64);
    let stamp = FileStamp::of(&entry);
    // Changed behind our back (another client): refused, the file untouched.
    std::thread::sleep(Duration::from_millis(1100));
    let other = b"changed elsewhere\n".to_vec();
    fs.replace_file(&name, &mut other.as_slice(), other.len() as u64, None)
        .unwrap();
    let mine = b"mine\n".to_vec();
    assert_eq!(
        fs.replace_file(&name, &mut mine.as_slice(), mine.len() as u64, Some(stamp)),
        Err(RemoteError::Changed)
    );
    let read_all = |path: &str| {
        let mut data = Vec::new();
        RangeReader::new(fs.open(path).unwrap())
            .read_to_end(&mut data)
            .unwrap();
        data
    };
    assert_eq!(read_all(&name), other);
    // The single-backup flow, twice.
    for (new, kept) in [
        (&b"two\n"[..], other.clone()),
        (&b"three\n"[..], b"two\n".to_vec()),
    ] {
        let before = FileStamp::of(&fs.stat(&name).unwrap());
        fs.replace_file_keeping_backup(&name, &mut &new[..], new.len() as u64, Some(before), &bak)
            .expect("replace keeping a backup");
        assert_eq!(read_all(&name), new);
        assert_eq!(read_all(&bak), kept);
    }
    let strays: Vec<_> = fs
        .list("")
        .unwrap()
        .into_iter()
        .filter(|entry| entry.name.starts_with(".moetmp-"))
        .collect();
    assert!(strays.is_empty());
    fs.remove_file(&name).unwrap();
    fs.remove_file(&bak).unwrap();
}

#[test]
#[ignore]
fn playlist_write_back_edits_the_file_on_the_server() {
    let Some(env) = env() else { return };
    let Some(fs) = scratch(&env) else { return };
    let dir = format!("moe-playlist-{}", std::process::id());
    let name = format!("{dir}.m3u8");
    let original = "\u{feff}#EXTM3U\r\n#EXTINF:1,A - a\r\nAlbum/a.flac\r\nAlbum/c.flac\r\n"
        .as_bytes()
        .to_vec();
    fs.replace_file(&name, &mut original.as_slice(), original.len() as u64, None)
        .unwrap();
    let outcome = player_remote::playlist_edit::edit_playlist_file(
        &fs,
        &name,
        &player_remote::playlist_edit::Edit::Add {
            server_path: "Album/b.flac",
            compare: &player_db::natural_title_cmp,
            title: Some("B - b"),
            duration_ms: Some(1000),
        },
    );
    assert!(outcome.is_ok(), "{outcome:?}");
    let mut data = Vec::new();
    RangeReader::new(fs.open(&name).unwrap())
        .read_to_end(&mut data)
        .unwrap();
    let text = String::from_utf8(data).unwrap();
    assert!(
        text.starts_with('\u{feff}') && text.contains("\r\nAlbum/b.flac\r\n"),
        "{text:?}"
    );
    let a = text.find("a.flac").unwrap();
    let b = text.find("b.flac").unwrap();
    let c = text.find("c.flac").unwrap();
    // As with local favorites: after the last entry of the same folder.
    assert!(a < c && c < b, "added after the folder's last entry: {text:?}");
    let _ = fs.remove_file(&name);
    let _ = fs.remove_file(&format!("{name}.bak"));
}

#[test]
#[ignore]
fn a_restarted_server_is_reconnected_and_a_frozen_one_is_found_within_seconds() {
    let Some(env) = env() else { return };
    let fs = Arc::new(music(&env));
    let (name, size) = audio_files(fs.as_ref()).into_iter().next().unwrap();
    let mut file = fs.open(&name).unwrap();
    let mut buf = vec![0u8; 256 * 1024];
    file.read_at(0, &mut buf).unwrap();
    if let Ok(restart) = std::env::var("MOE_WEBDAV_RESTART_CMD") {
        run(&restart);
        // The kept-alive connection is gone; the next read opens a new one.
        let started = Instant::now();
        let read = file
            .read_at(size / 2, &mut buf)
            .expect("read after the restart");
        assert!(read > 0);
        println!(
            "first read after a restart: {} ms",
            started.elapsed().as_millis()
        );
        // A prefetching stream that is mid-file when the server restarts finishes byte for byte.
        let mut whole = Vec::new();
        RangeReader::new(fs.open(&name).unwrap())
            .read_to_end(&mut whole)
            .unwrap();
        let mut stream =
            PrefetchReader::with_geometry(fs.open(&name).unwrap(), 256 * 1024, 512 * 1024);
        let mut streamed = vec![0u8; (size / 4) as usize];
        stream.read_exact(&mut streamed).unwrap();
        run(&restart);
        stream.read_to_end(&mut streamed).unwrap();
        assert!(
            streamed == whole,
            "the stream continued without gaps or repeats"
        );
    }
    if let (Ok(stall), Ok(resume)) = (
        std::env::var("MOE_WEBDAV_STALL_CMD"),
        std::env::var("MOE_WEBDAV_RESUME_CMD"),
    ) {
        // The server is thawed however this ends, even when a step panics: never leave it frozen.
        struct Thaw(String);
        impl Drop for Thaw {
            fn drop(&mut self) {
                let _ = std::process::Command::new("cmd").args(["/C", &self.0]).status();
            }
        }
        let thaw = Thaw(resume);
        run(&stall);
        let started = Instant::now();
        let result = file.read_at(1024, &mut buf);
        let waited = started.elapsed();
        drop(thaw);
        assert!(matches!(result, Err(RemoteError::Network(_))), "{result:?}");
        assert!(waited < Duration::from_secs(25), "found in {waited:?}");
        println!(
            "a frozen server was reported after {} ms",
            waited.as_millis()
        );
        assert!(
            file.read_at(1024, &mut buf).unwrap() > 0,
            "and reads again once it is back"
        );
    }
}
