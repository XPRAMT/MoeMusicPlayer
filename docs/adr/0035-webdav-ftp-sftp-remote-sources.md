# WebDAV、FTP、SFTP 遠端來源

狀態：部分採納（2026-10-24 使用者決定：「先把 webdav 做完驗證完；sftp、ftps 之後有需要再做」）——P1 WebDAV 進行中；SFTP、FTPS 延後到有需要時；明文 FTP 不支援。

日期：2026-10-24（Asia/Taipei）

相關：ADR 0026（遠端來源，SMB／WebDAV）、ADR 0027（建庫效能）、ADR 0033（遺失檔案隱藏）

## 0. 決定（2026-10-24）

- **P1 WebDAV：進行中。** 實作與真實伺服器驗證見下方「0.1」。
- **SFTP、FTPS：延後**，有需要時再依本文 §2.2、§2.3、§3 的計畫做；本文其餘段落保留為屆時的計畫。
- **明文 FTP：不支援。** 理由：（1）不適合範圍讀取與平行讀取——每次 seek 都要 `REST` 加重開資料連線，播放與 16 執行緒讀標籤都會被往返次數拖垮；（2）`LIST` 格式沒有標準（Unix／DOS／各家 NAS 各異），檔名字元集不保證是 UTF-8；（3）被動模式的資料埠在 NAT、防火牆與手機網路下常出問題；（4）時間精度差（`LIST` 常只到分鐘，`MDTM` 不是每台都有），「讀取後被改動」的判斷不可靠；（5）沒有建立時間；（6）帳密明文傳送。主流 NAS（Synology、QNAP、TrueNAS、Unraid）都已提供 SMB、WebDAV 與 SFTP，FTP 沒有不可取代的用途。
- **建立時間**：WebDAV 的 `creationdate` 是選用屬性，而且不一定是真的建立時間。實測 Apache 2.4.58 mod_dav_fs（Ubuntu 24.04，ext4）回報的是 inode 的 **ctime（狀態改變時間）**：對檔案 `chmod` 之後 `creationdate` 就跟著變，`stat` 的 Birth 不變（例：Birth 2026-10-24 00:53:41、chmod 後 Change 00:56:44，PROPFIND 的 `creationdate` 是 `2026-10-23T16:56:54Z`，也就是 Change）。所以不把 `creationdate` 當作建立時間顯示或儲存：`PROPFIND` 不要求它，`RemoteEntry.created_utc_100ns` 一律 `None`，歌曲資訊面板的建立時間對 WebDAV 留空。

### 0.1 P1 WebDAV 的狀態

- 已補強（`crates/player-remote/src/webdav.rs`）：每個請求有自己的時限（`request_timeout`：10 秒＋每 64 KiB 一秒，上限 30 分鐘；不再是整個 client 共用 60 秒，所以上傳大檔不會被截斷），連線停住時約 10–14 秒回報 `Network`，**停住的讀取不再於 `read_at` 內重試**（只重試 2 秒內就失敗的，例如伺服器已關掉的 keep-alive 連線），交給預取的退避重試與 3 分鐘後的「伺服器無法連線」暫停（ADR 0026）；5xx 視為可重試的網路錯誤；`TCP_NODELAY`、TCP keep-alive、連線池（每台伺服器最多 32 條閒置連線、閒置 4 秒丟棄，小於 Apache 預設 `KeepAliveTimeout` 5 秒）；帳號的基底路徑也做百分比編碼（含 `#`、`%`、空白、中日文）；自簽憑證被拒時正確回報為 `RemoteError::Tls`（以前沿著錯誤鏈看不到 rustls 錯誤，回報成網路錯誤，釘選提示永遠不會出現）。
- 介面：新增遠端伺服器可選 WebDAV（`WEBDAV_ENTRY_VISIBLE = true`）；WebDAV 路徑旁有「選擇資料夾」（從伺服器 `/` 瀏覽，所在資料夾即基底路徑）；https 遇到不受信任的憑證時，測試連線、選擇資料夾或儲存前會跳出「信任這個伺服器的憑證？」並顯示 SHA-256 指紋，信任後帳號以 `TlsMode::Pinned` 儲存；已釘選的帳號憑證改變時不提示，照舊回報「伺服器的憑證與你先前接受的不同」。純 http 的行為不變。修正：編輯已釘選的帳號以前會把 `tls` 存回 `verify`、讓釘選消失。
- `settings.json` 遇到不認得的 `protocol`（例如之後版本的 `sftp`）不再整個載入失敗：`RemoteProtocol::Unsupported`（`#[serde(other)]`），帳號保留、列出但不能連線；再次儲存時寫成 `unsupported`（原名不保留）。
- 真實伺服器：Apache mod_dav 測試機已依 §5A 建好，測試 `crates/player-remote/tests/real_webdav.rs`（環境變數見該檔開頭）。本機 WSL 網路模式退回 Consomme（不是 mirrored），Windows 連不到 WSL 的埠，測試經使用者空間的 stdio 轉送程式進行，**效能數字只供參考**。
- 尚未驗證：Release GUI（CDP）、手機／平板、區網其他裝置、Nextcloud 等其他 WebDAV 伺服器、真實憑證更換（以錯誤的釘選值代替）。

