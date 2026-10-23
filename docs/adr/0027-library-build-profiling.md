# In-process profiling of the cold library build

Status: Accepted and implemented on 2026-10-22; measurement and the cold-build optimization on `D:\Music` recorded below

Decision date: 2026-10-22, Asia/Taipei (UTC+08:00)

## Context

We want to know where the time goes when a library is built from nothing: no SQLite database, then every file under `D:\Music` (5,412 tracks, 9 formats, 323 GB, on a SATA HDD) listed, tagged and stored. An earlier attempt watched the GUI process from outside (CPU, disk, DB/WAL sizes). It could not tell when the library was done, so nothing it sampled could be tied to a phase. Only the process knows when each phase starts and ends, so the timeline has to come from inside the process. External counters are only useful when they are lined up against that timeline.

## Decision

- `player_core::profile` is an opt-in JSON Lines event log with no new dependencies.
  - It is off by default. While it is off, every hook costs one relaxed atomic load, and per-file timing is not collected.
  - It turns on when `MOE_LIBRARY_PROFILE=<file.jsonl>` is set, for the GUI or any binary, or when a benchmark calls `profile::init(path)`.
  - Each line carries `ts` (UTC RFC 3339), `ts_local` (offset from `MOE_LIBRARY_PROFILE_UTC_OFFSET_MINUTES`, for example `+08:00`), `utc_ms`, a monotonic `mono_ms`, the pid, the thread and the event name.
  - We chose this over `tracing` plus `tracing-subscriber` or Chrome traces. Those would add a dependency and Cargo.lock churn, and a handful of phase events does not need spans.
- `SyncEngine` emits these events. Every sync gets them, in the GUI too.
  - `enumerate_done`: track count and bytes.
  - `compare_done`.
  - `tags_done`: wall time, threads, summed per-file time and effective parallelism, p50/p90/p99/max, totals per extension, and the 15 slowest files.
  - `assemble_done`, `persist_done`, `lyric_sidecars_done` and `sync_done`.
- `player-db` emits these events.
  - `db_open`: open, pragmas and migration.
  - `db_apply`: the single scan transaction split into begin and clear, `sync_seen_items`, `resolve_track_id`, `persist_metadata`, `persist_mapping`, reconcile and state, and commit, plus the WAL size.
- Completion is explicit.
  - The GUI's `library_sync` command calls `profile::mark_ready`. This writes a `library_ready` event and `<profile>.ready.json`.
  - The headless benchmark writes `library_ready` and `<out>/library_ready.json`.
  - An external monitor waits for that file instead of guessing from counters.
- The headless benchmark is `crates/player-platform-windows/examples/profile_cold_library.rs`.
  - It builds a new `library.sqlite` in an output directory outside the music root, and refuses an existing one.
  - It uses the production `WindowsMediaIndex`, `SyncEngine` and `Database`, without the GUI.
  - It reads only the music folder, and it never touches the running player or `UserData`.
- The runner is `scripts/profile-cold-library.ps1`.
  - It builds the example in release mode and runs it N times, each in a fresh directory.
  - During each run it samples the process's CPU time, working set and I/O counters with UTC timestamps into `samples.csv`.
  - It prints phase tables, a per-extension table and the slowest files, and writes `summary.json`.
  - Run 1 is labelled `first`, later runs `warm`. Caches are never flushed, and the machine is never rebooted.

## First measurement (2026-10-22 02:00 +08:00, release build, 16 logical CPUs, 4 tag threads)

These numbers are from 5 runs. The files were already in the OS file cache from earlier scans, so these runs measure CPU and SQLite cost, not HDD seek time. A truly cold HDD run needs a fresh boot or an uncached folder.

| Phase | Time (median of 5) | Share |
|---|---|---|
| DB open, schema and migrate | 31 ms | 2% |
| Directory enumeration (5,412 files, 8 `.lrc`) | 104 ms | 6.5% |
| Compare with DB | 51 ms | 3% |
| Tag read: 4 threads, effective parallelism 3.95, per-file p50 0.39 ms, p99 2.3 ms | 600 ms | 36% |
| Assemble | 12 ms | 1% |
| Persist: one transaction | 776 ms | 47% |
| …of which `persist_mapping` | 494 ms | 30% |
| …of which `persist_metadata` (tracks, ICU sort key index) | 117 ms | 7% |
| …of which commit | 98 ms | 6% |
| …of which `sync_seen_items` | 37 ms | 2% |
| `.lrc` sidecars | 2.5 ms | 0% |
| First 1,000-row page | 65 ms | 4% |
| **Total until `library_ready`** | **1.60–1.87 s** | |

Other figures:
- The DB is 16.9 MB with a 17.2 MB WAL.
- The process wrote about 300 MB (OS write counter) for a DB of about 34 MB. A 5,412-row transaction is larger than SQLite's default 2 MB page cache, so pages are probably spilled and rewritten.
- Logical reads were about 500 MB; tag parsing reads only headers.
- FLAC accounts for 5,012 files at 0.41 ms each. DSF (2.0 ms), M4A (1.3 ms) and Opus (5.7 ms) are the slow formats per file.

