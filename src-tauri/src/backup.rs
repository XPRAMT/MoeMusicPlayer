//! Backup and restore of the settings and the library (Settings → About).
//!
//! **Export** writes one zip holding only what the user ticked (see [`ExportSelection`]): `manifest.json` (which lists
//! the parts), `settings.json` (without any password), the library database (a consistent snapshot made with
//! `VACUUM INTO`, so the running player is not disturbed), `MyLove.m3u8` (the favourites playlist) and the remote
//! accounts sealed under a passphrase. Album covers and the main background image are not included.
//!
//! **Import** takes that zip, or any single file from it (`settings.json`, the database, `MyLove.m3u8`), and only
//! replaces the parts it is given. Entries of the old layout that are no longer exported (`background/…`) are ignored.
//! Nothing is replaced while the player runs: the file is checked and unpacked into
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
/// Old backups carried the main background image under this prefix; it is ignored on import.
const LEGACY_BACKGROUND_DIRECTORY: &str = "background";
pub const IMPORT_DIRECTORY: &str = "import-pending";
pub const BEFORE_IMPORT_DIRECTORY: &str = "before-import";
const READY_NAME: &str = "READY.json";
/// The passphrase-sealed remote accounts of a backup (an entry inside the zip, never a stand-alone file).
const REMOTE_NAME: &str = "remote-accounts.sealed";
/// The name earlier builds gave that entry; still accepted on import.
const LEGACY_REMOTE_NAME: &str = "remote-accounts.moeremote";
/// The id of the remote accounts in the manifest (they are not staged like the other parts).
const ACCOUNTS_ID: &str = "accounts";
const MAX_REMOTE_BYTES: u64 = 1024 * 1024;

const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_LIBRARY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SETTINGS_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FAVOURITES_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The parts of the data that can be exported and imported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Settings,
    Library,
    Favourites,
}

impl Part {
    fn id(self) -> &'static str {
        match self {
            Part::Settings => "settings",
            Part::Library => "library",
            Part::Favourites => "favourites",
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
    /// What the zip holds: `settings`, `library`, `accounts`, `favourites`. Absent in older backups.
    #[serde(default)]
    parts: Vec<String>,
}

/// What the user ticked in the export panel.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSelection {
    #[serde(default)]
    pub library: bool,
    #[serde(default)]
    pub settings: bool,
    /// The remote accounts, passwords included, sealed under `passphrase`.
    #[serde(default)]
    pub accounts: bool,
    #[serde(default)]
    pub favourites: bool,
    #[serde(default)]
    pub passphrase: Option<String>,
}

impl ExportSelection {
    /// Nothing ticked, or accounts without a usable passphrase, is refused.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.library || self.settings || self.accounts || self.favourites) {
            return Err("請至少選擇一項要匯出的內容。".to_owned());
        }
        if self.accounts && self.passphrase.as_deref().map_or(0, |text| text.chars().count()) < 8 {
            return Err("匯出密碼至少需要 8 個字元。".to_owned());
        }
        Ok(())
    }
}

/// The finished pieces [`write_backup_zip`] puts into the zip.
pub struct ZipContents<'a> {
    /// A finished copy of the database (not the live file).
    pub library_snapshot: Option<&'a Path>,
    pub settings: bool,
    pub favourites: bool,
    /// The text of an account export (passwords sealed under the user's passphrase).
    pub remote_export: Option<&'a str>,
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
    /// `needsPassphrase` when the file carried remote accounts that wait for their passphrase (see
    /// [`backup_remote_accounts_import`]).
    pub remote_accounts: Option<String>,
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

/// `settings.json` with the stored remote-account passwords removed (the accounts stay, as "needs a password").
fn settings_without_passwords(settings: &Path) -> Result<Vec<u8>, String> {
    let bytes = fs::read(settings).map_err(|error| format!("無法讀取 settings.json：{error}"))?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("settings.json 無法讀取：{error}"))?;
    if let Some(accounts) = json.get_mut("remoteAccounts").and_then(serde_json::Value::as_array_mut) {
        for account in accounts {
            if let Some(object) = account.as_object_mut() {
                object.remove("password");
            }
        }
    }
    serde_json::to_vec_pretty(&json).map_err(|error| error.to_string())
}