## 1. 現況

- **SMB**：已完成，真實伺服器驗證過（掃描、播放、快取、編輯、播放清單寫回）。
- **WebDAV**：`crates/player-remote/src/webdav.rs` 已實作（reqwest blocking＋rustls、Basic／Digest、憑證釘選、PROPFIND、Range、PUT＋MOVE、`rename_file` 用 `MOVE Overwrite: F`），只有測試伺服器（`test_server.rs`）驗證；新增入口被 `src/lib/remote-protocols.js` 的 `WEBDAV_ENTRY_VISIBLE = false` 隱藏。既有 WebDAV 帳號仍可列出／編輯。
- **與協定無關、可原樣沿用的部分**（只依賴 `RemoteFs`／`RemoteFile` 兩個 trait）：
  - 掃描 `RemoteMediaIndex`（平行列舉、16 執行緒讀標籤、`Unavailable`／`PermissionRevoked`／`Incomplete` 分類、`.lrc` 旁檔收集、`scan_paths_with_progress`）；
  - 標籤讀取 `player-track-metadata`（`ByteSource`，`RangeReader` 的 16 KiB 區塊與成長式預讀）；
  - 磁碟快取 `cache.rs`（鍵含大小＋mtime）、預取 `prefetch.rs`（斷線退避重試、`REMOTE_LINK_LOST` 暫停）、`ReopeningFile`、`releasable`；
  - 播放清單同步與 `PlaylistStyle`、收藏寫回 `playlist_edit.rs` 與 `replace_file_keeping_backup`（`.bak`，trait 預設實作，只需 `replace_file`／`stat`／`remove_file`／`rename_file`）；
  - 歌曲資訊面板讀取與 `save_remote`、封面與歌詞（`remote_media.rs`、Windows `find_artwork` 等）；
  - Android 播放路徑（`android_dsd.rs` 的 `open_remote` 經 `RemoteContent`）；
  - 帳號加密 `secret.rs`、備份 `remote-accounts.sealed`、帳號層級 locator key（`remote-acct:`）與自動合併、來源範圍切換 `moe_in_scope`。
- **協定專屬、每個新協定要做的**：`RemoteFs` 實作（list／stat／open／replace_file／remove_file／rename_file／`read_ahead_blocks`）、連線池與時限、`remote_connections.rs` 依 `RemoteProtocol` 建立用戶端、`remote_sources.rs` 的測試連線／瀏覽、UI 表單。

結論：新協定的工作量集中在一個 `player-remote/src/<協定>.rs` 加表單，其餘不動。

## 2. 各協定如何對應 `RemoteFs`

### 2.1 WebDAV（已有）

- 列出：`PROPFIND Depth: 1`，取 `getcontentlength`、`getlastmodified`（RFC 1123）、`creationdate`（ISO 8601，多數伺服器有，Nextcloud／Apache mod_dav 都有）、`resourcetype`。`Depth: infinity` 多數伺服器停用，不用。
- 讀取：HTTP `Range: bytes=a-b`，keep-alive 連線池（reqwest 預設）。伺服器回 200 而非 206 時須拒絕或只讀需要的部分（要確認現有程式的處理）。
- 寫入：PUT 暫存檔 → 比對 `stat` → `MOVE Overwrite: T`。「讀取後被改動」以 mtime＋大小判斷；可加 `If-Match: <ETag>` 加強（可選）。
- 驗證：Basic 只允許在 https；Digest；自簽憑證以釘選指紋（`tls.rs`）。
- 路徑：百分比編碼 UTF-8；大小寫依伺服器。
- 待驗：真實 Nextcloud／Synology／Apache／rclone serve webdav 的 PROPFIND 回應差異（`href` 絕對／相對、尾斜線、時區）、大檔 PUT（目前整檔進記憶體，上限 512 MiB）。

### 2.2 SFTP