## Consequences

- To profile a GUI cold build, launch the player with `MOE_LIBRARY_PROFILE` set. The file then gets `app_start`, the `SyncEngine` and DB events, and `library_ready` when the sync finishes. That is a normal single launch, never a second instance.
- Every optimization is measured with the script before and after; the cold-build bulk mode below is the first.

## Optimization of the cached cold build (2026-10-22 02:10–02:20 +08:00)

Every step was measured as the median of 5 fresh builds of `D:\Music` with the OS file cache warm. Settings were switched per run with `MOE_LIBRARY_BULK` and `MOE_METADATA_READ_THREADS`. The before and after columns come from 2 interleaved rounds in one session.

| Phase (ms, median) | Before | After | Change |
|---|---|---|---|
| Enumerate | 110 | 109 | 0 |
| Tag read (4 threads) | 675 | 671 | 0 |
| Persist (one transaction) | 792 | 276 | −516 (−65%) |
| …`persist_mapping` | 507 | 39 | −468 |
| …`persist_metadata` | 119 | 34 | −85 |
| …`sync_seen_items` | 39 | 31 | −8 |
| …index rebuild + `foreign_key_check` | 0 | 51 | +51 |
| …commit | 87 | 88 | 0 |
| First page | 77 | 71 | −6 |
| **Total until `library_ready`** | **1,776** | **1,243** | **−533 (−30%)** |
| Process write bytes | 288 MB | 37 MB | −87% |
| DB + WAL | 32.5 MB | 31.4 MB | −3% (indexes built once are denser) |

Each setting was also measured on its own; the figure is the persist time, with 809 ms for none:

| Setting | Persist | Written |
|---|---|---|
| `fk` | 411 ms | 39 MB |
| `defer` | 732 ms | 207 MB |
| `cache` | 778 ms | |
| `sync` | 829 ms (no gain) | |
| `fk,defer` | 376 ms | |
| `fk,cache` | 361 ms | |
| `fk,defer,cache` | 261 ms | |
| `fk,defer,cache,sync` | 268 ms | |

### Decisions

- **Bulk build mode, in `BulkBuild` in `player-db`.** It applies when `source_mappings` is empty and the scan stores at least 256 rows, which is a cold build. Warm and partial syncs are unchanged. It does three things:
  - **Foreign keys.** It turns `foreign_keys` off for the one transaction, which can only be done outside a transaction. Before commit it runs `pragma_foreign_key_check` and rolls back if this build added violations. Violations already present in an old database are counted first and do not block the build.
  - **Deferred indexes.** It drops `source_mappings_track_id`, `source_mappings_locator_exact` and `tracks_sort_title` inside the transaction, and recreates them before commit from their own `sqlite_master` text. No per-row statement reads these three indexes. `source_mappings_locator_key` and the primary keys stay, because `resolve_track_id` reads them on every row.
  - **Page cache.** It sets `cache_size` to 64 MiB.
  - Everything is restored after the transaction on every path, success, error or cancel.
- **`synchronous=NORMAL` is rejected.** It gave no measurable gain (commit 97–99 ms either way), so the WAL keeps full durability.
- **No schema change, so no migration.** Slimming `source_mappings` was rejected. `source_item_id` and `locator_key` both hold the same hex-encoded UTF-16 path, about 300 bytes, and a standalone insert test put the gain from dropping one copy at about 56 ms, before deferral and FK-off. After them `persist_mapping` is only 39 ms. A change touching the 64 queries that use `locator_key`, plus an upgrade migration, is not worth at most a few tens of ms.
- **Tag thread count stays 4. `MOE_METADATA_READ_THREADS` (1–64) overrides it for tuning.** Per-file parse time grows with threads: about 0.43 ms at 4, 0.55 ms at 6, 0.73 ms at 8 and 1.2 ms at 16. So wall time improves little. Over 3 measurement sets (5 runs each), 6 and 8 threads were 0–25% faster than 4, and in the interleaved round within noise (4: 650–691 ms, 6: 613–661 ms, 8: 597–677 ms). A gain that does not reproduce, plus more concurrent seeks on an uncached HDD, does not justify a new default. Re-measure after a reboot on the HDD.
- **Correctness.** Compared with the build from before the change, all 5,412 `source_mappings` + `tracks` rows are equal on every column except the random ids. The schema text, the row counts of every table and `quick_check` are also equal, with 0 FK violations. A `player-db` test builds 300 rows each way and checks the same rows and order, that the indexes are back, `foreign_keys=1` and the original `cache_size`.

## Truly cold HDD build (2026-10-22 03:27 +08:00, first access after a reboot)

The run used the release example with 4 threads and bulk mode. `typeperf` sampled `PhysicalDisk(1 D:)` every second, in `%TEMP%\moe-cold-profile\reboot-20261022`.

| Phase | Cold | Warm (median of 3) |
|---|---|---|
| Enumerate | 5,851 ms | 100 ms |
| Tag read: per-file p50 / p90 / p99 / max 53 / 114 / 250 / 1,053 ms | 89,052 ms (93%) | 525 ms |
| Persist | 268 ms | 257 ms |
| **Total until `library_ready`** | **95,400 ms** | **1,060 ms** |

