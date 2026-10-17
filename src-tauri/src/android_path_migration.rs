//! One-time migration of legacy Android MediaStore / SAF sources and SAF playlists to real
//! filesystem paths under All files access (ADR 0014, 2026-10-17 path-source update).
//!
//! Track identity is preserved: every `source_mappings` row is rewritten in place to the
//! path key the filesystem scanner produces, so `TrackId`s and everything keyed by them
//! (play statistics, lyrics, playlist entries, playback session) stay attached. A full
//! database snapshot and a copy of `settings.json` are written before anything changes.

use std::path::{Path, PathBuf};

use player_core::{unix_path_key, LibraryRoot, MediaLocator, MediaSourceKind};
use player_db::Database;
use tauri::{AppHandle, Manager};
use tauri_plugin_media_index::{storage_paths, MediaIndexExt};

use crate::{
    settings::{SourceEntry, SourceEntryKind, StoredPath},
    AppState,
};

const PRIMARY_STORAGE: &str = "/storage/emulated/0";

#[derive(Debug, Default)]
pub struct MigrationReport {
    pub migrated_sources: usize,
    pub rewritten_items: usize,
    pub skipped: Vec<String>,
    pub backup_directory: Option<PathBuf>,
}

fn is_legacy(source: &SourceEntry) -> bool {
    match &source.kind {
        SourceEntryKind::Folder { media_kind, .. } => matches!(
            media_kind,
            MediaSourceKind::AndroidMediaStore | MediaSourceKind::AndroidSaf
        ),
        SourceEntryKind::PlaylistFile { path, .. } => matches!(path, StoredPath::Uri(_)),
    }
}

/// Run the migration when All files access is granted and legacy entries remain. Idempotent:
/// already-migrated mappings (filesystem locators) are left alone.
pub fn migrate_legacy_sources(
    app: &AppHandle,
    database: &Database,
) -> Result<MigrationReport, String> {
    let mut report = MigrationReport::default();
    let state = app.state::<AppState>();
    let settings = state.settings.snapshot().map_err(|error| error.to_string())?;
    let legacy_slideshow = matches!(settings.theme.slideshow_folder, Some(StoredPath::Uri(_)));
    let legacy_sources = settings
        .sources
        .iter()
        .filter(|source| is_legacy(source))
        .cloned()
        .collect::<Vec<_>>();
    if legacy_sources.is_empty() && !legacy_slideshow {
        return Ok(report);
    }
    let granted = app
        .media_index()
        .all_files_access_status()
        .map(|status| status.granted)
        .unwrap_or(false);
    if !granted {
        return Ok(report);
    }

    if !legacy_sources.is_empty() {
        report.backup_directory = Some(backup(&state, database)?);
    }

    for source in legacy_sources {
        match migrate_source(app, database, &source) {
            Ok(Some((migrated, rewritten))) => {
                state
                    .settings
                    .upsert_source(migrated)
                    .map_err(|error| error.to_string())?;
                report.migrated_sources += 1;
                report.rewritten_items += rewritten;
            }
            Ok(None) => report.skipped.push(source.display_name.clone()),
            Err(error) => report
                .skipped
                .push(format!("{}：{error}", source.display_name)),
        }
    }

    if let Some(directory) = &report.backup_directory {
        let _ = std::fs::write(
            directory.join("migration-report.txt"),
            format!(
                "migrated_sources={}\nrewritten_items={}\nskipped={:?}\n",
                report.migrated_sources, report.rewritten_items, report.skipped
            ),
        );
    }

    if let Some(StoredPath::Uri(uri)) = &settings.theme.slideshow_folder {
        if let Some(path) = storage_paths::external_storage_path_for_uri(uri)
            .and_then(|path| std::fs::canonicalize(path).ok())
            .filter(|path| path.is_dir())
        {
            if let Ok(stored) = StoredPath::from_path(&path) {
                let _ = state.settings.update(|settings| {
                    settings.theme.slideshow_folder = Some(stored);
                    Ok(())
                });
            }
        }
    }
    Ok(report)
}

fn backup(state: &AppState, database: &Database) -> Result<PathBuf, String> {
    let base = state
        .database_path
        .as_deref()
        .and_then(Path::parent)
        .ok_or_else(|| "找不到曲庫資料庫目錄，已略過來源遷移。".to_owned())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let directory = base.join(format!("pre-path-migration-{stamp}"));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("無法建立遷移備份資料夾，已略過來源遷移：{error}"))?;
    database
        .backup_into(&directory.join("moemusicplayer.sqlite3"))
        .map_err(|error| format!("無法備份曲庫資料庫，已略過來源遷移：{error}"))?;
    let settings_file = state
        .settings
        .data_directory()
        .map(|directory| directory.join("settings.json"))
        .ok_or_else(|| "找不到設定檔目錄，已略過來源遷移。".to_owned())?;
    std::fs::copy(settings_file, directory.join("settings.json"))
        .map_err(|error| format!("無法備份設定檔，已略過來源遷移：{error}"))?;
    Ok(directory)
}