- 列出：`OPENDIR`＋`READDIR`，屬性有 size、mtime（秒級）、權限（可判斷目錄、符號連結）；SFTP v3 **沒有 ctime**（`created_ms` 回 `None`）。符號連結以 `STAT`（跟隨）判斷目標類型，遇循環以深度上限保護。
- 讀取：`READ handle offset len`，可在同一 channel 管線化多個未完成請求（建議每檔 4–8 個 32 KiB 請求在途），seek 不需重開檔，非常契合 `RemoteFile::read_at`。
- 連線模型：一條 SSH 連線，多個 SFTP channel（或一個 channel 多個 handle、以請求 id 多工）。16 執行緒標籤讀取可共用一條 session，建議 1 條 SSH 連線＋2–4 個 SFTP channel；伺服器 `MaxSessions`（OpenSSH 預設 10）是上限。
- 寫入：寫到 `.moetmp-` → 比對 `stat` → 有 `posix-rename@openssh.com` 擴充就原子取代；沒有時 v3 `RENAME` 不覆蓋，需先改名原檔為 `.bak`（正好符合 `replace_file_keeping_backup` 的順序），單純 `replace_file` 則為「刪除＋改名」的非原子退路。
- 驗證：密碼、鍵盤互動（多數伺服器把密碼當 keyboard-interactive）、私鑰（Ed25519／RSA／ECDSA，可有密語）。**主機金鑰 TOFU**：第一次連線顯示指紋（SHA-256 base64，與 `ssh-keygen -lf` 一致）請使用者確認，存進帳號；之後不符就拒絕並明確提示「主機金鑰已改變」。
- 路徑：UTF-8 位元組（OpenSSH 不轉碼），區分大小寫；`.` 開頭隱藏檔沿用既有略過規則。根目錄可用 `REALPATH(".")` 得家目錄作為預設。

### 2.3 FTP／FTPS

- 列出：優先 `MLSD`（RFC 3659，機器可讀，含 `size`、`modify`（UTC）、`type`；部分伺服器有 `create`），不支援才退 `LIST`（Unix `ls -l` 與 IIS／DOS 格式需解析；日期無年份或只到分鐘、時區未知）。LIST 模式下對需要精確值的檔案以 `SIZE`、`MDTM` 補查（每檔多 2 次往返，只在掃描新檔或比對時用）。一般 **沒有 ctime**。
- 讀取：`TYPE I`＋`REST offset`＋`RETR`；一條控制連線一次只能有一個資料傳輸，**每次 seek 要中止（`ABOR`）並開新的資料連線**（被動模式 `EPSV`／`PASV`）。對策：`RemoteFile` 保持一條串流並在「往前小跳」時讀掉丟棄，大跳才重開；標籤讀取（頭＋尾兩段）每檔約 2 次資料連線，FTPS 每次資料連線還要 TLS 交握（可用 session resumption，部分伺服器強制）。
- 連線模型：每個並行讀取需要自己的控制連線。連線池大小建議 4（多數伺服器每 IP 限 2–10 條），所以 FTP 掃描的並行度要降，`metadata_read_threads` 依協定回報。
- 寫入：`STOR .moetmp-x` → `SIZE` 核對 → `RNFR`/`RNTO`。`RNTO` 覆蓋既有檔的行為依伺服器（vsftpd 會覆蓋、部分 Windows 伺服器拒絕），所以 `replace_file` 先改名原檔再改名暫存檔、失敗還原；「讀取後被改動」以 `MDTM`＋`SIZE` 判斷（秒級精度，風險可接受）。
- 驗證與安全：明文 FTP 帳密明文傳輸；FTPS 顯式（`AUTH TLS`，埠 21）與隱式（埠 990），資料連線 `PROT P`；自簽憑證沿用 WebDAV 的指紋釘選。
- 字元集：送 `OPTS UTF8 ON`（`FEAT` 有 UTF8 才送）；不支援時需讓使用者選伺服器編碼（Big5、Shift_JIS、GBK…），以 `encoding_rs` 轉換（MIT/Apache）。大小寫依伺服器。

### 2.4 時限與斷線偵測（三者共用 SMB 經驗）

- 每個伺服器呼叫都包上外部時限（`bounded` 模式）：列出／stat／開檔 10 秒，讀取「有位元組就繼續等、上限 120 秒」，寫入 30 秒。
- 停滯探測：1 秒無位元組就送小請求（SFTP `STAT .`、FTP `NOOP` 於控制連線——注意傳輸中 FTP 控制連線不能送，故只看資料連線位元組、2 秒判斷；WebDAV 用 `OPTIONS`／`HEAD`），再 1 秒無回應判定斷線並丟棄連線。
- 斷線時沿用 `prefetch.rs` 的退避重試與 `REMOTE_LINK_LOST` 暫停，不跳下一首。
- 傳輸層：所有自建 TCP 一律 `TCP_NODELAY`，並確認 crate 不把標頭與資料分兩次 `write`（SMB 的 45 ms 教訓）。reqwest／hyper 預設已設 nodelay；russh、suppaftp 須實測確認。

### 2.5 效能預期與量測

| | 每首標籤讀取往返 | 主要風險 |
|---|---|---|
| SMB（現況） | 約 2 次、32 KiB | 已解決 |
| SFTP | open＋2 讀＋close ≈ 4，可管線化 | SSH 視窗大小、加密 CPU（Android） |
| WebDAV | 2 個 Range GET（HTTP keep-alive） | 伺服器每請求開銷、PROPFIND 大目錄慢 |
| FTP | 2 次資料連線（各含 PASV＋REST＋RETR＋ABOR，FTPS 加 TLS 交握） | 往返最多、每 IP 連線上限、ABOR 實作不一 |

