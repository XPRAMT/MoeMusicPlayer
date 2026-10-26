//! The font pickers (ADR 0042): the installed fonts, listed from the SQLite cache at once and refreshed in the
//! background only when a picker opens (never at startup). The font files are the truth; the cache holds only
//! their names and metadata, keyed by each file's size and modification time.
//!
//! Each listed family also gets a name subset (a tiny font with only the glyphs of its display name) in the UserData
//! folder `font-subsets`, made in the background after the list is refreshed; a picker row loads it instead of the
//! whole font file. The subsets are only a cache of the font files: they are remade when the source file or the
//! display name changes and deleted when the family is gone.

use player_db::{CachedFontFace, Database, FontFileStamp, FontSubsetRecord};
use player_fonts::{FaceInfo, FaceNames, FileStamp, FontFamily, FontRegion, Scripts};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

/// Sent when a background refresh ends: the current list and whether it differs from the cached one.
pub const FONTS_UPDATED_EVENT: &str = "fonts-list-updated";

static REFRESHING: AtomicBool = AtomicBool::new(false);
/// The last list built (any region; only its families and faces are used to resolve a setting).
static LAST_CATALOG: Mutex<Option<Arc<Vec<FontFamily>>>> = Mutex::new(None);

/// The UserData folder of the name subsets (the asset protocol may serve only this folder's files).
const SUBSET_FOLDER: &str = "font-subsets";
/// Families whose subsets are made before the list is sent again.
const SUBSET_BATCH: usize = 32;

/// A picker entry as the page gets it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListedFont {
    #[serde(flatten)]
    family: FontFamily,
    /// The family's name subset file, once made; without it a row loads the family by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    subset: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontList {
    fonts: Vec<ListedFont>,
    /// A background refresh is running; `fonts-list-updated` follows.
    refreshing: bool,
    /// Android: the family the system's default text is drawn in (for 「預設（…）」); `None` elsewhere or unknown.
    system_default: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FontListUpdated {
    fonts: Vec<ListedFont>,
    changed: bool,
    system_default: Option<String>,
}

impl FontListUpdated {
    fn unchanged() -> Self {
        FontListUpdated {
            fonts: Vec::new(),
            changed: false,
            system_default: None,
        }
    }
}

fn to_cached(path: &str, face: &FaceInfo) -> CachedFontFace {
    CachedFontFace {
        path: path.to_owned(),
        face_index: face.face_index,
        names_json: serde_json::to_string(&face.names).unwrap_or_else(|_| "{}".to_owned()),
        weight: face.weight,
        italic: face.italic,
        scripts: face.scripts.0,
        hidden: face.hidden,
    }
}

fn from_cached(face: CachedFontFace) -> Option<(String, FaceInfo)> {
    let names: FaceNames = serde_json::from_str(&face.names_json).ok()?;
    Some((
        face.path,
        FaceInfo {
            face_index: face.face_index,
            names,
            weight: face.weight,
            italic: face.italic,
            scripts: Scripts(face.scripts),
            hidden: face.hidden,
        },
    ))
}

fn to_db_stamp(stamp: &FileStamp) -> FontFileStamp {
    FontFileStamp {
        path: stamp.path.clone(),
        size_bytes: stamp.size_bytes,
        modified_at_utc_ms: stamp.modified_ms,
        parse_version: player_fonts::PARSE_VERSION,
    }
}

/// A cached file as `diff_files` compares it. A file parsed by another `PARSE_VERSION` gets a modification time no
/// file has, so it counts as changed and is reparsed (and still counts as removed when it is gone).
fn from_db_stamp(stamp: FontFileStamp) -> FileStamp {
    let current = stamp.parse_version == player_fonts::PARSE_VERSION;
    FileStamp {
        path: stamp.path,
        size_bytes: stamp.size_bytes,
        modified_ms: if current {
            stamp.modified_at_utc_ms
        } else {
            i64::MIN
        },
    }
}

fn database(state: &crate::AppState) -> Result<&Database, String> {
    state.database.as_ref().ok_or_else(|| {
        state
            .database_error
            .clone()
            .unwrap_or_else(|| "曲庫資料庫尚未開啟。".to_owned())
    })
}

