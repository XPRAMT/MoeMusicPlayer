# Remote music sources: SMB, WebDAV and FTP

Status: Accepted in principle on 2026-10-20 (the owner's answers are recorded under Decisions); design only, nothing is implemented

Decision date: 2026-10-20, Asia/Taipei (UTC+08:00)

## Context

Every source today is a local path: a Windows folder (including a `\\server\share` UNC path, which the scanner and locator key already handle), an Android folder under All files access (ADR 0014), or a playlist file. The data model is path-shaped end to end:

- `MediaLocator` is `FileSystem(PathBuf)` or `ContentUri(String)`; `LibraryRoot`, `MediaTrackRecord`, the SQLite `source_mappings` (`locator_kind`, `locator_encoding`, `locator_data`) and the `settings.json` source registry (ADR 0003) store it.
- Scanning is the `MediaIndex` trait (list, fingerprint) plus a `MetadataReader` that opens each new or changed file with lofty; `SyncEngine` reads tags with 4 threads, can be cancelled, and keeps the old projection when a source is unavailable or incomplete.
- Playback opens a `&Path`: the Windows engine (`load(&Path)`, rodio `Decoder<BufReader<File>>`, our Opus / WavPack / DSD readers, the exclusive backend, the spectrogram) and, on Android, Media3 with `file://` or the `moedsd://` / `moewv://` data-source bridge (ADR 0016, 0025).
- The project already has `reqwest` with rustls (the updater), `tokio`, and no credential storage. `backup` exports `settings.json`.

The user wants SMB, WebDAV and FTP sources on Windows and Android: scan them into the library, play them, and keep the library usable when the server is off.

## Decision

### 1. A source describes a connection, never a secret

`settings.json` gets a new source kind `remote` with `protocol` (`smb` | `webdav` | `ftp`), `host`, `port`, `tls` (`verify` | `pinned:<sha256>` | `off`), `username`, `basePath` (the share and folder, or the WebDAV / FTP directory), `onlyOnWifi` (Android) and a `credentialId`. The **password lives in the platform credential store** (Windows Credential Manager through the `keyring` crate; Android through a Keystore-wrapped file written by the plugin), never in `settings.json`, SQLite, logs or the backup zip. A restored backup shows the source as "needs password".

### 2. A remote locator

`MediaLocator::Remote { source_id, path }` (UTF-8, NFC-normalized path relative to the source root). It is persisted as `locator_kind = 'remote'` with a JSON `locator_data`; `locator_key` is `remote:<source id>:<path>`; `TrackIdentity.source_item_id` is the relative path. Track IDs, playlists, statistics and the playback session keep working unchanged, because they use Track IDs. A remote track is playable only while its source is reachable; an unreachable source is `SourceScanState::Unavailable`, which already preserves every projection.

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
| P0 | `player-remote` trait, `RemoteReader` and cache, `MediaLocator::Remote`, registry and credential store, the wizard shell, loopback test servers | the reader serves a local test server byte for byte; the registry round-trips |
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
- The Android plugin gains a credential store and the remote bridge; a metered-network option is needed.

## Decisions (the owner, 2026-10-20)

1. **Protocols**: SMB and WebDAV first; FTP later.
2. **Windows SMB**: the built-in SMB client, the same code as Android.
3. **Tag editing of remote tracks**: read-only in the first release.
4. **WebDAV authentication**: Basic and Digest; NTLM and Kerberos are not supported.
5. **Cache** (default, not asked): a transient block cache bounded at 2 GiB and cleared by the user; remote tracks are not kept for offline listening in the first release.