量測計畫：把 `crates/player-remote/tests/scan_transfer.rs`（每首位元組與請求數）、`smb_latency.rs`（stat／開檔延遲、閒置間隔、nodelay）、`scan_heavy.rs`（大封面檔逐請求紀錄）泛化為以 `RemoteFs` 為參數的共用量測（`MOE_REMOTE_PROTOCOL` 選協定），每個協定都跑同一組並寫進本 ADR 的結果表。目標：SFTP 與 WebDAV 在 LAN 上 5,000 首掃描與 SMB 同量級（≤ 30 秒）；FTP 允許較慢。

### 2.6 Android

所有協定都在 Rust 內，經 `RemoteContent` 與 JNI 給 Media3，Kotlin 不需改。每個候選 crate 須實際 `cargo build --target aarch64-linux-android`（用專案既有的 Android 建置），重點是**不得引入 OpenSSL／C 依賴**；ring 已在專案內可為 Android 編譯。

## 3. Crate 候選

> 授權資訊：本機 cargo registry 沒有這些 crate 的原始碼，無法離線核對；以下依 crates.io／GitHub 的公開資訊（撰寫時記憶），**採納前須在 `cargo add` 後以 `Cargo.toml` 的 `license` 欄位與 `cargo deny`（或 `cargo license`）再確認一次**，尤其是傳遞依賴。

| 協定 | 候選 | 授權 | 型態 | TLS／加密 | Android | 評估 |
|---|---|---|---|---|---|---|
| WebDAV | 既有 `webdav.rs`（reqwest 0.12＋rustls） | MIT/Apache（reqwest） | blocking | rustls（已用） | 已可編譯 | **推薦：沿用** |
| SFTP | `russh` ＋ `russh-sftp` | Apache-2.0 | async（tokio） | 純 Rust（可選 ring／aws-lc-rs 後端） | 預期可行（純 Rust；須選 ring 後端以免 aws-lc 的 C/cmake） | **推薦** |
| SFTP | `ssh2`（libssh2 綁定） | ssh2：MIT/Apache；libssh2：BSD-3；需 OpenSSL（Apache-2.0，3.x）或 mbedTLS | blocking | OpenSSL | 需交叉編譯 OpenSSL＋libssh2，痛苦且增大 APK | 不推薦 |
| FTP/FTPS | `suppaftp` | MIT（須核對；GitHub 標示為 MIT） | 同時提供 sync 與 async | 可選 `rustls` feature（亦有 native-tls） | 純 Rust＋rustls 預期可行 | **推薦**（只開 sync＋rustls） |

補充：
- russh 是 async，`RemoteFs` 是 blocking。做法：在 `player-remote` 內建一個專用 tokio runtime（單一背景執行緒或小型 multi-thread），`RemoteFile::read_at` 以 `block_on`／channel 呼叫；斷線偵測直接在 async 端用 `tokio::time::timeout`。專案 src-tauri 已有 tokio，二進位增量主要是 russh 本身與其加密套件（估計 1–2 MB，未量測）。
- russh-sftp 的維護較 russh 本身弱，要確認 `posix-rename`、`READ` 管線化 API 是否夠用；不足時可在 russh channel 上自寫精簡 SFTP v3 用戶端（約 800–1,200 行，協定簡單）。
- suppaftp 的 LIST 解析器內建（Unix／DOS），MLSD 支援；要確認 `ABOR`、`REST`、`PROT P` 資料連線 TLS 重用的行為。
- 全部候選都不含 GPL／AGPL，符合專案授權規則。

## 4. 介面與設定

- **協定選擇器**：`WEBDAV_ENTRY_VISIBLE` 改為依協定的可見清單（例 `VISIBLE_PROTOCOLS = ['smb','webdav','sftp','ftp']`），表單標題「新增 X 伺服器」。
- **表單欄位**（共用：名稱、主機、埠、使用者、密碼、根目錄、只在 Wi-Fi）：
  - WebDAV：URL 或主機＋路徑、http／https（http 時警告密碼明文）、預設埠 443／80；
  - SFTP：預設埠 22；驗證方式（密碼／私鑰）、私鑰檔（Windows 檔案對話框、Android 文件選擇器；**私鑰內容匯入後以 `secret.rs` 加密存入帳號，不保留檔案路徑**）、私鑰密語；
  - FTP：模式（FTPS 顯式／FTPS 隱式／明文 FTP），預設埠 21／990；被動模式固定；字元集（自動 UTF-8／手選）。
