# In-process profiling of the cold library build

Status: Accepted and implemented on 2026-10-22; first measurement on `D:\Music` recorded below

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
- Optimizations are proposed in AGENTS.md and the commit report, and are not implemented by this ADR. Each one should be re-measured with the script before and after.