/// The cached fonts in `region` order (the app's natural collation within a group), hidden families included: they
/// still resolve a saved setting, and `player_fonts::visible_families` leaves them out of the pickers.
fn cached_catalog(database: &Database, region: FontRegion) -> Result<Vec<FontFamily>, String> {
    let faces = database
        .font_cache_faces()
        .map_err(|error| error.to_string())?;
    let catalog =
        player_fonts::build_catalog(faces.into_iter().filter_map(from_cached), region, |a, b| {
            database.compare_music_text(a, b)
        });
    if let Ok(mut last) = LAST_CATALOG.lock() {
        *last = Some(Arc::new(catalog.clone()));
    }
    Ok(catalog)
}

/// The OS region (not the interface language): Windows geo name, Android `Locale.getDefault().getCountry()`.
fn region_code(app: &AppHandle) -> String {
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        if let Ok(fonts) = app.media_index().system_fonts() {
            if !fonts.region.is_empty() {
                return fonts.region;
            }
        }
        String::new()
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        player_fonts::system_region().unwrap_or_default()
    }
}

/// The installed font files now (names and stat calls only).
fn current_files(app: &AppHandle) -> Vec<FileStamp> {
    #[allow(unused_mut)]
    let mut paths = player_fonts::system_font_files();
    #[cfg(target_os = "android")]
    {
        use tauri_plugin_media_index::MediaIndexExt;
        if let Ok(fonts) = app.media_index().system_fonts() {
            paths.extend(fonts.paths.into_iter().map(std::path::PathBuf::from));
        }
        paths.sort();
        paths.dedup();
    }
    let _ = app;
    paths
        .iter()
        .filter_map(|path| player_fonts::file_stamp(path))
        .collect()
}

/// Android WebView loads a chosen font through `@font-face` from its file: let the asset protocol serve exactly the
/// listed font files (nothing else is in its scope).
fn allow_font_files(app: &AppHandle, fonts: &[FontFamily]) {
    #[cfg(target_os = "android")]
    {
        let scope = app.asset_protocol_scope();
        for face in fonts.iter().flat_map(|family| &family.faces) {
            let _ = scope.allow_file(&face.path);
        }
    }
    let _ = (app, fonts);
}

fn allow_listed_font_files(app: &AppHandle, fonts: &[ListedFont]) {
    let families: Vec<FontFamily> = fonts.iter().map(|listed| listed.family.clone()).collect();
    allow_font_files(app, &families);
}

fn subset_directory(state: &crate::AppState) -> Option<PathBuf> {
    state
        .settings
        .data_directory()
        .map(|directory| directory.join(SUBSET_FOLDER))
}

/// Let the asset protocol serve the name subsets (only their folder, not its subfolders).
fn allow_subset_directory(app: &AppHandle) {
    let state = app.state::<crate::AppState>();
    if let Some(directory) = subset_directory(&state) {
        let _ = std::fs::create_dir_all(&directory);
        let _ = app
            .asset_protocol_scope()
            .allow_directory(&directory, false);
    }
}

/// The visible families with their subset files.
fn listed_fonts(
    families: Vec<FontFamily>,
    subsets: &[FontSubsetRecord],
    directory: Option<&Path>,
) -> Vec<ListedFont> {
    let files: HashMap<&str, &str> = subsets
        .iter()
        .filter_map(|record| Some((record.family_key.as_str(), record.subset_file.as_deref()?)))
        .collect();
    families
        .into_iter()
        .map(|family| {
            let subset = directory.and_then(|directory| {
                files
                    .get(family.family.to_lowercase().as_str())
                    .map(|file| directory.join(file).to_string_lossy().into_owned())
            });
            ListedFont { family, subset }
        })
        .collect()
}

/// The picker list now: the visible cached families with their subsets.
fn current_list(state: &crate::AppState, region: FontRegion) -> Result<Vec<ListedFont>, String> {
    let database = database(state)?;
    let fonts = player_fonts::visible_families(&cached_catalog(database, region)?);
    let subsets = database.font_subsets().map_err(|error| error.to_string())?;
    Ok(listed_fonts(
        fonts,
        &subsets,
        subset_directory(state).as_deref(),
    ))
}

/// Android: the listed family the system draws its default text in for the region (`sans-serif` and the CJK
/// fallback of `font_fallback.xml` / `fonts.xml`). Elsewhere the page knows its default fonts itself.
fn system_default_family(region: FontRegion, fonts: &[ListedFont]) -> Option<String> {
    #[cfg(target_os = "android")]
    {
        let config = player_fonts::ANDROID_FONT_CONFIGS
            .iter()
            .find_map(|path| std::fs::read_to_string(path).ok())?;
        let font = player_fonts::android_default_font(
            &player_fonts::parse_android_font_config(&config),
            region,
        )?;
        let families: Vec<FontFamily> = fonts.iter().map(|listed| listed.family.clone()).collect();
        player_fonts::family_of_system_font(&families, &font).map(|family| family.family.clone())
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (region, fonts);
        None
    }
}