- **根目錄選擇器**：沿用 `remote_browse_draft` 的資料夾瀏覽器；SFTP 起點為家目錄、FTP 為登入後的目錄、WebDAV 為 URL 路徑；任何一層都可選（沒有 SMB「共用資料夾」那一層的限制）。
- **信任提示**：主機金鑰（SFTP）與未受信任憑證（WebDAV／FTPS）共用一個 `.floating-surface` 面板：顯示主機、SHA-256 指紋、「信任並繼續」「取消」；指紋改變時改為警告樣式、預設拒絕。
- **i18n**：所有新字串走 `tr()`，補 zh-CN（腳本產生）、ja、en。
- **settings.json `remoteAccounts`**：擴充 `RemoteProtocol` 列舉（`sftp`、`ftp`），新增可選欄位：`ftpMode`（`explicitTls`／`implicitTls`／`plain`）、`ftpEncoding`、`sftpAuth`（`password`／`key`）、`sftpKey`（加密）、`sftpKeyPassphrase`（加密）、`hostKeyFingerprint`、沿用 `tls` 的憑證釘選。全部 `#[serde(default)]`，**不需遷移**；舊版程式讀到未知協定要能安全略過該帳號（需確認目前反序列化是否會整份失敗，若會，先在一個版本加相容處理）。備份 sealed 檔格式同步包含新欄位。

## 5. 分期與工作量（粗估）

1. **P1 WebDAV 驗證與重新開放（2–4 天）**：以 Nextcloud、Synology、rclone serve webdav 三種伺服器跑掃描／播放／編輯／收藏寫回；修 PROPFIND 差異、200 vs 206、大檔 PUT 串流化；加斷線探測；開放入口。交付：WebDAV 正式可用。
2. **P2 SFTP（6–10 天）**：russh 整合與 runtime 橋接、RemoteFs 實作、主機金鑰 TOFU UI、私鑰匯入、Android 建置與平板實測、量測。交付：SFTP 完整功能。
3. **P3 FTPS／FTP（6–9 天）**：suppaftp＋rustls、MLSD／LIST、連線池、seek 重開策略、字元集、憑證釘選、量測。交付：FTPS（與可選的明文 FTP）。
4. **P4 共用量測工具（1–2 天，可併入 P1）**：泛化 `scan_transfer`／`smb_latency`／`scan_heavy`。

**測試**：
- 單元：每協定以本機測試伺服器在測試中啟動——WebDAV 用既有 `test_server.rs`；SFTP 可用 russh 自己的 server 端組一個以資料夾為後端的測試伺服器（`testing` feature）；FTP 用純 Rust 的 `libunftp`（Apache-2.0，須核對）做測試伺服器，皆不需網路。涵蓋：列出、Range 讀取與 seek、取代與拒絕過期取代、`.bak` 順序、斷線探測（中途關 socket）、TOFU 指紋不符。
- 真實伺服器（`--ignored`，缺環境變數即略過）：`MOE_WEBDAV_URL/USER/PASS`、`MOE_SFTP_HOST/PORT/USER/PASS/KEY`、`MOE_FTP_HOST/PORT/USER/PASS/MODE`；寫入測試另需 `MOE_*_WRITE_TEST=1`，只動自己建立的檔案。可沿用 WSL Ubuntu 測試機（openssh-server、vsftpd、Apache mod_dav）。
- 實機：Windows Release CDP 與平板 23046RP50C。

## 5A. WSL 真實伺服器測試環境（計畫，尚未安裝）

比照既有 Samba 測試分享（AGENTS.md「區網測試分享（2026-10-21）」：WSL Ubuntu 24.04 mirrored 網路、`/etc/samba/moemusic-test.conf`、唯讀 `moemusic-test`、帳號 `moe_test`、TCP 1445、只允許 192.168.1.0/24、停用 guest）。以下指令需 sudo，由主工作階段與使用者處理密碼後執行；本次未執行。

### 共用

- 目錄：唯讀音樂 `/srv/moemusic-test`（與 Samba 共用同一份三首測試歌，不複製）；可寫暫存 `/srv/moemusic-scratch`（擁有者 `moe_test`，寫回、播放清單 `.bak`、歌曲資訊儲存測試用，測試只動自己建立的檔案，可隨時清空）。WebDAV／FTP／SFTP 各以唯讀綁定看到 `music/` 與可寫 `scratch/`：
  `sudo mkdir -p /srv/moemusic-scratch && sudo chown moe_test:moe_test /srv/moemusic-scratch`