/// Real path of a legacy content locator: SAF document URIs map lexically; MediaStore URIs are
/// resolved in one batch through the platform (`_data` under All files access).
fn resolve_item_paths(
    app: &AppHandle,
    items: &[(String, MediaLocator)],
) -> Result<Vec<Option<PathBuf>>, String> {
    let mut paths = vec![None; items.len()];
    let mut pending = Vec::new();
    for (index, (_, locator)) in items.iter().enumerate() {
        if let MediaLocator::ContentUri(uri) = locator {
            match storage_paths::external_storage_path_for_uri(uri) {
                Some(path) => paths[index] = Some(path),
                None => pending.push(index),
            }
        }
    }
    for chunk in pending.chunks(500) {
        let uris = chunk
            .iter()
            .filter_map(|index| match &items[*index].1 {
                MediaLocator::ContentUri(uri) => Some(uri.clone()),
                MediaLocator::FileSystem(_) => None,
            })
            .collect::<Vec<_>>();
        let resolved = app
            .media_index()
            .resolve_direct_paths(&uris)
            .map_err(|error| error.to_string())?;
        for (index, path) in chunk.iter().zip(resolved) {
            paths[*index] = path.map(PathBuf::from);
        }
    }
    Ok(paths)
}

fn media_store_volume_path(uri: &str) -> Option<PathBuf> {
    let volume = uri.strip_prefix("content://media/")?.split('/').next()?;
    match volume {
        "external" | "external_primary" => Some(PathBuf::from(PRIMARY_STORAGE)),
        "" => None,
        other => Some(PathBuf::from("/storage").join(other.to_ascii_uppercase())),
    }
}

fn migrate_source(
    app: &AppHandle,
    database: &Database,
    source: &SourceEntry,
) -> Result<Option<(SourceEntry, usize)>, String> {
    let (root_path, kind) = match &source.kind {
        SourceEntryKind::Folder {
            media_kind: MediaSourceKind::AndroidMediaStore,
            path: StoredPath::Uri(uri),
        } => (media_store_volume_path(uri), MediaSourceKind::AndroidFilesystem),
        SourceEntryKind::Folder {
            media_kind: MediaSourceKind::AndroidSaf,
            path: StoredPath::Uri(uri),
        } => (
            storage_paths::external_storage_path_for_uri(uri),
            MediaSourceKind::AndroidFilesystem,
        ),
        SourceEntryKind::Folder { media_kind, path } => {
            // Settings already carry a path; only the kind is legacy.
            (path.to_path_buf().ok(), {
                let _ = media_kind;
                MediaSourceKind::AndroidFilesystem
            })
        }
        SourceEntryKind::PlaylistFile {
            path: StoredPath::Uri(uri),
            ..
        } => (
            storage_paths::external_storage_path_for_uri(uri),
            MediaSourceKind::PlaylistFile,
        ),
        SourceEntryKind::PlaylistFile { .. } => return Ok(None),
    };
    let Some(root_path) = root_path.and_then(|path| std::fs::canonicalize(path).ok()) else {
        return Ok(None);
    };
    let is_playlist = kind == MediaSourceKind::PlaylistFile;
    if (is_playlist && !root_path.is_file()) || (!is_playlist && !root_path.is_dir()) {
        return Ok(None);
    }

    let items = database
        .source_item_locators(source.id)
        .map_err(|error| error.to_string())?;
    let paths = resolve_item_paths(app, &items)?;
    let rewrites = items
        .iter()
        .zip(paths)
        .filter_map(|((item_id, _), path)| {
            let path = std::fs::canonicalize(path?).ok()?;
            Some((item_id.clone(), unix_path_key(&path), path))
        })
        .collect::<Vec<_>>();

    let stored = StoredPath::from_path(&root_path).map_err(|error| error.to_string())?;
    let mut migrated = source.clone();
    migrated.kind = match &source.kind {
        SourceEntryKind::Folder { .. } => SourceEntryKind::Folder {
            media_kind: kind,
            path: stored,
        },
        SourceEntryKind::PlaylistFile { playlist_id, .. } => SourceEntryKind::PlaylistFile {
            playlist_id: *playlist_id,
            path: stored,
            tree_uri: None,
        },
    };
    let root = LibraryRoot {
        id: source.id,
        kind,
        display_name: source.display_name.clone(),
        locator: MediaLocator::FileSystem(root_path),
        enabled: source.enabled,
    };
    if !database
        .library_roots()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|existing| existing.id == source.id)
    {
        database
            .save_library_root(&root)
            .map_err(|error| error.to_string())?;
    }
    let rewritten = database
        .migrate_source_to_filesystem(&root, &rewrites)
        .map_err(|error| error.to_string())?;
    Ok(Some((migrated, rewritten)))
}
