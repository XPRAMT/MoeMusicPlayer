//! Backup and restore of the settings and the library (Settings → About).
//!
//! **Export** writes one zip: `manifest.json`, `settings.json`, the library database (a consistent snapshot made with
//! `VACUUM INTO`, so the running player is not disturbed), `MyLove.m3u8` (the favourites playlist) and the main
//! background image. Album covers are not included: they are a cache that is rebuilt from the music files.
//!
//! **Import** takes that zip, or any single file from it (`settings.json`, the database, `MyLove.m3u8`), and only
//! replaces the parts it is given. Nothing is replaced while the player runs: the file is checked and unpacked into
//! `import-pending`, and the next start (before the settings and the database are opened) moves the current files into
//! `before-import` and the imported ones into place. Several files can be imported one after the other.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tauri::AppHandle;

const APP_NAME: &str = "MoeMusicPlayer";
const FORMAT_VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
const SETTINGS_NAME: &str = "settings.json";
const LIBRARY_NAME: &str = "moemusicplayer.sqlite3";
const FAVOURITES_NAME: &str = "MyLove.m3u8";
const BACKGROUND_DIRECTORY: &str = "background";
pub const IMPORT_DIRECTORY: &str = "import-pending";
pub const BEFORE_IMPORT_DIRECTORY: &str = "before-import";
const READY_NAME: &str = "READY.json";

const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_LIBRARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SETTINGS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FAVOURITES_BYTES: u64 = 8 * 1024 * 1024;
const MAX_BACKGROUND_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BACKGROUND_FILES: usize = 16;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The parts of the data that can be exported and imported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Settings,
    Library,
    Favourites,
    Background,
}

impl Part {
    fn id(self) -> &'static str {
        match self {
            Part::Settings => "settings",
            Part::Library => "library",
            Part::Favourites => "favourites",
            Part::Background => "background",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    app: String,
    format_version: u32,
    platform: String,
    build_version: String,
    created_at_utc: String,
}

/// What is waiting for the next start.
#[derive(Default, Serialize, Deserialize)]
struct Pending {
    parts: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub file_name: String,
    pub size_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    /// The parts this file provided.
    pub imported: Vec<String>,
    /// Everything waiting to be applied at the next start (this import and earlier ones).
    pub pending: Vec<String>,
}

pub fn platform_name() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else {
        "windows"
    }
}