- 帳號：沿用 `moe_test`（系統帳號，`/usr/sbin/nologin` 殼層），各協定密碼可不同；WebDAV 另有自己的 htpasswd 檔。
- 埠：mirrored 網路下 WSL 監聽的埠就是 Windows 主機的埠，須避開 Windows 自身（21、22、80、443、445、990 可能被 IIS／OpenSSH Server／系統占用）與 Windows 保留範圍（先 `netsh int ipv4 show excludedportrange protocol=tcp` 確認）。提案：WebDAV https **18443**、FTPS 控制 **2121**（顯式）／**2990**（隱式）、被動資料 **40000–40031**、SFTP **2222**；與 Samba 的 1445 一樣都在 1024 以上、不與系統服務衝突。
- 存取限制：每個服務各自設定只允許 `192.168.1.0/24`（與 127.0.0.1），另在 WSL 內以 `ufw` 或 nftables 規則加一層（可選）；全部停用匿名。Windows 防火牆是否需放行由使用者決定（mirrored 模式下 Hyper-V 防火牆預設規則可能擋入站）。
- 憑證：自簽一張給 WebDAV 與 FTPS 共用，用來測試憑證釘選面板：
  `sudo mkdir -p /etc/moemusic-test && sudo openssl req -x509 -newkey rsa:3072 -nodes -days 3650 -subj "/CN=moemusic-test" -addext "subjectAltName=IP:192.168.1.<WSL/主機 IP>,DNS:localhost" -keyout /etc/moemusic-test/tls.key -out /etc/moemusic-test/tls.crt`
  另產生第二張（`tls-alt.*`）用來測試「憑證改變」警告。
- 啟停：Ubuntu 24.04 WSL 預設啟用 systemd，每個服務做成獨立 unit（`moemusic-webdav.service`、`moemusic-ftp.service`、`moemusic-sftp.service`），**不 enable**（不開機自動啟動），測試時 `sudo systemctl start …`、結束 `stop`；不修改系統預設的 apache2／vsftpd／ssh 服務（Ubuntu 套件安裝後會自動啟用預設服務，安裝後立即 `sudo systemctl disable --now apache2 vsftpd ssh`——若使用者本來就在用 ssh 則不關，改確認我們的實例不影響它）。

### WebDAV：Apache httpd ＋ mod_dav

**已安裝（2026-10-24）**，在 WSL 發行版 `Ubuntu`（24.04；預設發行版是 Debian，兩者同一個 VM、網路相同）：設定 `/etc/moemusic-test/webdav/httpd.conf`、unit `moemusic-webdav.service`（不 enable）、https 18443、自簽憑證 `/etc/moemusic-test/tls.crt`（另有 `tls-alt.*`）；`/music`（Basic，唯讀：只允許 GET／HEAD／OPTIONS／PROPFIND）、`/scratch`（Basic，可寫，`/srv/moemusic-scratch`，擁有者 www-data）、`/digest`（Digest，唯讀，同 `/music`）；只允許 192.168.1.0/24、127.0.0.1、::1，無匿名。與下方計畫的差異：`DavLockDB` 放 `/var/lib/moemusic-webdav/`（`/run/moemusic-webdav/` 屬 root，www-data 寫不進去，PUT 會 500）；為讀取 Samba 的唯讀音樂（`root:moe_test`、0640），把 `www-data` 加入 `moe_test` 群組，不改那些檔案的權限；未載入 autoindex，所以對資料夾 GET 是 404（PROPFIND 正常）。測試帳號密碼只在 repo 外的憑證檔。WSL 的 `127.0.0.1` 在這個網路模式下連不到自己的服務，WSL 內請用 192.168.1.200；Windows 端經 stdio 轉送（見 0.1）。

選 Apache 而非 nginx：nginx 內建 `ngx_http_dav_module` 不支援 PROPFIND（需第三方 `dav-ext`），LOCK 也不完整；Apache mod_dav_fs 是最常見的參考實作，PROPFIND 有 `creationdate`、支援 MOVE Overwrite、Range，且 Basic／Digest 都有現成模組。

- 安裝：`sudo apt install apache2 apache2-utils`
- 隔離：獨立 `ServerRoot`／設定 `/etc/moemusic-test/webdav/httpd.conf`，以 `apache2 -f … -k start` 由 unit 啟動，PID、log、DavLockDB 都在 `/run/moemusic-webdav/`、`/var/log/moemusic-webdav/`，不使用 `/etc/apache2/sites-*`。
- 內容：`Listen 18443`、`SSLEngine on`（上述自簽憑證）、`Alias /music /srv/moemusic-test`（`Dav On`＋`<LimitExcept GET HEAD OPTIONS PROPFIND> Require all denied</LimitExcept>`）、`Alias /scratch /srv/moemusic-scratch`（`Dav On` 可寫）、`AuthType Basic`（另一個 location 用 `AuthType Digest` 測 Digest）、`Require ip 192.168.1.0/24 127.0.0.1` 與 `Require valid-user` 同時成立。Apache 以 `www-data` 執行，scratch 目錄需給 `www-data` 寫權限（或 unit 以 `moe_test` 身分執行）。
- 帳號：`sudo htpasswd -c /etc/moemusic-test/webdav/htpasswd moe_test`、`htdigest` 同理。
- 另可加 `Listen 18080` 純 http 實例，只在使用者決定允許 http 時才測。

### FTP／FTPS：vsftpd

