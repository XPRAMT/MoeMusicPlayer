//! The font pickers (ADR 0042): the installed fonts, listed from the SQLite cache at once and refreshed in the
//! background only when a picker opens (never at startup). The font files are the truth; the cache holds only
//! their names and metadata, keyed by each file's size and modification time.

use player_db::{CachedFontFace, Database, FontFileStamp};
use player_fonts::{FaceInfo, FaceNames, FileStamp, FontFamily, FontRegion, Scripts};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

/// Sent when a background refresh ends: the current list and whether it differs from the cached one.
pub const FONTS_UPDATED_EVENT: &str = "fonts-list-updated";

static REFRESHING: AtomicBool = AtomicBool::new(false);
/// The last list built (any region; only its families and faces are used to resolve a setting).
static LAST_CATALOG: Mutex<Option<Arc<Vec<FontFamily>>>> = Mutex::new(None);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FontList {
    fonts: Vec<FontFamily>,
    /// A background refresh is running; `fonts-list-updated` follows.
    refreshing: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FontListUpdated {
    fonts: Vec<FontFamily>,
    changed: bool,
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
        use tauri::Manager;
        let scope = app.asset_protocol_scope();
        for face in fonts.iter().flat_map(|family| &family.faces) {
            let _ = scope.allow_file(&face.path);
        }
    }
    let _ = (app, fonts);
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
        let fonts = crate::run_database_work(app.clone(), move |state| {
            cached_catalog(database(state)?, region)
                .map(|catalog| player_fonts::visible_families(&catalog))
        })
        .await?;
        allow_font_files(&app, &fonts);
        let _ = app.emit(FONTS_UPDATED_EVENT, FontListUpdated { fonts, changed });
    } else {
        let _ = app.emit(
            FONTS_UPDATED_EVENT,
            FontListUpdated {
                fonts: Vec::new(),
                changed: false,
            },
        );
    }
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
    let fonts = crate::run_database_work(app.clone(), move |state| {
        cached_catalog(database(state)?, region)
            .map(|catalog| player_fonts::visible_families(&catalog))
    })
    .await?;
    allow_font_files(&app, &fonts);
    if !REFRESHING.swap(true, Ordering::SeqCst) {
        let was_empty = fonts.is_empty();
        let refresh_app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = refresh(refresh_app.clone(), region, was_empty).await {
                eprintln!("[fonts] refresh failed: {error}");
                let _ = refresh_app.emit(
                    FONTS_UPDATED_EVENT,
                    FontListUpdated {
                        fonts: Vec::new(),
                        changed: false,
                    },
                );
            }
            REFRESHING.store(false, Ordering::SeqCst);
        });
    }
    Ok(FontList {
        fonts,
        refreshing: true,
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
}