/// `(year, month, day, hour, minute, second)` in UTC.
fn utc_parts(epoch_seconds: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (epoch_seconds / 86_400) as i64 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    year += i64::from(month <= 2);
    (
        year,
        month,
        day,
        (epoch_seconds / 3600 % 24) as u32,
        (epoch_seconds / 60 % 60) as u32,
        (epoch_seconds % 60) as u32,
    )
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// `MoeMusicPlayer-backup-20261019-213045.zip`
pub fn backup_file_name(epoch_seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(epoch_seconds);
    format!("{APP_NAME}-backup-{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}.zip")
}

fn iso_stamp(epoch_seconds: u64) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(epoch_seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

// ------------------------------------------------------------------------------------------------ export

/// Writes the backup zip. `library_snapshot` is a finished copy of the database (not the live file).
pub fn write_backup_zip(
    data_dir: &Path,
    library_snapshot: &Path,
    output: &Path,
    build_version: &str,
    epoch_seconds: u64,
) -> Result<u64, String> {
    let file = fs::File::create(output).map_err(|error| format!("無法建立備份檔：{error}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut add = |zip: &mut zip::ZipWriter<fs::File>, name: &str, source: &Path| -> Result<(), String> {
        zip.start_file(name, deflated).map_err(|error| error.to_string())?;
        let mut input = fs::File::open(source).map_err(|error| format!("無法讀取 {name}：{error}"))?;
        std::io::copy(&mut input, zip).map_err(|error| format!("寫入 {name} 失敗：{error}"))?;
        Ok(())
    };

    let manifest = Manifest {
        app: APP_NAME.to_owned(),
        format_version: FORMAT_VERSION,
        platform: platform_name().to_owned(),
        build_version: build_version.to_owned(),
        created_at_utc: iso_stamp(epoch_seconds),
    };
    zip.start_file(MANIFEST_NAME, deflated)
        .map_err(|error| error.to_string())?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;

    let settings = data_dir.join(SETTINGS_NAME);
    if !settings.is_file() {
        return Err("找不到 settings.json。".to_owned());
    }
    add(&mut zip, SETTINGS_NAME, &settings)?;
    add(&mut zip, LIBRARY_NAME, library_snapshot)?;
    let favourites = data_dir.join(FAVOURITES_NAME);
    if favourites.is_file() {
        add(&mut zip, FAVOURITES_NAME, &favourites)?;
    }
    // The main background image (the settings point at it). Album covers are deliberately left out.
    if let Ok(entries) = fs::read_dir(data_dir.join(BACKGROUND_DIRECTORY)) {
        let mut files: Vec<_> = entries
            .flatten()
            .filter(|entry| entry.path().is_file())
            .collect();
        files.sort_by_key(|entry| entry.file_name());
        for entry in files.into_iter().take(MAX_BACKGROUND_FILES) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.metadata().map(|meta| meta.len()).unwrap_or(u64::MAX) <= MAX_BACKGROUND_BYTES {
                add(&mut zip, &format!("{BACKGROUND_DIRECTORY}/{name}"), &entry.path())?;
            }
        }
    }
    let file = zip.finish().map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::metadata(output)
        .map(|meta| meta.len())
        .map_err(|error| error.to_string())
}

// ------------------------------------------------------------------------------------------------ import

fn pending_directory(data_dir: &Path) -> PathBuf {
    data_dir.join(IMPORT_DIRECTORY)
}

fn read_pending(data_dir: &Path) -> Pending {
    fs::read(pending_directory(data_dir).join(READY_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// The parts waiting for the next start.
pub fn pending_parts(data_dir: &Path) -> Vec<String> {
    read_pending(data_dir).parts
}

fn mark_pending(data_dir: &Path, added: &[Part]) -> Result<Vec<String>, String> {
    let mut pending = read_pending(data_dir);
    for part in added {
        if !pending.parts.iter().any(|existing| existing == part.id()) {
            pending.parts.push(part.id().to_owned());
        }
    }
    let directory = pending_directory(data_dir);
    let temporary = directory.join(format!("{READY_NAME}.tmp"));
    fs::write(&temporary, serde_json::to_vec(&pending).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    fs::rename(&temporary, directory.join(READY_NAME)).map_err(|error| error.to_string())?;
    Ok(pending.parts)
}

/// Copies at most `limit` bytes; more than that is an error.
fn copy_limited(input: &mut impl Read, output: &Path, limit: u64) -> Result<u64, String> {
    let mut file = fs::File::create(output).map_err(|error| error.to_string())?;
    let copied = std::io::copy(&mut input.take(limit + 1), &mut file).map_err(|error| error.to_string())?;
    if copied > limit {
        drop(file);
        let _ = fs::remove_file(output);
        return Err("備份檔內的檔案超過大小上限。".to_owned());
    }
    file.sync_all().map_err(|error| error.to_string())?;
    Ok(copied)
}

fn stage_path(data_dir: &Path, part: Part, background_name: Option<&str>) -> PathBuf {
    let directory = pending_directory(data_dir);
    match part {
        Part::Settings => directory.join(SETTINGS_NAME),
        Part::Library => directory.join(LIBRARY_NAME),
        Part::Favourites => directory.join(FAVOURITES_NAME),
        Part::Background => directory
            .join(BACKGROUND_DIRECTORY)
            .join(background_name.unwrap_or("background")),
    }
}

/// Checks a staged file the way the player will read it, so a bad file is refused now rather than at the next start.
fn validate(part: Part, path: &Path) -> Result<(), String> {
    match part {
        Part::Settings => crate::settings::check_settings_file(path)
            .map_err(|error| format!("settings.json 無法讀取：{error}")),
        Part::Library => {
            let database = player_db::Database::open_checked(path)
                .map_err(|error| format!("曲庫資料庫無法開啟：{error}"))?;
            let _ = database.checkpoint_wal();
            drop(database);
            for suffix in ["-wal", "-shm"] {
                let mut sidecar = path.as_os_str().to_owned();
                sidecar.push(suffix);
                let _ = fs::remove_file(PathBuf::from(sidecar));
            }
            Ok(())
        }
        Part::Favourites => {
            let bytes = fs::read(path).map_err(|error| error.to_string())?;
            if String::from_utf8(bytes).is_err() {
                return Err("MyLove.m3u8 不是有效的文字檔。".to_owned());
            }
            Ok(())
        }
        Part::Background => Ok(()),
    }
}

/// Validates the file at `staged` and, when it is good, keeps it as the staged `part`.
fn accept(data_dir: &Path, part: Part, staged: &Path, name: Option<&str>) -> Result<(), String> {
    validate(part, staged).inspect_err(|_| {
        let _ = fs::remove_file(staged);
    })?;
    let target = stage_path(data_dir, part, name);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_file(&target);
    fs::rename(staged, &target).map_err(|error| error.to_string())
}

fn background_file_name(entry: &str) -> Option<String> {
    let name = entry.strip_prefix(&format!("{BACKGROUND_DIRECTORY}/"))?;
    let safe = !name.is_empty()
        && name.len() <= 128
        && !name.contains(['/', '\\', ':'])
        && name != "."
        && name != ".."
        && !name.starts_with('.');
    safe.then(|| name.to_owned())
}

/// Stages a backup zip. Returns the parts it held.
fn stage_zip(data_dir: &Path, archive: &Path, platform: &str) -> Result<Vec<Part>, String> {
    let file = fs::File::open(archive).map_err(|error| error.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| format!("不是有效的備份壓縮檔：{error}"))?;
    // Read the manifest first.
    let manifest: Manifest = {
        let mut entry = zip
            .by_name(MANIFEST_NAME)
            .map_err(|_| "壓縮檔缺少 manifest.json，不是 MoeMusicPlayer 的備份。".to_owned())?;
        if entry.size() > MAX_MANIFEST_BYTES {
            return Err("manifest.json 過大。".to_owned());
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|error| error.to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| format!("manifest.json 無法讀取：{error}"))?
    };
    if manifest.app != APP_NAME || manifest.format_version != FORMAT_VERSION {
        return Err("這不是此版本 MoeMusicPlayer 的備份檔。".to_owned());
    }
    if manifest.platform != platform {
        return Err(format!(
            "這份備份來自 {}，無法匯入到 {}（路徑與裝置設定不通用）。",
            manifest.platform, platform
        ));
    }
    // Every entry must be one we know; anything else (or a path that tries to leave the folder) refuses the file.
    let mut plan: Vec<(usize, Part, Option<String>, u64)> = Vec::new();
    let mut background_files = 0;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_owned();
        if entry.is_dir() {
            continue;
        }
        let (part, background, limit) = match name.as_str() {
            MANIFEST_NAME => continue,
            SETTINGS_NAME => (Part::Settings, None, MAX_SETTINGS_BYTES),
            LIBRARY_NAME => (Part::Library, None, MAX_LIBRARY_BYTES),
            FAVOURITES_NAME => (Part::Favourites, None, MAX_FAVOURITES_BYTES),
            _ => match background_file_name(&name) {
                Some(file_name) => {
                    background_files += 1;
                    if background_files > MAX_BACKGROUND_FILES {
                        return Err("備份檔含有過多背景圖片。".to_owned());
                    }
                    (Part::Background, Some(file_name), MAX_BACKGROUND_BYTES)
                }
                None => return Err(format!("備份檔含有不認得的項目：{name}")),
            },
        };
        plan.push((index, part, background, limit));
    }
    if !plan.iter().any(|(_, part, _, _)| *part == Part::Settings)
        || !plan.iter().any(|(_, part, _, _)| *part == Part::Library)
    {
        return Err("備份檔缺少 settings.json 或曲庫資料庫。".to_owned());
    }
    let directory = pending_directory(data_dir);
    fs::create_dir_all(directory.join(".incoming")).map_err(|error| error.to_string())?;
    let mut parts: Vec<Part> = Vec::new();
    let result = (|| -> Result<(), String> {
        for (index, part, background, limit) in &plan {
            let mut entry = zip.by_index(*index).map_err(|error| error.to_string())?;
            if entry.size() > *limit {
                return Err("備份檔內的檔案超過大小上限。".to_owned());
            }
            let staged = directory.join(".incoming").join(format!("{index}.part"));
            copy_limited(&mut entry, &staged, *limit)?;
            accept(data_dir, *part, &staged, background.as_deref())?;
            if !parts.contains(part) {
                parts.push(*part);
            }
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(directory.join(".incoming"));
    result?;
    Ok(parts)
}

/// Stages one file the user picked: a backup zip, or `settings.json`, the library database or `MyLove.m3u8`.
pub fn stage_import(data_dir: &Path, source: &Path, platform: &str) -> Result<ImportResult, String> {
    let size = fs::metadata(source)
        .map_err(|error| format!("無法讀取檔案：{error}"))?
        .len();
    if size == 0 || size > MAX_ARCHIVE_BYTES {
        return Err("檔案是空的或超過大小上限。".to_owned());
    }
    let mut magic = [0u8; 4];
    let read = fs::File::open(source)
        .and_then(|mut file| file.read(&mut magic))
        .map_err(|error| error.to_string())?;
    let is_zip = read == 4 && &magic == b"PK\x03\x04";
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    fs::create_dir_all(pending_directory(data_dir)).map_err(|error| error.to_string())?;
    let parts = if is_zip {
        stage_zip(data_dir, source, platform)?
    } else {
        let (part, limit) = match extension.as_str() {
            "json" => (Part::Settings, MAX_SETTINGS_BYTES),
            "sqlite3" | "sqlite" | "db" => (Part::Library, MAX_LIBRARY_BYTES),
            "m3u8" | "m3u" => (Part::Favourites, MAX_FAVOURITES_BYTES),
            _ => return Err("不認得的檔案：請選擇備份壓縮檔，或其中的 settings.json、曲庫資料庫（.sqlite3）、MyLove.m3u8。".to_owned()),
        };
        if size > limit {
            return Err("檔案超過大小上限。".to_owned());
        }
        let staged = pending_directory(data_dir).join(".incoming-file");
        let mut input = fs::File::open(source).map_err(|error| error.to_string())?;
        copy_limited(&mut input, &staged, limit)?;
        accept(data_dir, part, &staged, None)?;
        vec![part]
    };
    let pending = mark_pending(data_dir, &parts)?;
    Ok(ImportResult {
        imported: parts.iter().map(|part| part.id().to_owned()).collect(),
        pending,
    })
}

// ------------------------------------------------------------------------------------------------ apply at start

fn move_aside(data_dir: &Path, backup: &Path, name: &str, moved: &mut Vec<(PathBuf, PathBuf)>) -> Result<(), String> {
    let current = data_dir.join(name);
    if !current.exists() {
        return Ok(());
    }
    let target = backup.join(name);
    fs::rename(&current, &target).map_err(|error| format!("無法保留現有的 {name}：{error}"))?;
    moved.push((current, target));
    Ok(())
}

/// Called at the start of the program, before the settings and the database are opened: replaces the current files
/// by the staged ones (keeping the current ones in `before-import`). Returns the parts that were applied.
pub fn apply_pending_import(data_dir: &Path) -> Result<Vec<String>, String> {
    let pending = read_pending(data_dir);
    if pending.parts.is_empty() {
        return Ok(Vec::new());
    }
    let staging = pending_directory(data_dir);
    let backup = data_dir.join(BEFORE_IMPORT_DIRECTORY);
    let _ = fs::remove_dir_all(&backup);
    fs::create_dir_all(&backup).map_err(|error| error.to_string())?;
    let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut placed: Vec<PathBuf> = Vec::new();
    let result = (|| -> Result<(), String> {
        for part in &pending.parts {
            match part.as_str() {
                "settings" => {
                    for name in [SETTINGS_NAME, "settings.json.bak"] {
                        move_aside(data_dir, &backup, name, &mut moved)?;
                    }
                    place(&staging, data_dir, SETTINGS_NAME, &mut placed)?;
                }
                "library" => {
                    for name in [LIBRARY_NAME, "moemusicplayer.sqlite3-wal", "moemusicplayer.sqlite3-shm"] {
                        move_aside(data_dir, &backup, name, &mut moved)?;
                    }
                    place(&staging, data_dir, LIBRARY_NAME, &mut placed)?;
                }
                "favourites" => {
                    move_aside(data_dir, &backup, FAVOURITES_NAME, &mut moved)?;
                    place(&staging, data_dir, FAVOURITES_NAME, &mut placed)?;
                }
                "background" => {
                    move_aside(data_dir, &backup, BACKGROUND_DIRECTORY, &mut moved)?;
                    place(&staging, data_dir, BACKGROUND_DIRECTORY, &mut placed)?;
                }
                _ => {}
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Put everything back as it was.
        for path in placed.iter().rev() {
            if path.is_dir() {
                let _ = fs::remove_dir_all(path);
            } else {
                let _ = fs::remove_file(path);
            }
        }
        for (original, saved) in moved.iter().rev() {
            let _ = fs::rename(saved, original);
        }
        return Err(error);
    }
    let _ = fs::remove_dir_all(&staging);
    Ok(pending.parts)
}

fn place(staging: &Path, data_dir: &Path, name: &str, placed: &mut Vec<PathBuf>) -> Result<(), String> {
    let staged = staging.join(name);
    if !staged.exists() {
        return Ok(());
    }
    let target = data_dir.join(name);
    fs::rename(&staged, &target).map_err(|error| format!("無法套用匯入的 {name}：{error}"))?;
    placed.push(target);
    Ok(())
}

// ------------------------------------------------------------------------------------------------ commands

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

fn work_directory(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let base = if cfg!(target_os = "android") {
        app.path().app_cache_dir().map_err(|error| error.to_string())?
    } else {
        std::env::temp_dir()
    };
    let directory = base.join("moe-backup-work");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    // Leftovers of an interrupted run.
    if let Ok(entries) = fs::read_dir(&directory) {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for entry in entries.flatten() {
            if entry.metadata().and_then(|meta| meta.modified()).is_ok_and(|modified| modified < cutoff) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Ok(directory)
}

/// Exports the settings and the library into one zip chosen with the system file dialog.
#[tauri::command]
pub async fn backup_export(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ExportResult>, String> {
    let data_dir = crate::user_data_directory(&state)?;
    let _work = crate::enter_database_work(&state)?;
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| "曲庫資料庫尚未開啟。".to_owned())?;
    let work = work_directory(&app)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let snapshot = work.join(format!("{token}.sqlite3"));
    let archive = work.join(format!("{token}.zip"));
    let epoch = now_seconds();
    let file_name = backup_file_name(epoch);
    let build = {
        let (data_dir, snapshot, archive) = (data_dir.clone(), snapshot.clone(), archive.clone());
        database
            .snapshot_to(&snapshot)
            .map_err(|error| format!("無法複製曲庫資料庫：{error}"))?;
        blocking(move || write_backup_zip(&data_dir, &snapshot, &archive, env!("MOE_BUILD_VERSION"), epoch)).await
    };
    let _ = fs::remove_file(&snapshot);
    let size = match build {
        Ok(size) => size,
        Err(error) => {
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
    };
    let saved = deliver_export(&app, &window, &archive, &file_name).await;
    let _ = fs::remove_file(&archive);
    Ok(saved?.then_some(ExportResult { file_name, size_bytes: size }))
}

#[cfg(target_os = "windows")]
async fn deliver_export(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    archive: &Path,
    file_name: &str,
) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = app
        .dialog()
        .file()
        .set_parent(window)
        .set_title("匯出設定與曲庫")
        .set_file_name(file_name)
        .add_filter("備份壓縮檔", &["zip"])
        .blocking_save_file();
    let Some(selected) = selected else {
        return Ok(false);
    };
    let target = selected
        .into_path()
        .map_err(|error| format!("無法取得檔案路徑：{error}"))?;
    fs::copy(archive, &target).map_err(|error| format!("寫入失敗：{error}"))?;
    Ok(true)
}

#[cfg(target_os = "android")]
async fn deliver_export(
    app: &AppHandle,
    _window: &tauri::WebviewWindow,
    archive: &Path,
    file_name: &str,
) -> Result<bool, String> {
    use tauri_plugin_media_index::MediaIndexExt;
    let (app, archive, file_name) = (app.clone(), archive.to_path_buf(), file_name.to_owned());
    blocking(move || {
        let Some(uri) = app
            .media_index()
            .pick_create_document(&file_name, "application/zip")
            .map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        app.media_index()
            .copy_file_to_uri(&archive, &uri)
            .map_err(|error| format!("寫入失敗：{error}"))?;
        Ok(true)
    })
    .await
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
async fn deliver_export(
    _app: &AppHandle,
    _window: &tauri::WebviewWindow,
    _archive: &Path,
    _file_name: &str,
) -> Result<bool, String> {
    Err("此平台尚未支援匯出。".to_owned())
}

/// Imports a backup zip, or a single file from one, chosen with the system file dialog. The import is applied the next
/// time the program starts.
#[tauri::command]
pub async fn backup_import(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<ImportResult>, String> {
    let data_dir = crate::user_data_directory(&state)?;
    let work = work_directory(&app)?;
    let picked = pick_import_files(&app, &window, &work).await?;
    if picked.is_empty() {
        return Ok(None);
    }
    blocking(move || {
        let mut imported: Vec<String> = Vec::new();
        let mut pending: Vec<String> = Vec::new();
        for (path, temporary) in &picked {
            let result = stage_import(&data_dir, path, platform_name());
            if *temporary {
                let _ = fs::remove_file(path);
            }
            let result = result?;
            for part in result.imported {
                if !imported.contains(&part) {
                    imported.push(part);
                }
            }
            pending = result.pending;
        }
        Ok(Some(ImportResult { imported, pending }))
    })
    .await
}

/// The files to import with a flag telling whether the path is a temporary copy to delete afterwards.
#[cfg(target_os = "windows")]
async fn pick_import_files(
    app: &AppHandle,
    window: &tauri::WebviewWindow,
    _work: &Path,
) -> Result<Vec<(PathBuf, bool)>, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = app
        .dialog()
        .file()
        .set_parent(window)
        .set_title("匯入設定與曲庫")
        .add_filter("備份壓縮檔或其中的檔案", &["zip", "json", "sqlite3", "m3u8"])
        .blocking_pick_files();
    let mut files = Vec::new();
    for item in selected.unwrap_or_default() {
        files.push((
            item.into_path().map_err(|error| format!("無法取得檔案路徑：{error}"))?,
            false,
        ));
    }
    Ok(files)
}

#[cfg(target_os = "android")]
async fn pick_import_files(
    app: &AppHandle,
    _window: &tauri::WebviewWindow,
    work: &Path,
) -> Result<Vec<(PathBuf, bool)>, String> {
    use tauri_plugin_media_index::MediaIndexExt;
    let (app, work) = (app.clone(), work.to_path_buf());
    blocking(move || {
        let Some((uri, display_name)) = app
            .media_index()
            .pick_open_document(&["*/*"])
            .map_err(|error| error.to_string())?
        else {
            return Ok(Vec::new());
        };
        // Keep the real file name's extension: it tells a database from a settings file.
        let extension = display_name
            .as_deref()
            .and_then(|name| Path::new(name).extension())
            .and_then(|extension| extension.to_str())
            .filter(|extension| extension.len() <= 8 && extension.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("zip")
            .to_owned();
        let destination = work.join(format!("{}.{extension}", uuid::Uuid::new_v4().simple()));
        app.media_index()
            .copy_uri_to_file(&uri, &destination, MAX_ARCHIVE_BYTES)
            .map_err(|error| format!("讀取檔案失敗：{error}"))?;
        Ok(vec![(destination, true)])
    })
    .await
}

#[cfg(not(any(target_os = "windows", target_os = "android")))]
async fn pick_import_files(
    _app: &AppHandle,
    _window: &tauri::WebviewWindow,
    _work: &Path,
) -> Result<Vec<(PathBuf, bool)>, String> {
    Err("此平台尚未支援匯入。".to_owned())
}

/// The parts waiting for the next start (shown again when Settings → About is reopened).
#[tauri::command]
pub fn backup_pending(state: tauri::State<'_, crate::AppState>) -> Vec<String> {
    crate::user_data_directory(&state)
        .map(|data_dir| pending_parts(&data_dir))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppSettings, SettingsStore};

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("moe-backup-test-{name}-{}", uuid::Uuid::new_v4().simple()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A data folder like the player's: settings, a real library database, favourites, a background, covers, logs.
    fn data_folder(scratch: &Scratch, marker: &str) -> PathBuf {
        let data = scratch.0.join(format!("data-{marker}"));
        fs::create_dir_all(data.join("album-cover")).unwrap();
        fs::create_dir_all(data.join("background")).unwrap();
        fs::create_dir_all(data.join("diagnostics")).unwrap();
        let store = SettingsStore::open(data.join(SETTINGS_NAME), AppSettings::default()).unwrap();
        store
            .update(|settings| {
                settings.language = crate::settings::UiLanguage::Ja;
                Ok(())
            })
            .unwrap();
        drop(store);
        drop(player_db::Database::open(data.join(LIBRARY_NAME)).unwrap());
        fs::write(data.join(FAVOURITES_NAME), format!("#EXTM3U\n#{marker}\n")).unwrap();
        fs::write(data.join("background").join("wall.jpg"), format!("jpg-{marker}")).unwrap();
        fs::write(data.join("album-cover").join("abc.jpg"), b"cover").unwrap();
        fs::write(data.join("diagnostics").join("panic.jsonl"), b"log").unwrap();
        data
    }

    fn export(scratch: &Scratch, data: &Path) -> PathBuf {
        let database = player_db::Database::open(data.join(LIBRARY_NAME)).unwrap();
        let snapshot = scratch.0.join("snapshot.sqlite3");
        database.snapshot_to(&snapshot).unwrap();
        let archive = scratch.0.join("backup.zip");
        write_backup_zip(data, &snapshot, &archive, "26.10.19", 1_792_400_000).unwrap();
        archive
    }

    fn zip_names(archive: &Path) -> Vec<String> {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        let mut names: Vec<String> = (0..zip.len()).map(|index| zip.by_index(index).unwrap().name().to_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn the_export_is_one_zip_without_covers_logs_or_caches() {
        let scratch = Scratch::new("export");
        let data = data_folder(&scratch, "a");
        let archive = export(&scratch, &data);
        assert_eq!(
            zip_names(&archive),
            ["MyLove.m3u8", "background/wall.jpg", "manifest.json", "moemusicplayer.sqlite3", "settings.json"]
        );
        assert_eq!(backup_file_name(1_792_400_000), "MoeMusicPlayer-backup-20261019-085320.zip");
    }

    #[test]
    fn importing_the_zip_replaces_everything_at_the_next_start_and_keeps_the_old_files() {
        let scratch = Scratch::new("zip");
        let source = data_folder(&scratch, "source");
        let archive = export(&scratch, &source);
        let target = data_folder(&scratch, "target");
        let result = stage_import(&target, &archive, platform_name()).unwrap();
        assert_eq!(result.imported, ["settings", "library", "favourites", "background"]);
        // Nothing is touched while the player runs.
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("target"));
        assert_eq!(pending_parts(&target), result.pending);
        let applied = apply_pending_import(&target).unwrap();
        assert_eq!(applied.len(), 4);
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("source"));
        assert_eq!(fs::read_to_string(target.join("background").join("wall.jpg")).unwrap(), "jpg-source");
        assert!(fs::read_to_string(target.join(BEFORE_IMPORT_DIRECTORY).join(FAVOURITES_NAME)).unwrap().contains("target"));
        assert!(target.join("album-cover").join("abc.jpg").is_file(), "covers are never replaced");
        assert!(!target.join(IMPORT_DIRECTORY).exists());
        player_db::Database::open_checked(target.join(LIBRARY_NAME)).expect("the imported library opens");
        assert!(apply_pending_import(&target).unwrap().is_empty(), "nothing is applied twice");
    }

    #[test]
    fn a_single_file_from_the_backup_replaces_only_its_part() {
        let scratch = Scratch::new("single");
        let source = data_folder(&scratch, "source");
        let target = data_folder(&scratch, "target");
        let result = stage_import(&target, &source.join(FAVOURITES_NAME), platform_name()).unwrap();
        assert_eq!(result.imported, ["favourites"]);
        // A second file can follow before the restart.
        let second = stage_import(&target, &source.join(SETTINGS_NAME), platform_name()).unwrap();
        assert_eq!(second.pending, ["favourites", "settings"]);
        apply_pending_import(&target).unwrap();
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("source"));
        assert_eq!(fs::read_to_string(target.join("background").join("wall.jpg")).unwrap(), "jpg-target", "the background stays");
        assert!(target.join(BEFORE_IMPORT_DIRECTORY).join(SETTINGS_NAME).is_file());
    }

    #[test]
    fn bad_files_are_refused_and_stage_nothing() {
        let scratch = Scratch::new("bad");
        let target = data_folder(&scratch, "target");
        let garbage = scratch.0.join("settings.json");
        fs::write(&garbage, b"{ not json").unwrap();
        assert!(stage_import(&target, &garbage, platform_name()).is_err());
        let not_a_database = scratch.0.join("library.sqlite3");
        fs::write(&not_a_database, b"this is not a database file at all").unwrap();
        assert!(stage_import(&target, &not_a_database, platform_name()).is_err());
        let unknown = scratch.0.join("notes.txt");
        fs::write(&unknown, b"hello").unwrap();
        assert!(stage_import(&target, &unknown, platform_name()).is_err());
        assert!(pending_parts(&target).is_empty());
        assert!(apply_pending_import(&target).unwrap().is_empty());
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("target"));
    }

    #[test]
    fn a_backup_from_another_platform_or_with_odd_entries_is_refused() {
        let scratch = Scratch::new("zip-bad");
        let source = data_folder(&scratch, "source");
        let archive = export(&scratch, &source);
        let target = data_folder(&scratch, "target");
        let other = if platform_name() == "windows" { "android" } else { "windows" };
        assert!(stage_import(&target, &archive, other).unwrap_err().contains("無法匯入"));
        // A zip with an entry that tries to leave the folder, and one with an unknown entry.
        for entry in ["../evil.txt", "unknown.bin", "background/../../x.jpg"] {
            let path = scratch.0.join("odd.zip");
            {
                let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
                let options = zip::write::SimpleFileOptions::default();
                zip.start_file(MANIFEST_NAME, options).unwrap();
                zip.write_all(
                    serde_json::to_string(&Manifest {
                        app: APP_NAME.to_owned(),
                        format_version: FORMAT_VERSION,
                        platform: platform_name().to_owned(),
                        build_version: "x".to_owned(),
                        created_at_utc: "x".to_owned(),
                    })
                    .unwrap()
                    .as_bytes(),
                )
                .unwrap();
                zip.start_file(entry, options).unwrap();
                zip.write_all(b"x").unwrap();
                zip.finish().unwrap();
            }
            assert!(stage_import(&target, &path, platform_name()).is_err(), "{entry}");
        }
        assert!(pending_parts(&target).is_empty());
    }
}