- 安裝：`sudo apt install vsftpd`，立即停用預設服務。
- 隔離：`/etc/moemusic-test/vsftpd-explicit.conf` 與 `vsftpd-implicit.conf`，unit 以 `vsftpd <conf>` 啟動（獨立行程，`listen=YES`、`background=NO`）。
- 重點設定：`listen_port=2121`（隱式檔 `2990`＋`implicit_ssl=YES`）、`anonymous_enable=NO`、`local_enable=YES`、`write_enable=YES`、`chroot_local_user=YES`＋`local_root=/srv/moemusic-ftp-root`（內含 `music` 唯讀綁定掛載與 `scratch`）、`allow_writeable_chroot=NO`（root 目錄本身唯讀）、`userlist_enable=YES`＋`userlist_deny=NO`（只允許 `moe_test`）、`pasv_min_port=40000`、`pasv_max_port=40031`、`ssl_enable=YES`、`rsa_cert_file`/`rsa_private_key_file`＝上述憑證、`force_local_logins_ssl=YES`/`force_local_data_ssl=YES`（FTPS）、`require_ssl_reuse=NO`（先關，另測開啟時的相容性）、`utf8_filesystem=YES`、`tcp_wrappers` 不可用所以 IP 限制靠 `pam` 的 `pam_access`（`/etc/moemusic-test/access.conf` 只允許 192.168.1.0/24）或 nftables。
- 明文 FTP：另一份 `vsftpd-plain.conf`（埠 2120，`ssl_enable=NO`），只在使用者決定支援明文 FTP 時才啟動。
- 唯讀綁定：`sudo mkdir -p /srv/moemusic-ftp-root/{music,scratch}`，`mount --bind -o ro /srv/moemusic-test /srv/moemusic-ftp-root/music`（由 unit 的 `ExecStartPre` 做、`ExecStopPost` 解除）。

### SFTP：OpenSSH 獨立 sshd 實例

- 安裝：`sudo apt install openssh-server`（若使用者 WSL 已有 ssh，不動它）。
- 隔離：獨立設定 `/etc/moemusic-test/sshd_config`，unit 以 `/usr/sbin/sshd -D -f /etc/moemusic-test/sshd_config` 啟動，`PidFile /run/moemusic-sftp.pid`，主機金鑰獨立：
  `sudo ssh-keygen -t ed25519 -N "" -f /etc/moemusic-test/ssh_host_ed25519_key`（另產 `ssh_host_rsa_key`；並備一把 `..._alt` 用來測試「主機金鑰已改變」：替換後重啟）。
- 設定：`Port 2222`、`ListenAddress 0.0.0.0`、`PermitRootLogin no`、`AllowUsers moe_test@192.168.1.0/24 moe_test@127.0.0.1`、`PasswordAuthentication yes`、`PubkeyAuthentication yes`（`AuthorizedKeysFile /etc/moemusic-test/authorized_keys/%u`）、`KbdInteractiveAuthentication yes`（測試 keyboard-interactive）、`Subsystem sftp internal-sftp`、`ForceCommand internal-sftp`、`ChrootDirectory /srv/moemusic-sftp-root`（root 擁有、內含 `music` 唯讀綁定與可寫 `scratch`）、`AllowTcpForwarding no`、`X11Forwarding no`。
- 測試私鑰：在 WSL 產生 Ed25519（無密語）與 RSA（有密語）兩把，私鑰放在 repo 之外的本機憑證檔目錄，公鑰加入 authorized_keys。

### 真實伺服器 ignored 測試的環境變數

憑證一律來自 repo 外的本機檔案（例如 `%USERPROFILE%\Downloads\moemusic-test-servers.txt`，比照 `smb.txt`），由測試前的腳本讀入環境變數，**不提交、不寫入日誌**（測試輸出只印主機與埠，不印密碼；錯誤訊息須過濾）。缺變數即略過。

- WebDAV：`MOE_WEBDAV_URL`（例 `https://192.168.1.x:18443/music`）、`MOE_WEBDAV_SCRATCH_URL`、`MOE_WEBDAV_USER`、`MOE_WEBDAV_PASS`、`MOE_WEBDAV_AUTH`（basic／digest）、`MOE_WEBDAV_CERT_SHA256`（預期釘選指紋）。
- FTP：`MOE_FTP_HOST`、`MOE_FTP_PORT`、`MOE_FTP_MODE`（explicit／implicit／plain）、`MOE_FTP_USER`、`MOE_FTP_PASS`、`MOE_FTP_MUSIC_DIR`、`MOE_FTP_SCRATCH_DIR`、`MOE_FTP_CERT_SHA256`。
- SFTP：`MOE_SFTP_HOST`、`MOE_SFTP_PORT`、`MOE_SFTP_USER`、`MOE_SFTP_PASS` 或 `MOE_SFTP_KEY`（私鑰檔路徑）＋`MOE_SFTP_KEY_PASSPHRASE`、`MOE_SFTP_HOST_KEY_SHA256`、`MOE_SFTP_MUSIC_DIR`、`MOE_SFTP_SCRATCH_DIR`。
- 共用：`MOE_<協定>_WRITE_TEST=1` 才跑寫入測試；`MOE_<協定>_RESTART_CMD`（例如 `wsl -u root systemctl restart moemusic-sftp`）才跑斷線重連測試，因需要 sudo，預設不跑，由主工作階段手動觸發或事先設定 sudoers 只允許這三個 unit 的 start/stop/restart（由使用者決定）。

