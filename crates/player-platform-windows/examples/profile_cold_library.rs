//! Headless cold library build profile (ADR 0027).
//!
//! Builds a brand-new SQLite library from one music folder in an isolated output directory, with in-process phase
//! profiling on, without the GUI and without touching the user's library database. The music folder is only read.
//!
//! cargo run --release -p player-platform-windows --example profile_cold_library -- <music-root> <new-output-dir>
//!
//! Writes `<out>/profile.jsonl` (phase events, ends with `library_ready`), `<out>/library_ready.json` (the completion
//! marker external monitors wait for) and `<out>/library.sqlite`.

use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use player_core::{
    profile::{self, field, ms, str_field},
    ListTracksQuery, MediaLocator, MediaSourceKind, SourceScanState, SyncEngine,
};
use player_db::Database;
use player_platform_windows::WindowsMediaIndex;

const USAGE: &str = "usage: profile_cold_library <music-root> <new-output-dir>";

fn main() -> Result<(), Box<dyn Error>> {
    let process_started = Instant::now();
    let mut args = env::args_os().skip(1);
    let root_path = PathBuf::from(args.next().ok_or(USAGE)?);
    let out_dir = PathBuf::from(args.next().ok_or(USAGE)?);
    if args.next().is_some() {
        return Err(USAGE.into());
    }
    let canonical_root = fs::canonicalize(&root_path)?;
    let out_dir = std::path::absolute(&out_dir)?;
    if out_dir.starts_with(&canonical_root) || out_dir.starts_with(&root_path) {
        return Err("the output directory must be outside the music root".into());
    }
    let database_path = out_dir.join("library.sqlite");
    if database_path.exists() {
        return Err("refusing to reuse an existing database; give a new output directory".into());
    }
    fs::create_dir_all(&out_dir)?;
    let profile_path = out_dir.join("profile.jsonl");
    if !profile::init(&profile_path) && profile::output_path().is_none() {
        return Err("could not open the profile file".into());
    }
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    profile::event(
        "process_start",
        &[
            str_field("root", &root_path.display().to_string()),
            str_field("database", &database_path.display().to_string()),
            field("logical_cpus", cpus),
            field("metadata_read_threads", player_core::sync::METADATA_READ_THREADS),
            str_field("profile", if cfg!(debug_assertions) { "debug" } else { "release" }),
        ],
    );

    let mut database = Database::open(&database_path)?;
    let root = database.add_library_root(
        MediaSourceKind::WindowsFilesystem,
        "cold profile",
        MediaLocator::FileSystem(root_path.clone()),
    )?;
    profile::event("root_added", &[]);

    let synced_at_utc_ms =
        i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let mut index = WindowsMediaIndex::new();
    let sync_started = Instant::now();
    let mut progress_callbacks = 0u64;
    let report = SyncEngine::sync_with_progress(
        &root,
        &mut index,
        &mut database,
        synced_at_utc_ms,
        |_| progress_callbacks += 1,
    )?;
    let sync_elapsed = sync_started.elapsed();
    profile::event("progress_callbacks", &[field("count", progress_callbacks)]);

    let page_started = Instant::now();
    let page = database.list_tracks_page(ListTracksQuery {
        offset: 0,
        limit: 1000,
        query: None,
        field_filter: None,
        sort: Default::default(),
    })?;
    profile::event(
        "first_page",
        &[field("ms", ms(page_started.elapsed())), field("total", page.total_count)],
    );

    let size = |suffix: &str| {
        let mut name = database_path.as_os_str().to_os_string();
        name.push(suffix);
        fs::metadata(PathBuf::from(name)).map(|m| m.len()).unwrap_or(0)
    };
    let complete = report.state == Some(SourceScanState::Complete);
    let ready_fields = vec![
        field("ms", ms(process_started.elapsed())),
        field("sync_ms", ms(sync_elapsed)),
        field("tracks", page.total_count),
        field("observed", report.observed),
        field("metadata_reads", report.metadata_reads),
        field("metadata_errors", report.metadata_errors.len()),
        field("source_errors", report.source_errors.len()),
        field("complete", complete),
        field("db_bytes", size("")),
        field("wal_bytes", size("-wal")),
    ];
    let mut ready_fields = ready_fields;
    if let Some((read, write)) = process_io_bytes() {
        ready_fields.push(field("process_read_bytes", read));
        ready_fields.push(field("process_write_bytes", write));
    }
    profile::event("library_ready", &ready_fields);
    let marker = format!("{{{}}}\n", ready_fields.join(","));
    fs::write(out_dir.join("library_ready.json"), marker)?;
    println!(
        "library_ready tracks={} sync_seconds={:.3} total_seconds={:.3} profile={}",
        page.total_count,
        sync_elapsed.as_secs_f64(),
        process_started.elapsed().as_secs_f64(),
        profile_path.display()
    );
    if !complete {
        return Err(format!("scan state {:?}", report.state).into());
    }
    Ok(())
}

/// The process's own I/O byte counters (all files, including the database and WAL).
#[cfg(windows)]
fn process_io_bytes() -> Option<(u64, u64)> {
    #[repr(C)]
    #[derive(Default)]
    struct IoCounters {
        read_operations: u64,
        write_operations: u64,
        other_operations: u64,
        read_bytes: u64,
        write_bytes: u64,
        other_bytes: u64,
    }
    extern "system" {
        fn GetCurrentProcess() -> *mut core::ffi::c_void;
        fn GetProcessIoCounters(process: *mut core::ffi::c_void, counters: *mut IoCounters) -> i32;
    }
    let mut counters = IoCounters::default();
    // SAFETY: the pseudo handle needs no closing and `counters` is a valid IO_COUNTERS-sized buffer.
    let ok = unsafe { GetProcessIoCounters(GetCurrentProcess(), &mut counters) };
    (ok != 0).then_some((counters.read_bytes, counters.write_bytes))
}

#[cfg(not(windows))]
fn process_io_bytes() -> Option<(u64, u64)> {
    None
}
