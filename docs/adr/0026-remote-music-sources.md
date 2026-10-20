# Remote music sources: SMB, WebDAV and FTP

Status: Accepted on 2026-10-20 (the owner's answers are recorded under Decisions); SMB and WebDAV are implemented on Windows and Android and verified against a real SMB server, GUI and on-device acceptance are still to do (see Progress)

Decision date: 2026-10-20, Asia/Taipei (UTC+08:00)

## Context

Every source today is a local path: a Windows folder (including a `\\server\share` UNC path, which the scanner and locator key already handle), an Android folder under All files access (ADR 0014), or a playlist file. The data model is path-shaped end to end:

- `MediaLocator` is `FileSystem(PathBuf)` or `ContentUri(String)`; `LibraryRoot`, `MediaTrackRecord`, the SQLite `source_mappings` (`locator_kind`, `locator_encoding`, `locator_data`) and the `settings.json` source registry (ADR 0003) store it.
- Scanning is the `MediaIndex` trait (list, fingerprint) plus a `MetadataReader` that opens each new or changed file with lofty; `SyncEngine` reads tags with 4 threads, can be cancelled, and keeps the old projection when a source is unavailable or incomplete.
- Playback opens a `&Path`: the Windows engine (`load(&Path)`, rodio `Decoder<BufReader<File>>`, our Opus / WavPack / DSD readers, the exclusive backend, the spectrogram) and, on Android, Media3 with `file://` or the `moedsd://` / `moewv://` data-source bridge (ADR 0016, 0025).
- The project already has `reqwest` with rustls (the updater), `tokio`, and no credential storage. `backup` exports `settings.json`.

The user wants SMB, WebDAV and FTP sources on Windows and Android: scan them into the library, play them, and keep the library usable when the server is off.

## Decision

### 1. Accounts and sources; passwords are ours, sealed, and portable

A **remote account** is the connection (protocol, host, port, share or base path, user name, TLS mode, "only on Wi-Fi") and a **source** is a folder chosen under an account. `settings.json` gets a `remoteAccounts` list; sources will point at an account by id.

**The operating system does not manage the passwords** (the owner's decision, 2026-10-20: no Windows Credential Manager, no Android Keystore). The app seals them itself (`crates/player-remote/src/secret.rs`, XChaCha20-Poly1305, a fresh random nonce per message):

- **In `settings.json`** a password is only a *device-sealed secret* `v1.<nonce>.<ciphertext>`, keyed by a random 256-bit **device key** in the file `remote-credentials.key` beside the settings (created once, never overwritten, wiped from memory when dropped). The account id is bound in as associated data, so a sealed password cannot be pasted onto another account. A password that is not in sealed form (a hand edit) is dropped on load, never kept. The page never receives a password, only `hasPassword`.
- **What this protects, and what it does not**: copying `settings.json` alone (a backup, a screenshot, a support request) reveals nothing. Anyone who copies the whole data folder (settings and key file) can read the passwords, because the key has to live somewhere without the operating system; the interface says so.
- **Moving accounts to another device** is a *passphrase-sealed bundle* `p1.<salt>.<m>.<t>.<p>.<nonce>.<ciphertext>` (Argon2id, 64 MiB / 3 passes by default, costs capped when opening so a crafted file cannot exhaust memory) over the whole account list, passwords in the clear *inside* the bundle only: the **account export file** (`.moeremote`, minimum 8-character passphrase; the host names and user names are inside the sealed part too), and the same bundle as the entry `remote-accounts.moeremote` of a **full backup** when the user gives a passphrase at export. The `settings.json` in a backup never contains a password (not even a sealed one, which would mean nothing on another device). Importing opens the bundle with the passphrase and **re-seals every password with the new device's key**; accounts with the same id are replaced, others added. Without a passphrase the accounts of a backup are restored as "needs a password".
- A backup or an export file that waits for its passphrase is kept sealed in `import-pending/`; a wrong passphrase changes nothing and can be retried.

### 2. A remote locator

`MediaLocator::Remote { source_id, path }` (UTF-8 path relative to the source root, exactly as the server spells it: it has to be openable again, so it is not normalized). It is persisted as `locator_kind = 'remote'` with a JSON `locator_data`; `locator_key` is `remote:<source id>:<path>`; `TrackIdentity.source_item_id` is the relative path. Track IDs, playlists, statistics and the playback session keep working unchanged, because they use Track IDs. A remote track is playable only while its source is reachable; an unreachable source is `SourceScanState::Unavailable`, which already preserves every projection.

### 3. One client crate behind one trait

New crate `crates/player-remote`:

```text
trait RemoteFs: Send + Sync {
    fn list(&self, dir: &RemotePath) -> Result<Vec<RemoteEntry>>;      // name, is_dir, size, modified
    fn stat(&self, path: &RemotePath) -> Result<RemoteEntry>;
    fn open(&self, path: &RemotePath) -> Result<Box<dyn RemoteFile>>;   // len() + read_at(offset, buf)
}
```

Backends: **WebDAV** (`reqwest` + rustls, `PROPFIND` Depth 1 parsed with `quick-xml`, `GET` with `Range`, Basic and Digest authentication), **FTP / FTPS** (`suppaftp`, `MLSD` falling back to `LIST`, `REST` for offsets, a small connection pool), **SMB** (the pure-Rust `smb` crate, SMB2/3 with signing and encryption, on both platforms). All are MIT / Apache-2.0 and compile for Android without C toolchains beyond what the project already needs.

`RemoteReader` wraps a `RemoteFile` as `Read + Seek + Send` with a block cache (256 KiB blocks, read-ahead for sequential reads, an in-memory LRU, plus an on-disk cache folder with a size limit, default 2 GiB). Every consumer that is already generic over `Read + Seek` (lofty, our Opus / WavPack / DSD readers, the spectrogram, Symphonia through rodio) uses it as if it were a file, so no decoder is rewritten.

### 4. Scanning

`RemoteMediaIndex` implements `MediaIndex` and the parallel `MetadataReader`: a breadth-first listing (a few directories in flight), skipping hidden entries and bounding depth, fingerprint = size + server modified time (WebDAV also the ETag), then tags through ranged head and tail reads (the first 256 KiB; the tail for APE, ID3v1 and DSF). Cancelling, the 4-thread parallel read and the "partial progress is saved" rule of ADR 0003 and the 2026-10-19 sync work apply unchanged. A server that cannot be reached, an authentication failure or a TLS failure makes the scan `Unavailable` with a reason, never an empty library.

### 5. Playback

- **Windows**: the engine takes a `PlaybackSource` (`Path` or `Remote`) instead of a bare `&Path`; `open_decoder_with` builds the decoder over the file or the `RemoteReader`. Extension-based decisions (`decode_engine`, DSD detection) use the remote file name. DoP and exclusive mode work over the same reader. The engine reports **Buffering**, pauses and retries when the network drops, and prefetches the head of the next track.
- **Android**: the track is loaded as `moeremote:///<source id>/<path>`; the existing Kotlin data source bridge reads it through JNI from the Rust `RemoteReader`, so Media3 plays and seeks every format it supports, and the DSD and WavPack converters wrap the same reader (`Served` gets a remote variant). Media3 is not given the server address or the credentials.
- **SMB on Windows too**: the owner chose the built-in client (the `smb` crate) on both platforms, so a remote SMB source behaves identically on Windows and Android and does not depend on the machine's mapped drives or stored Windows credentials. A \\server\share folder that the user adds as an ordinary folder still works as a local path; it is a separate, older feature.

### 6. Artwork, lyrics, tags

Embedded covers come from the tag read. `cover.jpg` / `folder.jpg` and the `.lrc` sidecar are fetched with one capped `GET` and cached like local artwork. **Tag editing is read-only for remote tracks in the first release** (the details panel says why); the spectrogram works (streams the whole file, cancellable).

### 7. Security

TLS is verified by default; a self-signed server asks once and then **pins the certificate's SHA-256**; plain `ftp://` and `http://` show an "unencrypted" warning in the wizard. SMB requires signing and uses encryption when the server offers it. Credentials are redacted from logs and errors. The only network traffic is to the configured host.

### 8. User interface

Settings → Music sources → "Add remote source": protocol → host, port → account (password prompt; "anonymous" for FTP) → **Test connection** → browse the remote tree and tick one or more folders → save. Each remote source shows its protocol, an online / offline state, the last scan, and (Android) "only on Wi-Fi". All text goes through `tr()` in the four languages.

### 9. Phases

| Phase | Content | Done when |
| --- | --- | --- |
| P0 | `player-remote` trait, `RemoteReader` and cache, `MediaLocator::Remote`, registry and the credential encryption, the wizard shell, loopback test servers | the reader serves a local test server byte for byte; the registry round-trips |
| P1 | WebDAV and SMB scan + playback on Windows and Android (WebDAV first inside the phase, as it exercises the whole path with the simplest protocol) | a WebDAV folder and an SMB share of real songs scan, play, seek, show covers and lyrics on both platforms |
| P2 | FTP / FTPS (later, by the owner's choice) | same acceptance |
| P3 | Cache management UI, next-track prefetch, offline behaviour polish | decided later |

## Verification plan

Real songs from the library are served by loopback servers (a WebDAV server, an FTP server and a Windows share on `\\127.0.0.1`), so no NAS is needed: the byte-for-byte reader tests, a full scan, the playback and seek tests of the existing real-file suites run through the remote reader, network-drop and slow-link tests with a throttling proxy, and Android checks on the phone against the same servers over Wi-Fi.

## Considered options

- **Download the whole file before playing.** Simple, but a 600 MB DSD file takes tens of seconds to start and every seek of a lossless file is a stall for nothing; the block cache gives streaming with the same decoders.
- **Hand the remote URL to Media3 / the OS.** Works for WebDAV over HTTP on Android only, needs the credentials inside the player, and gives nothing for SMB and FTP, so one bridge for all protocols is simpler.
- **Mount everything as a drive.** Possible on Windows (and used for SMB), not on Android.
- **FUSE-like virtual filesystem in the scanner.** Hides the network from the scanner but not from playback; the explicit locator keeps the offline state visible.

## Consequences

- A new crate with three protocol clients and their tests; the audio engine's `load(&Path)` becomes a source enum (the largest change, about 140 places in `player-audio-windows`).
- Every place that assumes a local path must be revisited: tag editing, cover export, "open folder", file association, lyrics sidecars, the backup.
- The Android plugin gains the remote bridge; a metered-network option is needed. The credentials need no platform plugin (the app seals them itself).

## Decisions (the owner, 2026-10-20)

1. **Protocols**: SMB and WebDAV first; FTP later.
2. **Windows SMB**: the built-in SMB client, the same code as Android.
3. **Tag editing of remote tracks**: read-only in the first release.
4. **WebDAV authentication**: Basic and Digest; NTLM and Kerberos are not supported.
5. **Passwords**: managed by the app itself (no operating-system credential store), stored encrypted in `settings.json`, never in plaintext, and accounts can be exported and moved to another device (section 1).
6. **Cache** (default, not asked): a transient block cache bounded at 2 GiB and cleared by the user; remote tracks are not kept for offline listening in the first release.

## Progress

- Done (2026-10-20): the credential encryption and the account model (`player-remote`), the `remoteAccounts` settings field with its lenient loader, the account commands (list, save, delete, export, import), the account export / import file and the backup integration, all with tests including a restore on a second device. - Done (2026-10-20): `RemoteFs` / `RemoteFile`, the cached `RangeReader` (256 KiB blocks, 4-block read-ahead, 16 MiB, lazy seeks), and the **WebDAV client** (Basic and Digest after the server's challenge, PROPFIND listing, ranged GET with retry, base paths, Unicode names, TLS verified or pinned by SHA-256 with a fingerprint probe for the first-use prompt) tested against an in-process WebDAV server over real folders. Over WebDAV, 44 sampled files of the owner's Loss library and 41 of Hi-Res read tags, durations, sample rates and cover bytes equal to the local reads (about 2–3 requests per file; walking 300–400 files takes 0.2–0.4 s on loopback).
- Done (2026-10-20): the **SMB client** (`crates/player-remote/src/smb.rs`, the pure-Rust `smb` 0.12 crate with its blocking API, SMB 2/3 with signing and encryption, no SMB 1): `SmbFs` implements `RemoteFs` (list, stat, ranged reads), reconnects once when the link is found broken, never puts the password in an error or in `Debug` output, keeps a path from leaving the account's root, and skips hidden and system entries. Tested against the owner's Windows server (`220.134.129.189:1445`, shares `DATA1`–`DATA3`, `Downloads`): share listing, folder listing, stat, a wrong password reported as an authentication error, a seek-correct read of real music, and 8 files read at once by 4+ threads through one connection (head and tail of each in 0.3 s). **Speed**: the server is ~50 ms away, so a sequential read is latency-bound; fetching 4 blocks per request gave 16 MiB/s, 16 blocks 43 MiB/s and 32 blocks (8 MiB per request) 57 MiB/s, so `SmbFs::open_reader` reads ahead 32 blocks (`SMB_READ_AHEAD_BLOCKS`). The test file is `crates/player-remote/tests/real_smb.rs` and is skipped unless `MOE_SMB_HOST`, `MOE_SMB_PORT`, `MOE_SMB_SHARE`, `MOE_SMB_FILE` (a text file: user name, then password) and optionally `MOE_SMB_DIR` are set.
- Done (2026-10-20, later): **the whole data path, the Windows and Android playback code and the interface**.
  - `MediaLocator::Remote` / `MediaSourceKind::Remote` (SQLite `locator_kind = 'remote'`, `moeremote://` text form for playlists); the tag reader moved to `player-track-metadata` and reads from a `ByteSource`, so the Windows scanner and the remote scanner share one parser (mka included).
  - `RemoteMediaIndex` (`player-remote/src/index.rs`): 4 listing workers, 16 parallel tag reads (a new `MediaIndex::metadata_read_threads`); 2,668 Hi-Res tracks scan over a ~50 ms link in 36 s with no tag errors (131 s with 4 readers); an unreachable or refusing server is `Unavailable` / `PermissionRevoked` and never removes tracks.
  - One process-wide `player_core::RemoteContent` provider (installed by the app) serves the audio engine, the cover and lyrics lookups and the Android bridge. The engine takes a `moeremote://` pseudo path (`player-audio-windows/src/remote_input.rs`); every decoder (rodio formats, Opus, WavPack, WavPack-DSD, DSF, DoP) opens it through the provider, and remote decoding equals local decoding sample for sample in the unit tests.
  - **Streaming**: a decoder reads inside the audio thread, so a plain cached reader stalls the output for a round trip at every cache miss. `PrefetchReader` reads ahead on a thread (16 MiB); at playback pace the slowest read on the real server was 84 microseconds. Tags, covers and lyrics keep the plain `RangeReader`.
  - Android: `moeremote://<source id>/<path>` through the Kotlin data source and JNI; DSF and WavPack are converted to float WAV like local files, every other format is served byte for byte for Media3. Covers (notification and album) and lyrics of remote tracks come from `src-tauri/src/remote_media.rs`. The `smb` crate cross-compiles for aarch64-linux-android and the APK builds.
  - Interface: settings -> music sources -> "Remote servers" (add / edit / delete a server, list SMB shares, test the connection, browse and add folders, export / import accounts), in all four languages.
  - A real-server test of the app-side chain (account with a sealed password -> source -> sync of 2,668 tracks -> playback path -> engine reader -> streaming pace) is `src-tauri/src/remote_connections.rs` `real_server_tests` (skipped unless `MOE_SMB_*` is set).
- Not done: Release GUI (CDP) and on-device (NX809J) acceptance; FTP; the "Buffering" state and automatic retry when the link drops during playback (the track fails today); the track details panel, spectrogram and tag editing for remote tracks (read-only message); DSEE HX is not applied to remote tracks. Decision 3 below (the first release is read-only) stands.

- Playlist files on a server (2026-10-20): a source of kind PlaylistFile with `remote_account`; the songs it names are read from the same account, and songs a remote folder source already holds are not scanned twice. See `src-tauri/src/remote_playlist_sync.rs`.