### 測試矩陣（每個協定都跑）

| 項目 | 內容 | 判準 |
|---|---|---|
| list | 列出 `music/`、子資料夾、空資料夾、隱藏檔 | 名稱、大小、mtime 與 WSL 內 `stat` 一致；隱藏檔略過；ctime 有則一致、無則 `None` |
| stat | 檔案、資料夾、不存在路徑 | NotFound 正確分類；不存在 vs 連不上區分 |
| 範圍讀取 | 開頭、中間、檔尾、跨區塊、超出檔尾、隨機 seek 100 次 | 位元組與本機檔 SHA-256 逐段相同 |
| 掃描 | `RemoteMediaIndex` 掃測試曲庫 | 三首曲目標籤、時長、內嵌歌詞、`.lrc` 旁檔與 SMB 掃描結果相同；請求數／位元組記錄 |
| 串流播放 | `open_streaming`＋prefetch 讀整首、seek | 解碼結果與本機逐樣本相同（沿用 SMB 的 remote decode 測試） |
| 寫入／取代 | 在 `scratch/` 建立、`replace_file`、過期取代（中途由 WSL 端 `touch` 改檔） | 成功取代；過期回 `Changed` 且原檔不變、暫存檔清除 |
| `.bak` 流程 | `replace_file_keeping_backup` 兩次、模擬第 3／4 步失敗 | 只留一份 `.bak`；失敗時原檔還原 |
| 播放清單寫回與歌曲資訊儲存 | `edit_playlist_file`、`save_remote` 對 scratch 內的檔案 | 與 DirFs 測試相同的逐位元組結果 |
| 斷線重連 | 讀取中 `systemctl stop`、等 5 秒再 `start`；以及 `restart` | 約 2 秒偵測斷線；重啟後 prefetch 從原位置續讀、不缺不重；停止超過時限回 `REMOTE_LINK_LOST` |
| 信任提示 | 換憑證／換主機金鑰後連線 | 指紋不符被拒且錯誤可辨識 |
| 唯讀保護 | 對 `music/` 寫入 | 伺服器拒絕，錯誤分類為權限 |
| 延遲量測 | stat、開檔＋讀 64 B、閒置 0–1000 ms 後 stat，單／多執行緒（泛化 `smb_latency.rs`） | 記錄 p50／p95；確認無 ~40 ms Nagle 尖峰 |

## 6. 待使用者決定（與建議分開）

**使用者的決定**：
1. 分期順序（建議 WebDAV → SFTP → FTPS）以及是否三個都做。
2. 是否支援**明文 FTP**，或只支援 FTPS（明文會傳送明碼密碼）。
3. SFTP 是否支援**私鑰檔與密語**，或只做密碼。
4. WebDAV 是否允許 **http（非 https）**。
5. FTP 非 UTF-8 伺服器是否要支援手選字元集。
6. 主機金鑰／憑證改變時，是否允許使用者「仍然信任新的」，或一律要求刪除帳號重建。
8. WSL 測試服務的埠（提案 18443／2121／2990／2222／40000–40031）、是否在 Windows 防火牆放行，以及是否設定只限這三個 unit 的 sudoers 以便自動化重連測試。
7. 是否接受 SFTP 引入 async（tokio）依賴與約 1–2 MB 的二進位增量。

**建議（非決定）**：WebDAV 沿用既有實作；SFTP 用 russh＋russh-sftp（ring 後端），不足時自寫 SFTP 層；FTP 用 suppaftp（sync＋rustls）；FTP 掃描並行度降為 4；不使用 ssh2／OpenSSL。

## 參考

- RFC 4918（WebDAV）、RFC 7233（HTTP Range）、RFC 959／3659（FTP、MLSD/MDTM/SIZE/REST）、RFC 4217（FTPS）、draft-ietf-secsh-filexfer-02（SFTP v3）、OpenSSH PROTOCOL（`posix-rename@openssh.com`）。
- crates.io：`suppaftp`、`russh`、`russh-sftp`、`ssh2`、`libssh2-sys`、`libunftp`（授權待離線核對，見第 3 節）。
- 本專案：ADR 0026、`crates/player-remote/src/{fs,smb,smb_transport,webdav,prefetch,cache,index,account}.rs`。
