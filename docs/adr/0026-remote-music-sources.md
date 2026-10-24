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
- Done (2026-10-22): **a slow link is told from a dead one, and a dead one is retried** (the owner's design, phase 1). (1) `smb_transport.rs`: every byte received is noted in a `LinkHealth` (time of the last byte, byte count), and a message is received whole even when the 100 ms poll timeout falls inside it (the bytes read so far are kept for the next call; before, a timeout in the middle of a message could break the framing). `LinkHealth::kill` shuts the socket down, which ends the crate's receive loop and fails every request on the link at once. (2) `smb.rs` `bounded_on` (list, stat, open and file reads): a request waits as long as bytes keep coming (reads up to `READ_LIMIT` 120 s, other requests `OPERATION_TIMEOUT` 10 s); 1 s without a single byte (`STALL_DETECT`) asks the server something small in place of an SMB2 ECHO (`Link::ping`: open the share's root for its attributes and close it; the `smb` 0.12 crate keeps its message handlers private, so a real ECHO cannot be sent through it, and a hand-made one would bypass its message ids, credits and signing), and 1 s more without any byte (`ECHO_WAIT`) cuts the link and reports a network error, so a dead link is found in about 2 s. The probe's answer is not awaited: it only has to bring a byte back. Writes keep their fixed 30 s limit (nothing comes back while uploading). The fixed 15 s `READ_TIMEOUT` is gone. (3) `prefetch.rs`: a `Network` failure no longer stops the fetch thread; it keeps what it has and fetches the same place again after 0, 1, 2, 4, 8, 10, 10… s (`RETRY_BACKOFF`), the file reopening itself and the file system connecting again, until it works (reading on with nothing missing or repeated) or 3 minutes (`RETRY_GIVE_UP`) have passed since the outage began; any other error (file changed or released, refused) fails at once; a seek cuts the wait short and a seek after giving up tries again; the 45 s reader limit is gone (a 6-minute guard remains only for a fetch thread that is gone). Giving up fails the read with a message starting with `player_core::REMOTE_LINK_LOST` (「伺服器無法連線」). (4) Players: Windows (`remote_input.rs`) notes that failure; when the decoder then ends, the worker pauses at that position with `AudioError::RemoteLinkLost` (shown as the message) instead of ending the track, so the queue does not move on; Play reloads the track and seeks back to that position. Android: `nativeRead` returns -2, `DsdDataSource` throws `RemoteLinkLostException` (a `FileNotFoundException`, which Media3's default policy does not retry), the player stops in the error state with that message, and no "ended" event advances the queue. Tests: transport framing across timeouts, one byte every 0.5 s not called dead, silence found dead after about 2 s and the cut ending the receiver (local `TcpListener`), the watchdog's decisions, and in `prefetch.rs` an outage that recovers (bytes complete and in order), an outage that lasts (error after the allowance), a non-network error failing at once, the backoff steps, a seek cutting the wait, a seek after giving up. Not verified against a real server dropping off, and the "reconnecting" state is not shown in the interface yet (phase 2).
- Not done: Release GUI (CDP) and on-device (NX809J) acceptance; FTP; the "Buffering" / "reconnecting" state in the interface (the retry itself is done, see above); the track details panel, spectrogram and tag editing for remote tracks (read-only message); DSEE HX is not applied to remote tracks. Decision 3 below (the first release is read-only) stands.

- Playlist files on a server (2026-10-20): a source of kind PlaylistFile with `remote_account`; the songs it names are read from the same account, and songs a remote folder source already holds are not scanned twice. See `src-tauri/src/remote_playlist_sync.rs`.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

- **來源與身分**：`MediaLocator::Remote { source_id, path }`（root 的 path 為帳號內基底資料夾，曲目 path 相對於基底），`MediaSourceKind::Remote`，SQLite `locator_kind = 'remote'`。`settings.json` 的來源為 `SourceEntryKind::Folder { media_kind: Remote, path: StoredPath::Uri("moeremote://<來源 id>/<基底>"), remote_account: Some(帳號 id) }`；M3U／我的最愛以 `moeremote://<來源 id>/<百分比編碼路徑>` 表示（`player_core::remote_locator_uri`／`parse_remote_locator_uri`）。`locator_key` 為帳號層級 `remote-acct:{帳號 id}:{帳號根目錄起算路徑}`（`player_remote::index::remote_locator_key`，`RemoteResolver::account_id`），同一帳號下同一檔案只有一個 TrackId；每個來源保有自己的 mapping。`player-db` 的 `unify_remote_tracks` 在每次遠端或播放清單掃描交易最後合併同鍵不同 TrackId 的曲目（`merge_tracks`：存活者取播放統計較多者→有資料夾 mapping 者→較小 TrackId；mapping、清單項目、session 項目移過去，播放時間相加，手動歌詞優先，`track_lyrics` JSON 的 `trackId` 一併改寫）；舊鍵 `remote:…` 在來源下次同步時改寫，不需遷移。實機資料庫合併未驗證。

- **根目錄**：SMB 欄位叫「根目錄」，選擇器（`remote_browse_draft`）只列共用資料夾，點一個即選定並關閉；WebDAV 選擇器從伺服器 `/` 進入資料夾並以「使用這個資料夾」選目前那層。資料夾、播放清單、排除資料夾都在該伺服器分頁選（相對於根目錄）。舊版更深的 SMB 根目錄（如 `DATA1/Music`）照常可用、不遷移。未來規劃（使用者決定先不做）：SMB 根目錄＝整台伺服器需要跨共用資料夾的 `RemoteFs` 層，且改既有帳號根目錄會改變所有路徑身分，既有帳號如何處理尚未決定。

- **SMB 用戶端**：`player-remote` 的 `smb.rs`（vendored `smb`／`smb-rpc` 0.12.1 MIT fork，見 `vendor/smb-rs/UPSTREAM.md`；SMB 2/3 含簽章與加密）。`smb_transport.rs` 的 `NoDelayTransport` 設 `TCP_NODELAY` 並把標頭與訊息一次寫出（避免 Nagle＋延遲 ACK 每來回多約 45 ms），`Link` 以 `Connection::from_transport`＋`authenticate`＋`tree_connect` 建立（需 `sspi =0.21.3`）；`list_shares` 仍用 `Client`。SRVSVC NetrShareEnum：Bind 提出 NDR64 與 NDR32、依 BindAck 選擇；兩種語法都支援 `ERROR_MORE_DATA` 續頁（拒絕異常游標、超量項目、超過 1,024 頁；每次最多 65,536 筆、字串合計 4,194,304 個 UTF-16 code unit），NDR32 用專用解析器，NDR64 referent ID 只判斷 null／non-null。每次伺服器呼叫由 `bounded_on` 從外部限時：有位元組進來就繼續等（讀檔上限 `READ_LIMIT` 120 秒、其他 `OPERATION_TIMEOUT` 10 秒、連線 `CONNECT_TIMEOUT` 10 秒、寫入固定 30 秒）；`LinkHealth` 記錄收到的位元組，連續 1 秒無資料（`STALL_DETECT`）送 `Link::ping`（開共用資料夾根目錄讀屬性），再 1 秒（`ECHO_WAIT`）仍無位元組即判定斷線、`LinkHealth::kill` 並回 `Network`；`with_retry` 重連後重試一次。播放預讀 32 個 256 KiB 區塊。

- **WebDAV 用戶端**：`webdav.rs`（Basic／Digest、憑證釘選）：新增入口已開（`src/lib/remote-protocols.js` 的 `WEBDAV_ENTRY_VISIBLE = true`）；https 遇不受信任憑證時跳指紋確認面板（`remote_certificate_probe`），信任後存 `TlsMode::Pinned`，已釘選的憑證改變只報 `Tls` 錯誤不再詢問。每請求各自限時（`request_timeout`，10 秒＋每 64 KiB 一秒），停住時回 `Network` 且 `read_at` 不重試（交給預取退避），5xx 可重試，`TCP_NODELAY`＋keep-alive 連線池（閒置 4 秒丟棄），基底路徑百分比編碼；`creationdate` 不當建立時間。一次寫入最多 512 MiB。真實伺服器：WSL 發行版 `Ubuntu` 的 Apache mod_dav（ADR 0035 §5A，unit `moemusic-webdav`，不 enable），測試 `crates/player-remote/tests/real_webdav.rs`（`MOE_WEBDAV_*` 環境變數，帳密只在 repo 外的憑證檔）；這台電腦 WSL 網路是 Consomme，測試經 stdio 轉送、效能數字不具代表性，改 mirrored 需使用者決定。尚待：Release GUI、平板實機、區網裝置、其他 WebDAV 伺服器。

- **讀取層**：`fs.rs`（`RemoteFs`、`RangeReader`；標籤讀取用 `RemoteByteSource`：16 KiB 區塊〔`METADATA_BLOCK_SIZE`〕預讀 2 塊，循序讀取以 `with_growing_read_ahead` 加倍到 1 MiB，同一首共用 `SharedRangeReader`；大型內嵌封面〔MP3 APIC、DSF ID3、M4A covr、Opus 註解〕仍需整段讀過，省的是往返次數，跳過封面的格式專用 reader 未實作）、`prefetch.rs`（`PrefetchReader` 背景往前讀 16 MiB；`Network` 錯誤保留已抓資料，依 `RETRY_BACKOFF` 0、1、2、4、8、10… 秒從原位置重抓，斷線 3 分鐘〔`RETRY_GIVE_UP`〕才放棄，非網路錯誤立即失敗，seek 中斷等待；讀取者等 3 秒以上印 `[remote] prefetch stall`）、`ReopeningFile`（網路錯誤時重開檔，大小變了不接；`lazy` 只在讀到未快取區塊才開檔）、`local.rs`（`DirFs` 測試用）。放棄時錯誤訊息以 `player_core::REMOTE_LINK_LOST`（「伺服器無法連線」）開頭：Windows `remote_input.rs` 讓解碼器結束時在該位置暫停並設 `AudioError::RemoteLinkLost`（不跳下一首，再按播放重載並 seek 回原位）；Android `nativeRead` 回 -2，`DsdDataSource` 丟 `RemoteLinkLostException`，播放器停在 error。真實伺服器斷線下尚未驗證。

- **磁碟快取**：`player-remote/src/cache.rs` 的 `CachedFile`：1 MiB 區塊存 UserData 的 `remote-cache/`，鍵為來源 id＋路徑＋大小＋修改時間的雜湊，先進先出；區塊整塊讀完才寫入。`DiskCache` 在 `remote-cache/meta/` 記住大小與修改時間（`put_meta`／`meta`），有記錄時開檔不連網、背景 `stat` 驗證。標籤、封面、歌詞的小讀取（`RemoteFiles::open`，1 MiB 預讀）優先讀快取（`CachedFile::read_only`，未命中不存入）。設定 → 音樂來源最下面「快取」：「遠端快取」（`remoteCacheLimitMb` 100–5,000 MB、步進 100、預設 1,000；`remote_cache_get`／`remote_cache_set_limit`／`remote_cache_clear`）與「專輯封面快取」（`coverCacheLimitMb` 預設 500；`album_artwork_cache_stats`／`album_artwork_cache_set_limit`／`album_artwork_cache_clear`，清除後視窗重新載入）；放開拉桿即儲存並套用。logcat `RustStdoutStderr` 的 `[remote] open … took N ms`、`[remote] cache <鍵>: …`、`[remote] cover stages: …`、`[remote] lyrics lookup …` 供診斷；卡住時不要重裝或重開 App，先取 `adb logcat -d | grep "\[remote\]"` 與 `/proc/net/tcp6`。

- **掃描**：`index.rs` 的 `RemoteMediaIndex`（4 執行緒列舉、16 執行緒讀標籤）；連不上為 `Unavailable`、帳密被拒為 `PermissionRevoked`（都不移除曲目），資料夾列不出為 `Incomplete`；隱藏、系統與 `.` 開頭項目略過。建立／修改時間直接取伺服器回傳值（資料夾來源用 `list`、播放清單來源用既有 `stat`；SMB 建立時間 `RemoteEntry.created_utc_100ns` → `RemoteEntry::created_ms` → `source_mappings.created_at_utc_ms`；WebDAV 無建立時間），既有曲目在下一次同步以 `persist_mapping` 補值（量測測試 `crates/player-remote/tests/scan_created_timing.rs`；其他量測：`scan_transfer.rs`、`scan_heavy.rs`、`smb_latency.rs`，皆 `--ignored`）。真實 SMB 上的列表顯示未驗證。

- **封面與歌詞**：Windows `find_artwork`／`find_lyrics`／`find_all_lyrics` 處理 Remote（內嵌、同資料夾 `cover.jpg`／`folder.jpg`、同名 `.lrc`）；Android 用 `src-tauri/src/remote_media.rs`。`remote_media::remote_cover_bytes` 全域限流（互動最多 3、批次 `as_bulk_cover_work` 最多 2）；專輯卡片對含遠端曲目的專輯最多看前 10 首（`CARD_REMOTE_COVER_CANDIDATES`），沒找到而還有更多曲目時不記為沒有封面。`remote_cover_lookup_may_have_failed`：可能是讀取失敗時不寫空選擇、`library_get_track_artwork` 回 `REMOTE_UNAVAILABLE`；`album_artwork_cache_mark_missing_for_track(trackId, definitive)`：格式不明或縮圖失敗傳 `definitive: true` 一律寫入，否則遠端曲目只在伺服器連得上（`track_may_be_unreachable_on_a_server` 為否）才寫入。

- **歌曲資訊與編輯**：`track_details.rs` 的 `TrackSource::Remote` 經 `fetch_remote` 以 `open_streaming` 取頭尾各 4 MiB（`REMOTE_END_BYTES`）寫入暫存 `moemusicplayer-remote-details/<uuid>/<原檔名>`（讀不出才整首，上限 1 GiB），`scratch_for` 保留 2 分鐘；頻譜圖整首下載；`TrackDetails.remote` 時停用「在資料夾中開啟」。儲存 `save_remote`：`release` 放開該檔串流 → `stat` → 整首下載並 `write_edit` → `RemoteFs::replace_file`（先傳 `.moetmp-<檔名>` 核對大小再改名；伺服器檔案與讀取時大小／修改時間不同即 `RemoteError::Changed`）→ 更新快取記錄。前端儲存前暫停正在播放的該曲，存完重新載入、seek 回原位置。真實 SMB 寫入測試 `MOE_SMB_WRITE_TEST=1`（`real_smb.rs`、`a_song_on_a_real_server_is_edited_while_a_stream_of_it_is_open`）。實機在真實歌曲上的儲存驗收待做。

- **遠端播放清單**：伺服器分頁「選擇播放清單」（`remote_browse_playlists`、`remote_playlist_add`）登記為 `SourceEntryKind::PlaylistFile { remote_account: Some(帳號 id), path: moeremote://… }`。同步 `remote_playlist_sync.rs` 的 `sync_remote_playlist_source`（Windows 與 Android 共用，開始時送「列舉中」並把進度交給 `on_progress`）以 `read_remote_playlist` 讀整個清單檔（上限 16 MiB），依內容雜湊與修改時間判斷變更，歌曲掃進清單來源自己（`RemoteMediaIndex::scan_paths`，讀不到的檔只讓掃描不完整）。路徑樣式由 `crates/player-remote/src/playlist_style.rs` 的 `PlaylistStyle` 統一處理讀寫（`detect`、`to_server_path`、`to_entry_text`，往返相同）：相對條目以清單資料夾為基準（沿用檔內固定的上爬層數 `fixed_ups`、`./`、分隔符多數決）；絕對條目（磁碟機、`/srv/…`、UNC、`file://`）記 `lead` 並以 `MapRule { drop, dropped, prefix }` 試到伺服器上有檔（`stat`，連續 3 個找不到就放棄）；百分比編碼依檔內慣例；無條目時預設相對、用 `/`。

- **收藏到伺服器清單**：收藏浮出面板列出遠端清單，勾選＝改寫伺服器檔（`crates/player-remote/src/playlist_edit.rs` 的 `edit_playlist_file`：寫入前才 `stat`＋讀取，保留編碼〔UTF-8 有無 BOM、`.m3u` 的 UTF-16〕、換行與未動到的行，非 UTF-8 拒絕；多數條目為絕對路徑卻無對應規則時拒絕寫入）。插入位置與本機收藏共用 `player_core::playlist::favorite_insertion_index`（比較函式 `player_db::natural_title_cmp`：接在同資料夾最後一條之後，否則往上找最近群組，否則放最後）；只有檔案本來有 `#EXTINF`（或全新空檔）才寫 `#EXTINF`；移除刪該條目與緊鄰的 `#EXTINF`。應用層 `src-tauri/src/remote_favorites.rs`（`favorites_get_playlists`／`favorites_set_membership`）：曲目須屬同一帳號（`server_path_of_track`），否則清單停用並給 `FavoritePlaylistDto.reason`（`notOnServer`／`noAccount`）；寫入後一律重跑 `sync_remote_playlist_source`。遠端清單不算「我的最愛」。真實伺服器寫入、Android 裝置實測未驗證。

- **單一備份（本機與遠端一致）**：每次改寫播放清單檔（遠端清單、本機收藏含 `MyLove.m3u8`）只保留一份改寫前版本「原檔名＋`.bak`」：新內容先寫 `.moetmp-<檔名>` 並核對 → 刪舊 `.bak` → 原檔改名 `.bak` → 暫存改名為原檔名，失敗就還原；讀取後被改動的檔案拒絕寫入且不產生備份。遠端 `RemoteFs::replace_file_keeping_backup`（`rename_file`：SMB `FileRenameInformation`、WebDAV `MOVE` 加 `Overwrite: F`、`DirFs` `std::fs::rename`），本機 `favorites.rs` 的 `write_keeping_backup`。`.bak` 不被當成播放清單（`is_playlist_name`）。`playlist_export` 不產生備份。SMB／WebDAV 上的備份順序未在真實伺服器驗證。

- **連不上的伺服器不得拖累其他工作**：`RemoteConnections` 每個帳號一把鎖，只在找帳號時鎖整張表，連線（最多 `CONNECT_TIMEOUT`）期間不擋其他帳號與本機工作；連線因網路失敗就記下時間，`UNREACHABLE_BACKOFF` 內該帳號的所有使用（封面、歌詞、播放、掃描）立即回 `Network` 錯誤，不各自再等一次逾時（原本全域鎖下每個呼叫依序各等約 10 秒）；編輯或刪除帳號（`forget`）清除記錄。會連伺服器的判斷（例如 `remote_cover_lookup_may_have_failed`）只在 `spawn_blocking` 執行，不在 Tauri 非同步執行緒，也不在持有資料庫連線時執行。既有規則不變：伺服器連不上時不把遠端封面記成「沒有封面」。

  補充：帳號的連線包在 `GuardedFs`，連線失敗或任何操作（含已快取連線上）以 `Network` 結束，就啟動該帳號的 `UNREACHABLE_BACKOFF`（`LinkHealth`），期間所有使用立即失敗；到期後換新連線，第一次成功清除記錄。封面與歌詞查找在第一個網路錯誤後停止（`is_remote_network_error`）。哪些算 `Network`：SMB `classify` 把 `smb::Error::TransportError` 內的 `IoError`（無路由、拒絕連線）、`NotConnected`、`Timeout`、`InvalidAddress`，以及主機名稱解析失敗（`Link::connect`）都歸 `Network`，成框錯誤等其餘傳輸錯誤為 `Protocol`；WebDAV `network()` 把非 TLS 的 reqwest 錯誤（連線、解析、逾時）都歸 `Network`。

- **新選的曲目取代仍在等伺服器的載入**：Android 播放 actor 在等載入確認時（`transact`）看到新的選曲／上下一首請求就放棄等待（`SUPERSEDED_MESSAGE`），`PlaybackService` 在載入尚未就緒時仍輪詢下一個指令；`moeremote` 開檔失敗回 -2 讓 Kotlin 不重試並顯示「伺服器無法連線」。前端把這個失敗（最新指令的錯誤，或快照 `lastError` 變成連不上的訊息）顯示成會自動消失的浮動通知「伺服器無法連線」（`App.svelte` 的 `showUnreachableNotice`、`remote-unreachable-notice.js`；同一次失敗 2 秒內只出現一次，被取代的指令不顯示，本機曲目的錯誤不會比對到）；原本只有 `.dock-error` 一行小字，貼在播放列下緣，手機上看不到。Windows 音訊 worker 為載入編號（`LoadCounter`），已被取代的載入不啟動，執行中的載入其 `PrefetchReader` 在 `with_cancel` 通知後結束讀取（只限該載入進行中，播放中的串流不受影響）。