/// Writes the backup zip with the parts in `contents`.
pub fn write_backup_zip(
    data_dir: &Path,
    contents: &ZipContents,
    output: &Path,
    build_version: &str,
    epoch_seconds: u64,
) -> Result<u64, String> {
    // The passwords in settings.json are sealed with this device's key and mean nothing elsewhere: the copy in the backup
    // has none. The accounts' passwords travel in the passphrase-sealed entry, when the user gave a passphrase.
    let settings_bytes = if contents.settings {
        let settings = data_dir.join(SETTINGS_NAME);
        if !settings.is_file() {
            return Err("找不到 settings.json。".to_owned());
        }
        Some(settings_without_passwords(&settings)?)
    } else {
        None
    };
    let favourites = data_dir.join(FAVOURITES_NAME);
    let favourites = (contents.favourites && favourites.is_file()).then_some(favourites);
    let mut parts: Vec<String> = Vec::new();
    if settings_bytes.is_some() {
        parts.push(Part::Settings.id().to_owned());
    }
    if contents.library_snapshot.is_some() {
        parts.push(Part::Library.id().to_owned());
    }
    if contents.remote_export.is_some() {
        parts.push(ACCOUNTS_ID.to_owned());
    }
    if favourites.is_some() {
        parts.push(Part::Favourites.id().to_owned());
    }
    if parts.is_empty() {
        return Err("沒有可匯出的內容。".to_owned());
    }

    let file = fs::File::create(output).map_err(|error| format!("無法建立備份檔：{error}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let deflated = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let add = |zip: &mut zip::ZipWriter<fs::File>, name: &str, source: &Path| -> Result<(), String> {
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
        parts,
    };
    zip.start_file(MANIFEST_NAME, deflated)
        .map_err(|error| error.to_string())?;
    zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    if let Some(bytes) = settings_bytes {
        zip.start_file(SETTINGS_NAME, deflated).map_err(|error| error.to_string())?;
        zip.write_all(&bytes)
            .map_err(|error| format!("寫入 {SETTINGS_NAME} 失敗：{error}"))?;
    }
    if let Some(remote) = contents.remote_export {
        zip.start_file(REMOTE_NAME, deflated).map_err(|error| error.to_string())?;
        zip.write_all(remote.as_bytes())
            .map_err(|error| format!("寫入 {REMOTE_NAME} 失敗：{error}"))?;
    }
    if let Some(snapshot) = contents.library_snapshot {
        add(&mut zip, LIBRARY_NAME, snapshot)?;
    }
    if let Some(favourites) = favourites {
        add(&mut zip, FAVOURITES_NAME, &favourites)?;
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

fn stage_path(data_dir: &Path, part: Part) -> PathBuf {
    let directory = pending_directory(data_dir);
    match part {
        Part::Settings => directory.join(SETTINGS_NAME),
        Part::Library => directory.join(LIBRARY_NAME),
        Part::Favourites => directory.join(FAVOURITES_NAME),
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
    }
}

/// Validates the file at `staged` and, when it is good, keeps it as the staged `part`.
fn accept(data_dir: &Path, part: Part, staged: &Path) -> Result<(), String> {
    validate(part, staged).inspect_err(|_| {
        let _ = fs::remove_file(staged);
    })?;
    let target = stage_path(data_dir, part);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let _ = fs::remove_file(&target);
    fs::rename(staged, &target).map_err(|error| error.to_string())
}

/// An entry of the old layout (`background/<file>`) that is ignored; unsafe names are not recognised and refuse the file.
fn is_legacy_background_entry(entry: &str) -> bool {
    let Some(name) = entry.strip_prefix(&format!("{LEGACY_BACKGROUND_DIRECTORY}/")) else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 128
        && !name.contains(['/', '\\', ':'])
        && name != "."
        && name != ".."
        && !name.starts_with('.')
}

/// Stages a backup zip. Returns the parts it held.
fn stage_zip(data_dir: &Path, archive: &Path, platform: &str) -> Result<(Vec<Part>, bool), String> {
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
    let mut plan: Vec<(usize, Part, u64)> = Vec::new();
    let mut remote_entry: Option<usize> = None;
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|error| error.to_string())?;
        let name = entry.name().to_owned();
        if entry.is_dir() {
            continue;
        }
        let (part, limit) = match name.as_str() {
            MANIFEST_NAME => continue,
            REMOTE_NAME | LEGACY_REMOTE_NAME => {
                remote_entry = Some(index);
                continue;
            }
            SETTINGS_NAME => (Part::Settings, MAX_SETTINGS_BYTES),
            LIBRARY_NAME => (Part::Library, MAX_LIBRARY_BYTES),
            FAVOURITES_NAME => (Part::Favourites, MAX_FAVOURITES_BYTES),
            _ if is_legacy_background_entry(&name) => continue,
            _ => return Err(format!("備份檔含有不認得的項目：{name}")),
        };
        plan.push((index, part, limit));
    }
    if plan.is_empty() && remote_entry.is_none() {
        return Err("備份檔沒有任何可匯入的內容。".to_owned());
    }
    let directory = pending_directory(data_dir);
    fs::create_dir_all(directory.join(".incoming")).map_err(|error| error.to_string())?;
    let mut parts: Vec<Part> = Vec::new();
    let result = (|| -> Result<(), String> {
        for (index, part, limit) in &plan {
            let mut entry = zip.by_index(*index).map_err(|error| error.to_string())?;
            if entry.size() > *limit {
                return Err("備份檔內的檔案超過大小上限。".to_owned());
            }
            let staged = directory.join(".incoming").join(format!("{index}.part"));
            copy_limited(&mut entry, &staged, *limit)?;
            accept(data_dir, *part, &staged)?;
            if !parts.contains(part) {
                parts.push(*part);
            }
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(directory.join(".incoming"));
    result?;
    // The accounts wait, sealed, for the passphrase the user is asked for next.
    let mut has_remote = false;
    if let Some(index) = remote_entry {
        let mut entry = zip.by_index(index).map_err(|error| error.to_string())?;
        if entry.size() > MAX_REMOTE_BYTES {
            return Err("備份檔內的檔案超過大小上限。".to_owned());
        }
        copy_limited(&mut entry, &directory.join(REMOTE_NAME), MAX_REMOTE_BYTES)?;
        has_remote = true;
    }
    Ok((parts, has_remote))
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
    let mut remote_waiting = false;
    let parts = if is_zip {
        let (parts, has_remote) = stage_zip(data_dir, source, platform)?;
        remote_waiting = has_remote;
        parts
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
        accept(data_dir, part, &staged)?;
        vec![part]
    };
    let pending = mark_pending(data_dir, &parts)?;
    Ok(ImportResult {
        imported: parts.iter().map(|part| part.id().to_owned()).collect(),
        pending,
        remote_accounts: remote_waiting.then(|| "needsPassphrase".to_owned()),
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

/// Exports the parts the user ticked into one zip chosen with the system file dialog.
#[tauri::command]
pub async fn backup_export(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, crate::AppState>,
    selection: ExportSelection,
) -> Result<Option<ExportResult>, String> {
    selection.validate()?;
    let data_dir = crate::user_data_directory(&state)?;
    // The remote accounts go into the backup, passwords included, only under the passphrase the user typed.
    let remote_export = if selection.accounts {
        let passphrase = selection.passphrase.clone().unwrap_or_default();
        let (settings, data_dir) = (state.settings.clone(), data_dir.clone());
        Some(
            blocking(move || {
                let vault = player_remote::account::CredentialVault::open(
                    &data_dir.join(crate::remote_accounts::KEY_FILE_NAME),
                )
                .map_err(|error| error.to_string())?;
                crate::remote_accounts::export_text(&settings, &vault, &passphrase)
            })
            .await?,
        )
    } else {
        None
    };
    let _work = if selection.library {
        Some(crate::enter_database_work(&state)?)
    } else {
        None
    };
    let work = work_directory(&app)?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let snapshot = work.join(format!("{token}.sqlite3"));
    let archive = work.join(format!("{token}.zip"));
    let epoch = now_seconds();
    let file_name = backup_file_name(epoch);
    let snapshot_made = if selection.library {
        let database = state
            .database
            .as_ref()
            .ok_or_else(|| "曲庫資料庫尚未開啟。".to_owned())?;
        database
            .snapshot_to(&snapshot)
            .map_err(|error| format!("無法複製曲庫資料庫：{error}"))?;
        true
    } else {
        false
    };
    let build = {
        let (data_dir, snapshot, archive) = (data_dir.clone(), snapshot.clone(), archive.clone());
        let selection = selection.clone();
        blocking(move || {
            write_backup_zip(
                &data_dir,
                &ZipContents {
                    library_snapshot: snapshot_made.then_some(snapshot.as_path()),
                    settings: selection.settings,
                    favourites: selection.favourites,
                    remote_export: remote_export.as_deref(),
                },
                &archive,
                env!("MOE_BUILD_VERSION"),
                epoch,
            )
        })
        .await
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
        .set_title("匯出備份")
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
        let mut remote_accounts: Option<String> = None;
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
            remote_accounts = remote_accounts.or(result.remote_accounts);
        }
        Ok(Some(ImportResult { imported, pending, remote_accounts }))
    })
    .await
}

/// Whether remote accounts from an import still wait for their passphrase.
#[tauri::command]
pub fn backup_remote_pending(state: tauri::State<'_, crate::AppState>) -> bool {
    crate::user_data_directory(&state)
        .map(|data_dir| pending_directory(&data_dir).join(REMOTE_NAME).is_file())
        .unwrap_or(false)
}

/// Opens the waiting remote accounts with `passphrase` and adds them: into the staged `settings.json` of a backup that
/// waits for the next start, else into the current settings. A wrong passphrase leaves everything waiting.
#[tauri::command]
pub async fn backup_remote_accounts_import(
    state: tauri::State<'_, crate::AppState>,
    passphrase: String,
) -> Result<crate::remote_accounts::RemoteAccountsImportResult, String> {
    let data_dir = crate::user_data_directory(&state)?;
    let settings = state.settings.clone();
    blocking(move || {
        let waiting = pending_directory(&data_dir).join(REMOTE_NAME);
        let text = fs::read_to_string(&waiting).map_err(|_| "沒有等待輸入密碼的遠端帳號。".to_owned())?;
        let vault = player_remote::account::CredentialVault::open(&data_dir.join(crate::remote_accounts::KEY_FILE_NAME))
            .map_err(|error| error.to_string())?;
        let staged_settings = pending_directory(&data_dir).join(SETTINGS_NAME);
        let result = if read_pending(&data_dir).parts.iter().any(|part| part == "settings") && staged_settings.is_file() {
            merge_into_staged_settings(&staged_settings, &vault, &text, &passphrase)?
        } else {
            crate::remote_accounts::import_text(&settings, &vault, &text, &passphrase)?
        };
        let _ = fs::remove_file(&waiting);
        Ok(result)
    })
    .await
}

/// Forgets remote accounts that were waiting for a passphrase.
#[tauri::command]
pub fn backup_remote_accounts_discard(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let data_dir = crate::user_data_directory(&state)?;
    let _ = fs::remove_file(pending_directory(&data_dir).join(REMOTE_NAME));
    Ok(())
}

/// Adds the accounts of an export to the `settings.json` that waits for the next start (same id: replaced).
fn merge_into_staged_settings(
    staged: &Path,
    vault: &player_remote::account::CredentialVault,
    text: &str,
    passphrase: &str,
) -> Result<crate::remote_accounts::RemoteAccountsImportResult, String> {
    let accounts = vault.import(text, passphrase).map_err(|error| error.to_string())?;
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(staged).map_err(|error| error.to_string())?)
            .map_err(|error| format!("settings.json 無法讀取：{error}"))?;
    let object = json
        .as_object_mut()
        .ok_or_else(|| "settings.json 的格式不正確。".to_owned())?;
    let list = object
        .entry("remoteAccounts")
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let list = list
        .as_array_mut()
        .ok_or_else(|| "settings.json 的遠端帳號格式不正確。".to_owned())?;
    let (mut added, mut updated) = (0, 0);
    for account in &accounts {
        let value = serde_json::to_value(account).map_err(|error| error.to_string())?;
        match list.iter().position(|item| item.get("id").and_then(serde_json::Value::as_str) == Some(account.id.as_str())) {
            Some(index) => {
                list[index] = value;
                updated += 1;
            }
            None => {
                list.push(value);
                added += 1;
            }
        }
    }
    let temporary = staged.with_extension("json.merge");
    fs::write(&temporary, serde_json::to_vec_pretty(&json).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    validate(Part::Settings, &temporary).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })?;
    fs::rename(&temporary, staged).map_err(|error| error.to_string())?;
    Ok(crate::remote_accounts::RemoteAccountsImportResult { added, updated })
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
        .set_title("匯入備份")
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

    fn selection(library: bool, settings: bool, accounts: bool, favourites: bool) -> ExportSelection {
        ExportSelection {
            library,
            settings,
            accounts,
            favourites,
            passphrase: accounts.then(|| "backup passphrase".to_owned()),
        }
    }

    /// Exports `data` the way the command does for `selection`; `vault` seals the accounts when they are ticked.
    fn export_selected(
        scratch: &Scratch,
        data: &Path,
        selection: &ExportSelection,
        vault: Option<&player_remote::account::CredentialVault>,
    ) -> Result<PathBuf, String> {
        selection.validate()?;
        let snapshot = scratch.0.join("snapshot.sqlite3");
        let _ = fs::remove_file(&snapshot);
        if selection.library {
            let database = player_db::Database::open(data.join(LIBRARY_NAME)).unwrap();
            database.snapshot_to(&snapshot).unwrap();
        }
        let remote = if selection.accounts {
            let settings = SettingsStore::open(data.join(SETTINGS_NAME), AppSettings::default()).unwrap();
            Some(crate::remote_accounts::export_text(&settings, vault.unwrap(), selection.passphrase.as_deref().unwrap())?)
        } else {
            None
        };
        let archive = scratch.0.join("backup.zip");
        let _ = fs::remove_file(&archive);
        write_backup_zip(
            data,
            &ZipContents {
                library_snapshot: selection.library.then_some(snapshot.as_path()),
                settings: selection.settings,
                favourites: selection.favourites,
                remote_export: remote.as_deref(),
            },
            &archive,
            "26.10.19",
            1_792_400_000,
        )?;
        Ok(archive)
    }

    fn export(scratch: &Scratch, data: &Path) -> PathBuf {
        export_selected(scratch, data, &selection(true, true, false, true), None).unwrap()
    }

    fn zip_names(archive: &Path) -> Vec<String> {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        let mut names: Vec<String> = (0..zip.len()).map(|index| zip.by_index(index).unwrap().name().to_owned()).collect();
        names.sort();
        names
    }

    fn zip_text(archive: &Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        let mut text = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut text).unwrap();
        text
    }

    fn manifest_parts(archive: &Path) -> Vec<String> {
        serde_json::from_str::<Manifest>(&zip_text(archive, MANIFEST_NAME)).unwrap().parts
    }

    /// Every entry of the zip, decompressed.
    fn all_entry_bytes(archive: &Path) -> Vec<(String, Vec<u8>)> {
        let mut zip = zip::ZipArchive::new(fs::File::open(archive).unwrap()).unwrap();
        (0..zip.len())
            .map(|index| {
                let mut entry = zip.by_index(index).unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (entry.name().to_owned(), bytes)
            })
            .collect()
    }

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle.as_bytes())
    }

    #[test]
    fn the_export_holds_only_what_was_ticked_and_the_manifest_lists_it() {
        let scratch = Scratch::new("export");
        let data = data_folder(&scratch, "a");
        let all = export(&scratch, &data);
        assert_eq!(
            zip_names(&all),
            ["MyLove.m3u8", "manifest.json", "moemusicplayer.sqlite3", "settings.json"],
            "no covers, logs or background image"
        );
        assert_eq!(manifest_parts(&all), ["settings", "library", "favourites"]);
        assert_eq!(backup_file_name(1_792_400_000), "MoeMusicPlayer-backup-20261019-085320.zip");

        let only_library = export_selected(&scratch, &data, &selection(true, false, false, false), None).unwrap();
        assert_eq!(zip_names(&only_library), ["manifest.json", "moemusicplayer.sqlite3"]);
        assert_eq!(manifest_parts(&only_library), ["library"]);

        let only_settings = export_selected(&scratch, &data, &selection(false, true, false, false), None).unwrap();
        assert_eq!(zip_names(&only_settings), ["manifest.json", "settings.json"]);
        assert_eq!(manifest_parts(&only_settings), ["settings"]);

        let only_favourites = export_selected(&scratch, &data, &selection(false, false, false, true), None).unwrap();
        assert_eq!(zip_names(&only_favourites), ["MyLove.m3u8", "manifest.json"]);
    }

    #[test]
    fn nothing_ticked_or_a_short_passphrase_is_refused() {
        let scratch = Scratch::new("refused");
        let data = data_folder(&scratch, "a");
        assert!(export_selected(&scratch, &data, &selection(false, false, false, false), None).is_err());
        let mut short = selection(false, true, true, false);
        short.passphrase = Some("1234567".to_owned());
        assert!(short.validate().unwrap_err().contains('8'));
        short.passphrase = None;
        assert!(short.validate().is_err());
        // A missing favourites file leaves nothing to export.
        fs::remove_file(data.join(FAVOURITES_NAME)).unwrap();
        assert!(export_selected(&scratch, &data, &selection(false, false, false, true), None).is_err());
    }

    #[test]
    fn importing_the_zip_replaces_everything_at_the_next_start_and_keeps_the_old_files() {
        let scratch = Scratch::new("zip");
        let source = data_folder(&scratch, "source");
        let archive = export(&scratch, &source);
        let target = data_folder(&scratch, "target");
        let result = stage_import(&target, &archive, platform_name()).unwrap();
        assert_eq!(result.imported, ["settings", "library", "favourites"]);
        // Nothing is touched while the player runs.
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("target"));
        assert_eq!(pending_parts(&target), result.pending);
        let applied = apply_pending_import(&target).unwrap();
        assert_eq!(applied.len(), 3);
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("source"));
        assert_eq!(fs::read_to_string(target.join("background").join("wall.jpg")).unwrap(), "jpg-target", "the background is never touched");
        assert!(fs::read_to_string(target.join(BEFORE_IMPORT_DIRECTORY).join(FAVOURITES_NAME)).unwrap().contains("target"));
        assert!(target.join("album-cover").join("abc.jpg").is_file(), "covers are never replaced");
        assert!(!target.join(IMPORT_DIRECTORY).exists());
        player_db::Database::open_checked(target.join(LIBRARY_NAME)).expect("the imported library opens");
        assert!(apply_pending_import(&target).unwrap().is_empty(), "nothing is applied twice");
    }

    #[test]
    fn a_zip_with_only_one_part_replaces_only_that_part() {
        let scratch = Scratch::new("partial");
        let source = data_folder(&scratch, "source");
        let archive = export_selected(&scratch, &source, &selection(false, false, false, true), None).unwrap();
        let target = data_folder(&scratch, "target");
        let target_settings = fs::read(target.join(SETTINGS_NAME)).unwrap();
        let result = stage_import(&target, &archive, platform_name()).unwrap();
        assert_eq!(result.imported, ["favourites"]);
        apply_pending_import(&target).unwrap();
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("source"));
        assert_eq!(fs::read(target.join(SETTINGS_NAME)).unwrap(), target_settings, "the settings stay");
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
        // A stand-alone account export is no longer recognised.
        let old_accounts = scratch.0.join("accounts.moeremote");
        fs::write(&old_accounts, b"sealed text").unwrap();
        assert!(stage_import(&target, &old_accounts, platform_name()).is_err());
        assert!(pending_parts(&target).is_empty());
        assert!(!pending_directory(&target).join(REMOTE_NAME).exists());
        assert!(apply_pending_import(&target).unwrap().is_empty());
        assert!(fs::read_to_string(target.join(FAVOURITES_NAME)).unwrap().contains("target"));
    }

    fn manifest_json(parts: Vec<String>) -> String {
        serde_json::to_string(&Manifest {
            app: APP_NAME.to_owned(),
            format_version: FORMAT_VERSION,
            platform: platform_name().to_owned(),
            build_version: "x".to_owned(),
            created_at_utc: "x".to_owned(),
            parts,
        })
        .unwrap()
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
                zip.write_all(manifest_json(Vec::new()).as_bytes()).unwrap();
                zip.start_file(entry, options).unwrap();
                zip.write_all(b"x").unwrap();
                zip.finish().unwrap();
            }
            assert!(stage_import(&target, &path, platform_name()).is_err(), "{entry}");
        }
        // A zip with a manifest and nothing else has nothing to import.
        let empty = scratch.0.join("empty.zip");
        {
            let mut zip = zip::ZipWriter::new(fs::File::create(&empty).unwrap());
            zip.start_file(MANIFEST_NAME, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(manifest_json(Vec::new()).as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        assert!(stage_import(&target, &empty, platform_name()).is_err());
        assert!(pending_parts(&target).is_empty());
    }

    #[test]
    fn an_old_backup_with_the_background_image_is_still_accepted_and_the_image_is_ignored() {
        let scratch = Scratch::new("legacy-background");
        let source = data_folder(&scratch, "source");
        let new_archive = export(&scratch, &source);
        let old = scratch.0.join("old.zip");
        {
            let mut zip = zip::ZipWriter::new(fs::File::create(&old).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, bytes) in all_entry_bytes(&new_archive) {
                zip.start_file(name, options).unwrap();
                zip.write_all(&bytes).unwrap();
            }
            zip.start_file("background/wall.jpg", options).unwrap();
            zip.write_all(b"jpg-old-backup").unwrap();
            zip.finish().unwrap();
        }
        let target = data_folder(&scratch, "target");
        let result = stage_import(&target, &old, platform_name()).unwrap();
        assert_eq!(result.imported, ["settings", "library", "favourites"]);
        apply_pending_import(&target).unwrap();
        assert_eq!(fs::read_to_string(target.join("background").join("wall.jpg")).unwrap(), "jpg-target");
    }

    /// A data folder with two remote accounts (`pw-one`, `pw-two`) sealed under this machine's key; returns the vault.
    fn data_with_accounts(scratch: &Scratch, marker: &str) -> (PathBuf, player_remote::account::CredentialVault) {
        use crate::remote_accounts::{save_account, RemoteAccountInput};
        use player_remote::account::{CredentialVault, RemoteProtocol, TlsMode};
        let data = data_folder(scratch, marker);
        let store = SettingsStore::open(data.join(SETTINGS_NAME), AppSettings::default()).unwrap();
        let vault = CredentialVault::open(&data.join(crate::remote_accounts::KEY_FILE_NAME)).unwrap();
        for (name, password) in [("NAS 一", "pw-one"), ("NAS 二", "pw-two")] {
            save_account(
                &store,
                &vault,
                RemoteAccountInput {
                    id: None,
                    name: name.to_owned(),
                    protocol: RemoteProtocol::Webdav,
                    host: "nas.local".to_owned(),
                    port: None,
                    path: "/music".to_owned(),
                    username: "me".to_owned(),
                    password: Some(password.to_owned()),
                    tls: TlsMode::Verify,
                    only_on_wifi: false,
                },
            )
            .unwrap();
        }
        (data, vault)
    }

    #[test]
    fn no_password_is_readable_in_any_entry_and_the_accounts_only_travel_under_a_passphrase() {
        let scratch = Scratch::new("remote-export");
        let (data, vault) = data_with_accounts(&scratch, "a");
        let without = export_selected(&scratch, &data, &selection(true, true, false, true), Some(&vault)).unwrap();
        assert!(!zip_names(&without).contains(&REMOTE_NAME.to_owned()), "accounts not ticked, no account entry");
        assert_eq!(manifest_parts(&without), ["settings", "library", "favourites"]);
        let settings = zip_text(&without, SETTINGS_NAME);
        assert!(settings.contains("nas.local"), "the accounts themselves stay in settings.json");
        assert!(!settings.contains("\"password\""), "but never a password, not even a sealed one: {settings}");
        // The live settings.json still has its sealed passwords.
        assert!(fs::read_to_string(data.join(SETTINGS_NAME)).unwrap().contains("\"password\": \"v1."));

        let with = export_selected(&scratch, &data, &selection(true, true, true, true), Some(&vault)).unwrap();
        assert!(zip_names(&with).contains(&REMOTE_NAME.to_owned()));
        assert_eq!(manifest_parts(&with), ["settings", "library", "accounts", "favourites"]);
        let sealed = zip_text(&with, REMOTE_NAME);
        for secret in ["pw-one", "pw-two", "nas.local", "NAS 一"] {
            assert!(!sealed.contains(secret), "{secret} is readable in the account entry");
        }
        assert!(!zip_text(&with, SETTINGS_NAME).contains("\"password\""));
        // Neither the entries nor the raw zip bytes show a password.
        for archive in [&without, &with] {
            for (name, bytes) in all_entry_bytes(archive) {
                for secret in ["pw-one", "pw-two"] {
                    assert!(!contains(&bytes, secret), "{secret} is readable in {name}");
                }
            }
            let raw = fs::read(archive).unwrap();
            assert!(!contains(&raw, "pw-one") && !contains(&raw, "pw-two"));
        }
    }

    #[test]
    fn the_accounts_alone_can_be_exported_and_come_back_into_the_current_settings() {
        let scratch = Scratch::new("accounts-only");
        let (data_a, vault_a) = data_with_accounts(&scratch, "a");
        let archive = export_selected(&scratch, &data_a, &selection(false, false, true, false), Some(&vault_a)).unwrap();
        assert_eq!(zip_names(&archive), ["manifest.json", REMOTE_NAME]);
        assert_eq!(manifest_parts(&archive), ["accounts"]);

        let data_b = data_folder(&scratch, "b");
        let result = stage_import(&data_b, &archive, platform_name()).unwrap();
        assert!(result.imported.is_empty());
        assert_eq!(result.remote_accounts.as_deref(), Some("needsPassphrase"));
        // No settings wait for the next start, so the accounts go straight into the current settings.
        let settings_b = SettingsStore::open(data_b.join(SETTINGS_NAME), AppSettings::default()).unwrap();
        let vault_b = player_remote::account::CredentialVault::open(&data_b.join(crate::remote_accounts::KEY_FILE_NAME)).unwrap();
        let text = fs::read_to_string(pending_directory(&data_b).join(REMOTE_NAME)).unwrap();
        assert!(crate::remote_accounts::import_text(&settings_b, &vault_b, &text, "wrong passphrase").is_err());
        assert!(pending_directory(&data_b).join(REMOTE_NAME).is_file(), "a wrong passphrase leaves them waiting");
        let added = crate::remote_accounts::import_text(&settings_b, &vault_b, &text, "backup passphrase").unwrap();
        assert_eq!(added.added, 2);
        let accounts = settings_b.snapshot().unwrap().remote_accounts;
        assert_eq!(&*vault_b.password_of(&accounts[0]).unwrap().unwrap(), "pw-one");
    }

    #[test]
    fn restoring_on_another_device_brings_the_accounts_back_after_the_passphrase() {
        let scratch = Scratch::new("remote-restore");
        let (data_a, vault_a) = data_with_accounts(&scratch, "a");
        let archive = export_selected(&scratch, &data_a, &selection(true, true, true, true), Some(&vault_a)).unwrap();

        // Another device: its own data folder and its own device key.
        let data_b = data_folder(&scratch, "b");
        let vault_b = player_remote::account::CredentialVault::open(&data_b.join(crate::remote_accounts::KEY_FILE_NAME)).unwrap();
        let result = stage_import(&data_b, &archive, platform_name()).unwrap();
        assert_eq!(result.remote_accounts.as_deref(), Some("needsPassphrase"));
        let waiting = pending_directory(&data_b).join(REMOTE_NAME);
        assert!(waiting.is_file());

        // The staged settings already list the accounts, without passwords.
        let staged = pending_directory(&data_b).join(SETTINGS_NAME);
        let staged_text = fs::read_to_string(&staged).unwrap();
        assert!(staged_text.contains("nas.local") && !staged_text.contains("\"password\""));

        let text = fs::read_to_string(&waiting).unwrap();
        let wrong = merge_into_staged_settings(&staged, &vault_b, &text, "wrong passphrase").unwrap_err();
        assert!(wrong.contains("密碼錯誤"), "{wrong}");
        assert!(!fs::read_to_string(&staged).unwrap().contains("\"password\""), "a wrong passphrase changes nothing");
        let merged = merge_into_staged_settings(&staged, &vault_b, &text, "backup passphrase").unwrap();
        assert_eq!(merged, crate::remote_accounts::RemoteAccountsImportResult { added: 0, updated: 2 });

        // At the next start the staged settings are put in place; the passwords open with the new device's key only.
        apply_pending_import(&data_b).unwrap();
        let settings = SettingsStore::open(data_b.join(SETTINGS_NAME), AppSettings::default()).unwrap();
        let accounts = settings.snapshot().unwrap().remote_accounts;
        assert_eq!(accounts.len(), 2);
        assert_eq!(&*vault_b.password_of(&accounts[0]).unwrap().unwrap(), "pw-one");
        assert_eq!(&*vault_b.password_of(&accounts[1]).unwrap().unwrap(), "pw-two");
        assert!(vault_a.password_of(&accounts[0]).is_err(), "sealed for the new device, not the old one");
        assert!(!fs::read_to_string(data_b.join(SETTINGS_NAME)).unwrap().contains("pw-"));
    }
}
