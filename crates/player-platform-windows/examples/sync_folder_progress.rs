use std::{
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use player_core::{ListTracksQuery, MediaLocator, MediaSourceKind, SourceScanState, SyncEngine};
use player_db::Database;
use player_platform_windows::WindowsMediaIndex;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os().skip(1);
    let root_path = args.next().map(PathBuf::from).ok_or(
        "usage: cargo run -p player-platform-windows --example sync_folder_progress -- <music-root> <new-isolated-database>",
    )?;
    let database_path = args.next().map(PathBuf::from).ok_or(
        "usage: cargo run -p player-platform-windows --example sync_folder_progress -- <music-root> <new-isolated-database>",
    )?;
    if args.next().is_some() {
        return Err("expected exactly two paths".into());
    }

    let canonical_root = fs::canonicalize(&root_path)?;
    let absolute_database_path = std::path::absolute(&database_path)?;
    if absolute_database_path.starts_with(&canonical_root) {
        return Err("the test database must be outside the music root".into());
    }
    if database_path.exists() {
        return Err("refusing to overwrite an existing test database".into());
    }

    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let canonical_parent = fs::canonicalize(parent)?;
    let file_name = database_path
        .file_name()
        .ok_or("database path must include a file name")?;
    let database_path = canonical_parent.join(file_name);
    if database_path.starts_with(&canonical_root) {
        return Err("the test database must be outside the music root".into());
    }
    if database_path.exists() {
        return Err("refusing to overwrite an existing test database".into());
    }

    let mut database = Database::open(&database_path)?;
    let root = database.add_library_root(
        MediaSourceKind::WindowsFilesystem,
        "progress acceptance",
        MediaLocator::FileSystem(root_path),
    )?;
    let synced_at_utc_ms =
        i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    let mut index = WindowsMediaIndex::new();
    let report = SyncEngine::sync_with_progress(
        &root,
        &mut index,
        &mut database,
        synced_at_utc_ms,
        |progress| {
            println!(
                "progress source={} stage={:?} processed={} total={:?} unit={:?} observed={} metadata_reads={} unchanged={} errors={} outcome={:?}",
                progress.source_id,
                progress.stage,
                progress.processed,
                progress.total,
                progress.unit,
                progress.observed,
                progress.metadata_reads,
                progress.unchanged,
                progress.error_count,
                progress.outcome,
            );
        },
    )?;
    let page = database.list_tracks_page(ListTracksQuery {
        offset: 0,
        limit: 1,
        query: None,
    })?;

    println!(
        "finished state={:?} observed={} metadata_reads={} unchanged={} updated={} removed_mappings={} errors={} database_tracks={} database={}",
        report.state,
        report.observed,
        report.metadata_reads,
        report.unchanged,
        report.applied.inserted_or_updated,
        report.applied.removed_source_mappings,
        report.source_errors.len() + report.metadata_errors.len(),
        page.total_count,
        database_path.display(),
    );
    if report.state != Some(SourceScanState::Complete) {
        return Err("scan did not complete; inspect the reported state and error count".into());
    }
    if page.total_count != report.observed {
        return Err(format!(
            "database count {} does not match observed count {}",
            page.total_count, report.observed
        )
        .into());
    }
    Ok(())
}