During tag reading the disk was 0.2% idle, with a queue of 3.5. It did 228 reads/s at 17.5 ms each but moved only 10.7 MB/s. The cold build is seek-bound: about 3.7 disk reads per track, not bytes.

## Where the reads go (trace of every file, warm cache)

`crates/player-track-metadata/examples/trace_tag_reads.rs` wraps the production `PathSource` and records every read and seek the parser makes. Set `TRACE_RANGES=1` to print the merged byte ranges per file.

| Format (files) | Opens | Read calls | Bytes | Distinct regions | Second region |
|---|---|---|---|---|---|
| FLAC (5,012) | 1 | 4.8 | 41 KB (19 KB unique) | 1.99 | Metadata after the skipped PICTURE block |
| MP3 (169) | 2 | 77 | 529 KB | 2.0 | ID3v1/APE probe at the file end, in 100% of files |
| M4A (92) | 1 | 58 | 1,050 KB | 2.47 | `moov` including `covr` |
| DSF (99) | 1 | 2 | 621 KB | 2.0 | ID3 at the end, including its picture |
| Opus (5) | 1 | 95 | 2.9 MB | 2.0 | OpusTags with a base64 picture, then the last page |
| WAV (28) | 2 | 8 | 31 KB | 2.8 | Trailing chunks |

- `scan_parse_options()` already sets `read_cover_art(false)`. FLAC pictures are seeked over, never read. For ID3v2, MP4 and OpusTags, lofty still reads the tag bytes, but skips decoding the picture.
- The process total of about 95 KB per track comes from the 365 non-FLAC files reading their tag area. A FLAC reads only about 19 KB.
- Every format already touches the disk in about 2 places. Fewer bytes therefore cannot remove seeks; only fewer regions or better I/O ordering can.

## A shared block cache was tested and rejected

`BlockCachedPathSource` was tried as the local tag source: one file handle per track and a 64 KiB block cache shared by every `open`, with reads of a block or more passed straight through. It was compared on the 40-files-per-format sample, on block counts for all files, and on warm runs.

- **Read calls** dropped (FLAC 4.8 → 2.1, MP3 77 → 8.7, M4A 58 → 15).
- **Distinct regions** were unchanged (FLAC 1.99 → 1.96). Those are what cost a seek.
- **Bytes read** rose from 513 MB to 962 MB.
- **Warm tag reading** was unchanged (514–524 ms against 508–541 ms).

It was removed. Reading through the FLAC picture to save the second seek would need the merge gap raised to 1 MiB. That only drops regions to 1.35 per file, adds about 400 KB per track, and on a queued HDD the transfer costs about as much as the seek it saves.

The second FLAC region is needed for the audio offset that the bitrate is computed from. The MP3 tail probe is needed for ID3v1-only files. Dropping either would change stored tags.

## Next levers (not implemented)

- **Order reads by physical location.** Use `FSCTL_GET_RETRIEVAL_POINTERS` for each file's first extent, falling back to the path.
- **Queue depth for an HDD.** Measure 4, 8 and 16 threads cold. Each truly cold run needs a reboot, or one disjoint subfolder set per setting after a single reboot. Flushing the standby list (RAMMap / EmptyStandbyList) needs administrator rights and the user's approval.

## 目前設計細節（自 AGENTS.md 移入）

以下是目前仍有效的實作細節；AGENTS.md 只保留摘要與指向本節的連結。

冷建庫效能（ADR 0027）：設 `MOE_LIBRARY_PROFILE=<檔案.jsonl>`（可加 `MOE_LIBRARY_PROFILE_UTC_OFFSET_MINUTES=480`）後寫出 `enumerate_done`、`compare_done`、`tags_done`、`db_open`、`db_apply`、`persist_done`、`sync_done`，GUI `library_sync` 完成時寫 `library_ready` 與 `<檔案>.ready.json`。無 GUI 基準：`scripts\profile-cold-library.ps1 -MusicRoot D:\Music -Runs 5`（`profile_cold_library` example，每輪在 `%TEMP%\moe-cold-profile\...` 建新 DB，不碰播放器與 UserData）。批次模式（`player-db` 的 `BulkBuild`）：`source_mappings` 為空且寫入 ≥ 256 列時，交易外關 `foreign_keys`、交易內刪除再重建 `source_mappings_track_id`／`source_mappings_locator_exact`／`tracks_sort_title`、以 `pragma_foreign_key_check` 確認（有違反就回滾）、`cache_size` 64 MiB，結束後恢復；`MOE_LIBRARY_BULK`、`MOE_METADATA_READ_THREADS` 只供量測。冷 HDD 的瓶頸是尋軌（共享區塊快取已評估放棄）；工具 `cargo run --release -p player-track-metadata --example trace_tag_reads -- D:\Music 40`（`TRACE_RANGES=1`）。下一步：GUI 冷建庫量測、依磁碟實體位置排序讀取、冷狀態量 HDD 執行緒數。