/// Re-read the font files that are new or changed since they were cached, forget the vanished ones, and send the
/// list to the page when anything changed (or when the cache was empty, so the page leaves its loading state).
async fn refresh(app: AppHandle, region: FontRegion, was_empty: bool) -> Result<(), String> {
    let scan_app = app.clone();
    let current = tauri::async_runtime::spawn_blocking(move || current_files(&scan_app))
        .await
        .map_err(|error| error.to_string())?;
    let cached = crate::run_database_work(app.clone(), |state| {
        database(state)?
            .font_cache_files()
            .map_err(|error| error.to_string())
    })
    .await?;
    let cached: Vec<FileStamp> = cached.into_iter().map(from_db_stamp).collect();
    let diff = player_fonts::diff_files(&cached, &current);
    let changed = !diff.is_empty();
    if changed {
        let to_parse = diff.to_parse.clone();
        let parsed = tauri::async_runtime::spawn_blocking(move || {
            to_parse
                .iter()
                .map(|stamp| {
                    let faces = player_fonts::read_font_file(std::path::Path::new(&stamp.path))
                        .unwrap_or_default();
                    (
                        to_db_stamp(stamp),
                        faces
                            .iter()
                            .map(|face| to_cached(&stamp.path, face))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|error| error.to_string())?;
        let removed = diff.removed.clone();
        crate::run_database_work(app.clone(), move |state| {
            database(state)?
                .font_cache_apply(&removed, &parsed)
                .map_err(|error| error.to_string())
        })
        .await?;
    }
    if changed || was_empty {
        emit_list(&app, region, changed).await?;
    } else {
        let _ = app.emit(FONTS_UPDATED_EVENT, FontListUpdated::unchanged());
    }
    Ok(())
}

/// Send the current list to the pages.
async fn emit_list(app: &AppHandle, region: FontRegion, changed: bool) -> Result<(), String> {
    let fonts =
        crate::run_database_work(app.clone(), move |state| current_list(state, region)).await?;
    allow_listed_font_files(app, &fonts);
    let system_default = system_default_family(region, &fonts);
    let _ = app.emit(
        FONTS_UPDATED_EVENT,
        FontListUpdated {
            fonts,
            changed,
            system_default,
        },
    );
    Ok(())
}

/// One name subset to make: the record it will have (without its file yet) and the name it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SubsetJob {
    record: FontSubsetRecord,
    display_name: String,
    /// The file of the record being replaced (deleted once the new one is written).
    previous_file: Option<String>,
}

/// What the subset folder needs: the families to (re)make, in list order, and the families to forget.
#[derive(Debug, Default, PartialEq, Eq)]
struct SubsetPlan {
    make: Vec<SubsetJob>,
    remove: Vec<String>,
}

/// Compare the listed families (regular face, cached file stamps, display name) with the subset records. A record is
/// current when its source file, size, modification time, face, display name and `SUBSET_VERSION` all match and its
/// file is still there (a face that cannot be subset has no file and stays current until something changes).
fn plan_subsets(
    families: &[FontFamily],
    stamps: &HashMap<String, (u64, i64)>,
    records: &[FontSubsetRecord],
    file_exists: impl Fn(&str) -> bool,
) -> SubsetPlan {
    let existing: HashMap<&str, &FontSubsetRecord> = records
        .iter()
        .map(|record| (record.family_key.as_str(), record))
        .collect();
    let mut plan = SubsetPlan::default();
    let mut listed: HashSet<String> = HashSet::new();
    for family in families {
        let Some(face) = family.faces.first() else {
            continue;
        };
        let Some(&(size, modified)) = stamps.get(&face.path) else {
            continue;
        };
        let key = family.family.to_lowercase();
        if !listed.insert(key.clone()) {
            continue;
        }
        let wanted = FontSubsetRecord {
            family_key: key.clone(),
            source_path: face.path.clone(),
            face_index: face.face_index,
            source_size_bytes: size,
            source_modified_at_utc_ms: modified,
            name_hash: player_fonts::stable_hash(&[&family.display_name]),
            subset_version: player_fonts::SUBSET_VERSION,
            subset_file: None,
        };
        let previous = existing.get(key.as_str()).copied();
        let current = previous.is_some_and(|record| {
            FontSubsetRecord {
                subset_file: None,
                ..record.clone()
            } == wanted
                && record.subset_file.as_deref().is_none_or(&file_exists)
        });
        if !current {
            plan.make.push(SubsetJob {
                record: wanted,
                display_name: family.display_name.clone(),
                previous_file: previous.and_then(|record| record.subset_file.clone()),
            });
        }
    }
    plan.remove = records
        .iter()
        .filter(|record| !listed.contains(&record.family_key))
        .map(|record| record.family_key.clone())
        .collect();
    plan
}

/// The subset file name: a hash of everything the subset is made from, so a remade subset gets a new URL (the
/// WebView never shows a stale cached one).
fn subset_file_name(record: &FontSubsetRecord, display_name: &str, cff: bool) -> String {
    let hash = player_fonts::stable_hash(&[
        &record.family_key,
        &record.source_path,
        &record.face_index.to_string(),
        &record.source_size_bytes.to_string(),
        &record.source_modified_at_utc_ms.to_string(),
        display_name,
        &record.subset_version.to_string(),
    ]);
    format!("{hash}.{}", if cff { "otf" } else { "ttf" })
}

/// Make the subsets of `jobs` in `directory` (each source face read once): the records with their files (`None` for
/// a face that cannot be subset, such as a legacy-encoding font).
fn make_subsets(directory: &Path, jobs: &[SubsetJob]) -> Vec<FontSubsetRecord> {
    let mut faces: HashMap<(String, u32), Option<Vec<u8>>> = HashMap::new();
    jobs.iter()
        .map(|job| {
            let record = &job.record;
            let face = faces
                .entry((record.source_path.clone(), record.face_index))
                .or_insert_with(|| {
                    player_fonts::read_standalone_face(
                        Path::new(&record.source_path),
                        record.face_index,
                    )
                    .ok()
                })
                .as_deref();
            let subset_file = face
                .and_then(|data| player_fonts::name_subset(data, 0, &job.display_name))
                .and_then(|bytes| {
                    let name =
                        subset_file_name(record, &job.display_name, bytes.starts_with(b"OTTO"));
                    let temporary = directory.join(format!("{name}.tmp"));
                    std::fs::write(&temporary, &bytes).ok()?;
                    std::fs::rename(&temporary, directory.join(&name)).ok()?;
                    Some(name)
                });
            FontSubsetRecord {
                subset_file,
                ..record.clone()
            }
        })
        .collect()
}

/// Bring the name subsets in line with the listed families: forget the vanished families, make the missing and
/// stale subsets in batches (the list is sent again after each batch that made one, so rows switch to their subsets
/// as they come) and delete the files no record names.
async fn refresh_subsets(app: AppHandle, region: FontRegion) -> Result<(), String> {
    let (families, stamps, records, directory) =
        crate::run_database_work(app.clone(), move |state| {
            let database = database(state)?;
            let families = player_fonts::visible_families(&cached_catalog(database, region)?);
            let stamps: HashMap<String, (u64, i64)> = database
                .font_cache_files()
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|stamp| (stamp.path, (stamp.size_bytes, stamp.modified_at_utc_ms)))
                .collect();
            let records = database.font_subsets().map_err(|error| error.to_string())?;
            Ok((families, stamps, records, subset_directory(state)))
        })
        .await?;
    let Some(directory) = directory else {
        return Ok(());
    };
    let plan_directory = directory.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        let _ = std::fs::create_dir_all(&plan_directory);
        plan_subsets(&families, &stamps, &records, |file| {
            plan_directory.join(file).is_file()
        })
    })
    .await
    .map_err(|error| error.to_string())?;
    if !plan.remove.is_empty() {
        let removed = plan.remove.clone();
        crate::run_database_work(app.clone(), move |state| {
            database(state)?
                .font_subsets_apply(&removed, &[])
                .map_err(|error| error.to_string())
        })
        .await?;
    }
    for batch in plan.make.chunks(SUBSET_BATCH) {
        let jobs = batch.to_vec();
        let batch_directory = directory.clone();
        let (made, jobs) = tauri::async_runtime::spawn_blocking(move || {
            (make_subsets(&batch_directory, &jobs), jobs)
        })
        .await
        .map_err(|error| error.to_string())?;
        let any_file = made.iter().any(|record| record.subset_file.is_some());
        crate::run_database_work(app.clone(), move |state| {
            database(state)?
                .font_subsets_apply(&[], &made)
                .map_err(|error| error.to_string())
        })
        .await?;
        for previous in jobs.iter().filter_map(|job| job.previous_file.as_ref()) {
            let _ = std::fs::remove_file(directory.join(previous));
        }
        if any_file {
            emit_list(&app, region, true).await?;
        }
    }
    // Files no record names: subsets of vanished or remade families, interrupted writes.
    let referenced: HashSet<String> = crate::run_database_work(app.clone(), |state| {
        database(state)?
            .font_subsets()
            .map_err(|error| error.to_string())
            .map(|records| {
                records
                    .into_iter()
                    .filter_map(|record| record.subset_file)
                    .collect()
            })
    })
    .await?;
    let _ = tauri::async_runtime::spawn_blocking(move || {
        for entry in std::fs::read_dir(&directory)
            .into_iter()
            .flatten()
            .flatten()
        {
            if !referenced.contains(entry.file_name().to_string_lossy().as_ref()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    })
    .await;
    Ok(())
}

/// The fonts for a picker: the cached list at once (empty on the very first open), then a background refresh whose
/// result arrives as `fonts-list-updated`.
#[tauri::command]
pub async fn fonts_list(app: AppHandle) -> Result<FontList, String> {
    let region_app = app.clone();
    let region = tauri::async_runtime::spawn_blocking(move || region_code(&region_app))
        .await
        .map_err(|error| error.to_string())?;
    let region = FontRegion::from_code(&region);
    let fonts =
        crate::run_database_work(app.clone(), move |state| current_list(state, region)).await?;
    allow_listed_font_files(&app, &fonts);
    allow_subset_directory(&app);
    let system_default = system_default_family(region, &fonts);
    if !REFRESHING.swap(true, Ordering::SeqCst) {
        let was_empty = fonts.is_empty();
        let refresh_app = app.clone();
        tauri::async_runtime::spawn(async move {
            match refresh(refresh_app.clone(), region, was_empty).await {
                // The name subsets follow the refreshed list (never at startup: only after a picker opened).
                Ok(()) => {
                    if let Err(error) = refresh_subsets(refresh_app.clone(), region).await {
                        eprintln!("[fonts] subsets failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("[fonts] refresh failed: {error}");
                    let _ = refresh_app.emit(FONTS_UPDATED_EVENT, FontListUpdated::unchanged());
                }
            }
            REFRESHING.store(false, Ordering::SeqCst);
        });
    }
    Ok(FontList {
        fonts,
        refreshing: true,
        system_default,
    })
}

/// The cached family named `family` (the stored setting), with its face files; `None` when it is not (or no longer)
/// installed. A family the pickers hide still resolves. Reads only the cache, never the font files.
pub fn resolve_family(state: &crate::AppState, family: &str) -> Result<Option<FontFamily>, String> {
    let wanted = family.trim().to_lowercase();
    if wanted.is_empty() {
        return Ok(None);
    }
    let last = LAST_CATALOG.lock().ok().and_then(|last| last.clone());
    let catalog = match last {
        Some(catalog) => catalog,
        None => Arc::new(cached_catalog(database(state)?, FontRegion::Other)?),
    };
    Ok(catalog
        .iter()
        .find(|entry| entry.family.to_lowercase() == wanted)
        .cloned())
}

/// The cached family `family` for applying a font at startup (Android `@font-face`), without scanning.
#[tauri::command]
pub async fn fonts_resolve(app: AppHandle, family: String) -> Result<Option<FontFamily>, String> {
    let resolved =
        crate::run_database_work(app.clone(), move |state| resolve_family(state, &family)).await?;
    if let Some(found) = &resolved {
        allow_font_files(&app, std::slice::from_ref(found));
    }
    Ok(resolved)
}

/// Android floating lyrics: the overlay builds its typeface from files, so a `settings.fontFamily` in a
/// `floatingLyricsUpdate` payload is resolved to the family's upright faces (`fontFaces`; empty: the default font).
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub fn attach_floating_font_faces(state: &crate::AppState, payload: &mut serde_json::Value) {
    let Some(settings) = payload
        .get_mut("settings")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let family = settings
        .get("fontFamily")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let faces = family
        .and_then(|family| resolve_family(state, &family).ok().flatten())
        .map(|family| floating_faces(&family))
        .unwrap_or_default();
    settings.insert("fontFaces".to_owned(), serde_json::Value::Array(faces));
}

/// The upright faces of `family` as the overlay takes them (`path`, `faceIndex`, `weight`).
fn floating_faces(family: &FontFamily) -> Vec<serde_json::Value> {
    family
        .faces
        .iter()
        .filter(|face| !face.italic)
        .map(|face| {
            serde_json::json!({
                "path": face.path,
                "faceIndex": face.face_index,
                "weight": face.weight,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use player_fonts::{FamilyFace, FontGroup, LocalizedName};

    #[test]
    fn cached_faces_round_trip_through_the_database_rows() {
        let face = FaceInfo {
            face_index: 2,
            names: FaceNames {
                family: vec![LocalizedName {
                    lang: 0x0404,
                    name: "微軟正黑體".into(),
                }],
                subfamily: vec![],
                full_name: vec![],
            },
            weight: 700,
            italic: true,
            scripts: Scripts(Scripts::TRADITIONAL | Scripts::JAPANESE),
            hidden: true,
        };
        let row = to_cached("C:/Windows/Fonts/msjhbd.ttc", &face);
        assert_eq!((row.scripts, row.hidden), (5, true));
        assert_eq!(
            from_cached(row),
            Some(("C:/Windows/Fonts/msjhbd.ttc".to_owned(), face))
        );
        let broken = CachedFontFace {
            path: "x".into(),
            face_index: 0,
            names_json: "not json".into(),
            weight: 400,
            italic: false,
            scripts: 0,
            hidden: false,
        };
        assert_eq!(from_cached(broken), None);
        let stamp = FileStamp {
            path: "a.ttf".into(),
            size_bytes: 3,
            modified_ms: 9,
        };
        assert_eq!(from_db_stamp(to_db_stamp(&stamp)), stamp);
        // Parsed by an earlier version: changed for `diff_files`, so it is reparsed; gone: removed.
        let mut stale = to_db_stamp(&stamp);
        stale.parse_version = 0;
        let cached = vec![from_db_stamp(stale)];
        let diff = player_fonts::diff_files(&cached, std::slice::from_ref(&stamp));
        assert_eq!((diff.to_parse, diff.unchanged), (vec![stamp.clone()], 0));
        assert_eq!(
            player_fonts::diff_files(&cached, &[]).removed,
            vec!["a.ttf".to_owned()]
        );
    }

    #[test]
    fn floating_lyrics_get_only_the_upright_faces() {
        let family = FontFamily {
            family: "Arial".into(),
            display_name: "Arial".into(),
            group: FontGroup::Latin,
            hidden: false,
            faces: vec![
                FamilyFace {
                    path: "/system/fonts/a.ttf".into(),
                    face_index: 0,
                    weight: 400,
                    italic: false,
                },
                FamilyFace {
                    path: "/system/fonts/ai.ttf".into(),
                    face_index: 0,
                    weight: 400,
                    italic: true,
                },
                FamilyFace {
                    path: "/system/fonts/ab.ttc".into(),
                    face_index: 1,
                    weight: 700,
                    italic: false,
                },
            ],
        };
        assert_eq!(
            floating_faces(&family),
            vec![
                serde_json::json!({"path": "/system/fonts/a.ttf", "faceIndex": 0, "weight": 400}),
                serde_json::json!({"path": "/system/fonts/ab.ttc", "faceIndex": 1, "weight": 700}),
            ]
        );
    }

    fn family(name: &str, display: &str, path: &str) -> FontFamily {
        FontFamily {
            family: name.into(),
            display_name: display.into(),
            group: FontGroup::Traditional,
            hidden: false,
            faces: vec![FamilyFace {
                path: path.into(),
                face_index: 1,
                weight: 400,
                italic: false,
            }],
        }
    }

    fn made(family: &FontFamily, size: u64, file: Option<&str>) -> FontSubsetRecord {
        FontSubsetRecord {
            family_key: family.family.to_lowercase(),
            source_path: family.faces[0].path.clone(),
            face_index: 1,
            source_size_bytes: size,
            source_modified_at_utc_ms: 5,
            name_hash: player_fonts::stable_hash(&[&family.display_name]),
            subset_version: player_fonts::SUBSET_VERSION,
            subset_file: file.map(str::to_owned),
        }
    }

    #[test]
    fn subsets_are_made_for_new_stale_and_missing_files_and_forgotten_for_gone_families() {
        let jhenghei = family("Microsoft JhengHei", "微軟正黑體", "msjh.ttc");
        let legacy = family("Legacy", "文鼎新藝體", "old.ttf");
        let renamed = family("Yu Gothic", "游ゴシック", "yugoth.ttc");
        let fresh = family("Arial", "Arial", "arial.ttf");
        let missing_file = family("Consolas", "Consolas", "consola.ttf");
        let changed = family("SimSun", "宋体", "simsun.ttc");
        let stamps: HashMap<String, (u64, i64)> = [
            "msjh.ttc",
            "old.ttf",
            "yugoth.ttc",
            "arial.ttf",
            "consola.ttf",
        ]
        .into_iter()
        .map(|path| (path.to_owned(), (10, 5)))
        .chain([("simsun.ttc".to_owned(), (11, 5))])
        .collect();
        let mut stale_name = made(&renamed, 10, Some("yu.ttf"));
        stale_name.name_hash = player_fonts::stable_hash(&["Yu Gothic"]);
        let records = vec![
            made(&jhenghei, 10, Some("jh.ttf")),
            made(&legacy, 10, None),
            stale_name,
            made(&missing_file, 10, Some("gone.ttf")),
            made(&changed, 10, Some("sun.ttf")),
            made(&family("Removed", "Removed", "x.ttf"), 10, Some("x.ttf")),
        ];
        let families = vec![
            jhenghei,
            legacy,
            renamed.clone(),
            fresh.clone(),
            missing_file.clone(),
            changed.clone(),
        ];
        let plan = plan_subsets(&families, &stamps, &records, |file| file != "gone.ttf");
        let made_keys: Vec<(&str, Option<&str>)> = plan
            .make
            .iter()
            .map(|job| (job.record.family_key.as_str(), job.previous_file.as_deref()))
            .collect();
        assert_eq!(
            made_keys,
            vec![
                ("yu gothic", Some("yu.ttf")),
                ("arial", None),
                ("consolas", Some("gone.ttf")),
                ("simsun", Some("sun.ttf")),
            ],
            "current subsets and the legacy font without one are left alone"
        );
        assert_eq!(plan.make[3].record.source_size_bytes, 11);
        assert_eq!(plan.make[0].display_name, "游ゴシック");
        assert_eq!(plan.remove, vec!["removed".to_owned()]);
    }

    #[test]
    fn the_list_carries_the_subset_files_and_a_subset_file_names_its_source() {
        let a = family("A", "甲", "a.ttf");
        let b = family("B", "乙", "b.ttf");
        let dir = std::path::Path::new("D:/UserData/font-subsets");
        let listed = listed_fonts(
            vec![a.clone(), b.clone()],
            &[made(&a, 1, Some("a.ttf")), made(&b, 1, None)],
            Some(dir),
        );
        assert_eq!(
            listed[0].subset.as_deref(),
            Some(dir.join("a.ttf").to_string_lossy().as_ref())
        );
        assert_eq!(listed[1].subset, None);
        let json = serde_json::to_value(&listed[0]).unwrap();
        assert_eq!(json["family"], "A");
        assert!(json["subset"].is_string());
        assert!(serde_json::to_value(&listed[1])
            .unwrap()
            .get("subset")
            .is_none());
        let record = made(&a, 1, None);
        let name = subset_file_name(&record, "甲", false);
        assert!(name.ends_with(".ttf"));
        assert!(subset_file_name(&record, "甲", true).ends_with(".otf"));
        assert_ne!(name, subset_file_name(&record, "甲甲", false));
        let mut changed = record.clone();
        changed.source_modified_at_utc_ms += 1;
        assert_ne!(name, subset_file_name(&changed, "甲", false));
    }

    #[test]
    fn a_face_that_cannot_be_read_gets_a_record_without_a_file() {
        let dir = std::env::temp_dir().join(format!("moe-font-subsets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let not_a_font = dir.join("not-a-font.ttf");
        std::fs::write(&not_a_font, b"plain text").unwrap();
        let fake = family("Fake", "Fake", &not_a_font.to_string_lossy());
        let job = SubsetJob {
            record: made(&fake, 10, None),
            display_name: "Fake".into(),
            previous_file: None,
        };
        let records = make_subsets(&dir, &[job]);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].subset_file, None);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "nothing written"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
