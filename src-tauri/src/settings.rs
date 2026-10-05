use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use player_core::{LibraryRoot, MediaLocator, MediaSourceKind, PlaylistId, SourceId};
use serde::{Deserialize, Serialize};

const SETTINGS_SCHEMA_VERSION: u32 = 6;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    Off,
    All,
    One,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeSettings {
    pub background_hex: String,
    pub accent_hex: String,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            background_hex: "#000000".to_owned(),
            accent_hex: "#55D9FF".to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricsPreferences {
    pub show_translation: bool,
    pub show_romanization: bool,
    pub inactive_opacity_percent: u8,
    pub primary_font_size_px: u8,
    pub auxiliary_font_size_px: u8,
    #[serde(default = "default_line_gap_px")]
    pub line_gap_px: u8,
    #[serde(default = "default_lyrics_text_effect")]
    pub text_effect: LyricsTextEffect,
}

fn default_line_gap_px() -> u8 {
    24
}

impl Default for LyricsPreferences {
    fn default() -> Self {
        Self {
            show_translation: false,
            show_romanization: false,
            inactive_opacity_percent: 70,
            primary_font_size_px: 14,
            auxiliary_font_size_px: 10,
            line_gap_px: default_line_gap_px(),
            text_effect: default_lyrics_text_effect(),
        }
    }
}

impl LyricsPreferences {
    fn validate(&self) -> Result<(), SettingsError> {
        if !(10..=100).contains(&self.inactive_opacity_percent) {
            return Err(SettingsError::InvalidData(
                "inactiveOpacityPercent must be between 10 and 100".into(),
            ));
        }
        if !(12..=36).contains(&self.primary_font_size_px) {
            return Err(SettingsError::InvalidData(
                "primaryFontSizePx must be between 12 and 36".into(),
            ));
        }
        if !(9..=24).contains(&self.auxiliary_font_size_px) {
            return Err(SettingsError::InvalidData(
                "auxiliaryFontSizePx must be between 9 and 24".into(),
            ));
        }
        if self.line_gap_px > 64 {
            return Err(SettingsError::InvalidData(
                "lineGapPx must be between 0 and 64".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverCornerStyle {
    Rounded,
    Square,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineStyle {
    Line,
    Edge,
}

impl<'de> Deserialize<'de> for TimelineStyle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            "edge" => Ok(Self::Edge),
            // Default line look; also accept legacy bar/minimal from earlier WIP builds.
            "line" | "bar" | "minimal" => Ok(Self::Line),
            other => Err(serde::de::Error::unknown_variant(other, &["line", "edge"])),
        }
    }
}

impl Default for TimelineStyle {
    fn default() -> Self {
        Self::Line
    }
}

fn default_timeline_style() -> TimelineStyle {
    TimelineStyle::Line
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsTextEffect {
    Shadow,
    Stroke,
    None,
}

impl Default for LyricsTextEffect {
    fn default() -> Self {
        Self::Shadow
    }
}

fn default_lyrics_text_effect() -> LyricsTextEffect {
    LyricsTextEffect::Shadow
}

impl Default for CoverCornerStyle {
    fn default() -> Self {
        Self::Rounded
    }
}

fn default_cover_corner_style() -> CoverCornerStyle {
    CoverCornerStyle::Rounded
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlayingAppearancePreferences {
    pub background_blur_px: u8,
    #[serde(default = "default_background_brightness_percent")]
    pub background_brightness_percent: u8,
    #[serde(default = "default_cover_corner_style")]
    pub cover_corner_style: CoverCornerStyle,
    #[serde(default = "default_timeline_style")]
    pub timeline_style: TimelineStyle,
}

fn default_background_brightness_percent() -> u8 {
    40
}

impl Default for NowPlayingAppearancePreferences {
    fn default() -> Self {
        Self {
            background_blur_px: 20,
            background_brightness_percent: 40,
            cover_corner_style: CoverCornerStyle::Rounded,
            timeline_style: TimelineStyle::Line,
        }
    }
}

impl NowPlayingAppearancePreferences {
    fn validate(&self) -> Result<(), SettingsError> {
        if self.background_blur_px > 40 {
            return Err(SettingsError::InvalidData(
                "backgroundBlurPx must be between 0 and 40".into(),
            ));
        }
        if self.background_brightness_percent > 100 {
            return Err(SettingsError::InvalidData(
                "backgroundBrightnessPercent must be between 0 and 100".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackListColumnId {
    Title,
    /// Renderer label for the tag-derived `ARTIST` value.
    Artist,
    Album,
    Year,
    AudioFormat,
    Duration,
    PlayCount,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackListColumnPreference {
    pub id: TrackListColumnId,
    pub visible: bool,
}

/// Configurable information columns. Row index and play action are deliberately absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackListColumnSettings {
    /// Column order is represented by array order and must contain every information column once.
    pub columns: Vec<TrackListColumnPreference>,
}

impl Default for TrackListColumnSettings {
    fn default() -> Self {
        Self {
            columns: [
                TrackListColumnId::Title,
                TrackListColumnId::Artist,
                TrackListColumnId::Album,
                TrackListColumnId::Year,
                TrackListColumnId::AudioFormat,
                TrackListColumnId::Duration,
                TrackListColumnId::PlayCount,
            ]
            .into_iter()
            .map(|id| TrackListColumnPreference { id, visible: true })
            .collect(),
        }
    }
}

impl TrackListColumnSettings {
    fn validate(&self) -> Result<(), SettingsError> {
        use TrackListColumnId::{Album, Artist, AudioFormat, Duration, PlayCount, Title, Year};

        const REQUIRED: [TrackListColumnId; 7] =
            [Title, Artist, Album, Year, AudioFormat, Duration, PlayCount];
        let actual = self
            .columns
            .iter()
            .map(|column| column.id)
            .collect::<std::collections::HashSet<_>>();
        if self.columns.len() != REQUIRED.len()
            || actual.len() != REQUIRED.len()
            || REQUIRED.iter().any(|id| !actual.contains(id))
        {
            return Err(SettingsError::InvalidData(
                "track list columns must contain title, artist, album, year, audioFormat, duration, and playCount exactly once".into(),
            ));
        }
        Ok(())
    }
}

/// Accept persisted preferences that predate the playCount column.
fn repair_track_list_columns(settings: TrackListColumnSettings) -> TrackListColumnSettings {
    use TrackListColumnId::{Album, Artist, AudioFormat, Duration, PlayCount, Title, Year};

    if settings.validate().is_ok() {
        return settings;
    }

    const LEGACY: [TrackListColumnId; 6] = [Title, Artist, Album, Year, AudioFormat, Duration];
    let ids: Vec<_> = settings.columns.iter().map(|column| column.id).collect();
    let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
    if ids.len() == LEGACY.len()
        && unique.len() == LEGACY.len()
        && LEGACY.iter().all(|id| unique.contains(id))
        && !unique.contains(&PlayCount)
    {
        let mut columns = settings.columns;
        columns.push(TrackListColumnPreference {
            id: PlayCount,
            visible: true,
        });
        return TrackListColumnSettings { columns };
    }

    TrackListColumnSettings::default()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NowPlayingLayout {
    A,
    B,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "encoding", content = "value", rename_all = "camelCase")]
pub enum StoredPath {
    WindowsUtf16(Vec<u16>),
    Utf8(String),
    Uri(String),
}

impl StoredPath {
    pub fn from_path(path: &Path) -> Result<Self, SettingsError> {
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Ok(Self::WindowsUtf16(path.as_os_str().encode_wide().collect()))
        }
        #[cfg(not(windows))]
        {
            path.to_str()
                .map(|value| Self::Utf8(value.to_owned()))
                .ok_or_else(|| SettingsError::InvalidData("native path is not valid UTF-8".into()))
        }
    }

    pub fn from_locator(locator: &MediaLocator) -> Result<Self, SettingsError> {
        match locator {
            MediaLocator::FileSystem(path) => Self::from_path(path),
            MediaLocator::ContentUri(uri) => Ok(Self::Uri(uri.clone())),
        }
    }

    pub fn to_path_buf(&self) -> Result<PathBuf, SettingsError> {
        match self {
            Self::WindowsUtf16(units) => {
                #[cfg(windows)]
                {
                    use std::os::windows::ffi::OsStringExt;
                    Ok(PathBuf::from(OsString::from_wide(units)))
                }
                #[cfg(not(windows))]
                {
                    String::from_utf16(units).map(PathBuf::from).map_err(|_| {
                        SettingsError::InvalidData(
                            "Windows path cannot be represented on this platform".into(),
                        )
                    })
                }
            }
            Self::Utf8(value) => Ok(PathBuf::from(value)),
            Self::Uri(_) => Err(SettingsError::InvalidData(
                "content URI is not a filesystem path".into(),
            )),
        }
    }

    pub fn to_media_locator(&self) -> Result<MediaLocator, SettingsError> {
        match self {
            Self::Uri(uri) => Ok(MediaLocator::ContentUri(uri.clone())),
            _ => Ok(MediaLocator::FileSystem(self.to_path_buf()?)),
        }
    }

    pub fn display_lossy(&self) -> String {
        match self {
            Self::WindowsUtf16(units) => String::from_utf16_lossy(units),
            Self::Utf8(value) | Self::Uri(value) => value.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceEntryKind {
    Folder {
        media_kind: MediaSourceKind,
        path: StoredPath,
    },
    PlaylistFile {
        playlist_id: PlaylistId,
        path: StoredPath,
        /// Android SAF tree grant used to authorize playlist and relative media URIs.
        /// `None` preserves compatibility with Windows filesystem playlist sources.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tree_uri: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceEntry {
    pub id: SourceId,
    pub display_name: String,
    pub enabled: bool,
    pub kind: SourceEntryKind,
}

impl SourceEntry {
    pub fn from_library_root(root: &LibraryRoot) -> Result<Self, SettingsError> {
        Ok(Self {
            id: root.id,
            display_name: root.display_name.clone(),
            enabled: root.enabled,
            kind: SourceEntryKind::Folder {
                media_kind: root.kind,
                path: StoredPath::from_locator(&root.locator)?,
            },
        })
    }

    pub fn library_root(&self) -> Result<Option<LibraryRoot>, SettingsError> {
        match &self.kind {
            SourceEntryKind::Folder { media_kind, path } => Ok(Some(LibraryRoot {
                id: self.id,
                kind: *media_kind,
                display_name: self.display_name.clone(),
                locator: path.to_media_locator()?,
                enabled: self.enabled,
            })),
            SourceEntryKind::PlaylistFile { .. } => Ok(None),
        }
    }

    /// Build the SQLite synchronization projection for either a folder or a playlist file.
    pub fn database_projection(&self) -> Result<LibraryRoot, SettingsError> {
        let kind = match &self.kind {
            SourceEntryKind::Folder { media_kind, .. } => *media_kind,
            SourceEntryKind::PlaylistFile { .. } => MediaSourceKind::PlaylistFile,
        };
        Ok(LibraryRoot {
            id: self.id,
            kind,
            display_name: self.display_name.clone(),
            locator: self.path().to_media_locator()?,
            enabled: self.enabled,
        })
    }

    pub fn path(&self) -> &StoredPath {
        match &self.kind {
            SourceEntryKind::Folder { path, .. } | SourceEntryKind::PlaylistFile { path, .. } => {
                path
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowGeometry {
    pub const MIN_WIDTH: u32 = 360;
    pub const MIN_HEIGHT: u32 = 540;

    pub fn clamped_size(self) -> Self {
        Self {
            width: self.width.max(Self::MIN_WIDTH),
            height: self.height.max(Self::MIN_HEIGHT),
            ..self
        }
    }
}

/// Keep the window at least partially visible on one of the given monitor work areas.
/// `monitors` entries are `(x, y, width, height)` in physical pixels.
pub fn clamp_window_geometry_to_monitors(
    geometry: WindowGeometry,
    monitors: &[(i32, i32, u32, u32)],
) -> WindowGeometry {
    let geometry = geometry.clamped_size();
    const MIN_VISIBLE: i32 = 48;
    let width = i32::try_from(geometry.width).unwrap_or(i32::MAX);
    let height = i32::try_from(geometry.height).unwrap_or(i32::MAX);

    let visible = monitors.iter().any(|(mx, my, mw, mh)| {
        let mw = i32::try_from(*mw).unwrap_or(i32::MAX);
        let mh = i32::try_from(*mh).unwrap_or(i32::MAX);
        let left = geometry.x.max(*mx);
        let top = geometry.y.max(*my);
        let right = (geometry.x.saturating_add(width)).min(mx.saturating_add(mw));
        let bottom = (geometry.y.saturating_add(height)).min(my.saturating_add(mh));
        (right - left) >= MIN_VISIBLE && (bottom - top) >= MIN_VISIBLE
    });
    if visible || monitors.is_empty() {
        return geometry;
    }

    let (mx, my, mw, mh) = monitors[0];
    let mw = i32::try_from(mw).unwrap_or(width);
    let mh = i32::try_from(mh).unwrap_or(height);
    let x = mx + ((mw - width) / 2).max(0);
    let y = my + ((mh - height) / 2).max(0);
    WindowGeometry {
        x,
        y,
        width: geometry.width.min(u32::try_from(mw.max(1)).unwrap_or(geometry.width)),
        height: geometry.height.min(u32::try_from(mh.max(1)).unwrap_or(geometry.height)),
        maximized: geometry.maximized,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub theme: ThemeSettings,
    pub lyrics_preferences: LyricsPreferences,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub track_list_columns: TrackListColumnSettings,
    pub now_playing_layout: NowPlayingLayout,
    pub now_playing_appearance_preferences: NowPlayingAppearancePreferences,
    /// False means the source registry came from defaults after both JSON copies failed.
    /// Sync must remain paused until the user rebuilds and confirms the registry.
    pub source_registry_authoritative: bool,
    pub sources: Vec<SourceEntry>,
    /// Last main-window outer position / inner size / maximized flag (desktop).
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            theme: ThemeSettings::default(),
            lyrics_preferences: LyricsPreferences::default(),
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            track_list_columns: TrackListColumnSettings::default(),
            now_playing_layout: NowPlayingLayout::A,
            now_playing_appearance_preferences: NowPlayingAppearancePreferences::default(),
            source_registry_authoritative: true,
            sources: Vec::new(),
            window_geometry: None,
        }
    }
}

impl AppSettings {
    pub fn from_legacy(
        theme: player_db::ThemePreferences,
        roots: Vec<LibraryRoot>,
    ) -> Result<Self, SettingsError> {
        let mut settings = Self {
            theme: ThemeSettings {
                background_hex: theme.background_hex,
                accent_hex: theme.accent_hex,
            },
            ..Self::default()
        };
        settings.sources = roots
            .iter()
            .map(SourceEntry::from_library_root)
            .collect::<Result<_, _>>()?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedVersion(self.schema_version));
        }
        validate_color(&self.theme.background_hex, "backgroundHex")?;
        validate_color(&self.theme.accent_hex, "accentHex")?;
        self.lyrics_preferences.validate()?;
        self.now_playing_appearance_preferences.validate()?;
        self.track_list_columns.validate()?;
        let mut ids = std::collections::HashSet::new();
        for source in &self.sources {
            if !ids.insert(source.id) {
                return Err(SettingsError::InvalidData(format!(
                    "duplicate source id {}",
                    source.id
                )));
            }
            if source.display_name.trim().is_empty() {
                return Err(SettingsError::InvalidData(
                    "source display name cannot be empty".into(),
                ));
            }
            if source.path().display_lossy().is_empty() {
                return Err(SettingsError::InvalidData(
                    "source path cannot be empty".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(std::io::Error),
    Json(serde_json::Error),
    UnsupportedVersion(u32),
    InvalidData(String),
    Poisoned,
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "settings file I/O failed: {error}"),
            Self::Json(error) => write!(f, "settings JSON is invalid: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "settings schema version {version} is unsupported")
            }
            Self::InvalidData(message) => write!(f, "settings data is invalid: {message}"),
            Self::Poisoned => f.write_str("settings lock is poisoned"),
        }
    }
}

impl std::error::Error for SettingsError {}

impl From<std::io::Error> for SettingsError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for SettingsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub struct SettingsStore {
    path: PathBuf,
    settings: Mutex<AppSettings>,
    recovery_warning: Option<String>,
}

impl SettingsStore {
    pub fn needs_legacy_import(path: &Path) -> bool {
        !path.exists() && !path.with_extension("migration-v1-done").exists()
    }

    pub fn open(path: impl Into<PathBuf>, legacy: AppSettings) -> Result<Self, SettingsError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let migration_marker = path.with_extension("migration-v1-done");
        let backup_path = backup_path(&path);
        let (settings, mut recovery_warning, must_persist) = if path.exists() {
            match read_settings(&path) {
                Ok((settings, migrated)) => (settings, None, migrated),
                Err(SettingsError::UnsupportedVersion(version)) => {
                    return Err(SettingsError::UnsupportedVersion(version));
                }
                Err(primary_error) => {
                    archive_corrupt(&path)?;
                    match read_settings(&backup_path) {
                        Ok((settings, _)) => (
                            settings,
                            Some("設定檔無法讀取，已從上次有效備份還原。".to_owned()),
                            true,
                        ),
                        Err(_) => {
                            let settings = AppSettings {
                                source_registry_authoritative: false,
                                ..AppSettings::default()
                            };
                            (
                                settings,
                                Some(format!(
                                    "設定檔損毀且沒有可用備份，已載入預設值；來源同步暫停，請重新登記並確認來源。原檔已保留。詳情：{primary_error}"
                                )),
                                true,
                            )
                        }
                    }
                }
            }
        } else if backup_path.exists() {
            match read_settings(&backup_path) {
                Ok((settings, _)) => (
                    settings,
                    Some("設定檔遺失，已從上次有效備份還原。".to_owned()),
                    true,
                ),
                Err(SettingsError::UnsupportedVersion(version)) => {
                    return Err(SettingsError::UnsupportedVersion(version));
                }
                Err(backup_error) => {
                    let settings = AppSettings {
                        source_registry_authoritative: false,
                        ..AppSettings::default()
                    };
                    (
                        settings,
                        Some(format!(
                            "設定檔遺失且備份無效，已載入預設值；來源同步暫停，請重新登記並確認來源。未從 SQLite 還原可能已移除的來源。詳情：{backup_error}"
                        )),
                        true,
                    )
                }
            }
        } else if migration_marker.exists() {
            let settings = AppSettings {
                source_registry_authoritative: false,
                ..AppSettings::default()
            };
            (
                settings,
                Some(
                    "設定檔遺失且沒有可用備份，已載入預設值；來源同步暫停，請重新登記並確認來源。未從 SQLite 還原可能已移除的來源。"
                        .to_owned(),
                ),
                true,
            )
        } else {
            (legacy, None, true)
        };
        if !settings.source_registry_authoritative && recovery_warning.is_none() {
            recovery_warning = Some(
                "來源清單尚未確認，來源同步暫停以保留既有曲庫。請重新登記所有來源並確認來源清單。"
                    .to_owned(),
            );
        }
        settings.validate()?;
        if must_persist {
            persist_settings(&path, &settings)?;
        }
        write_migration_marker(&migration_marker)?;
        Ok(Self {
            path,
            settings: Mutex::new(settings),
            recovery_warning,
        })
    }

    pub fn snapshot(&self) -> Result<AppSettings, SettingsError> {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .map_err(|_| SettingsError::Poisoned)
    }

    pub fn recovery_warning(&self) -> Option<String> {
        self.recovery_warning.clone()
    }

    pub fn source_registry_authoritative(&self) -> Result<bool, SettingsError> {
        self.settings
            .lock()
            .map(|settings| settings.source_registry_authoritative)
            .map_err(|_| SettingsError::Poisoned)
    }

    pub fn confirm_source_registry(&self) -> Result<AppSettings, SettingsError> {
        self.update(|settings| {
            settings.source_registry_authoritative = true;
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn update<F>(&self, change: F) -> Result<AppSettings, SettingsError>
    where
        F: FnOnce(&mut AppSettings) -> Result<(), SettingsError>,
    {
        let mut current = self.settings.lock().map_err(|_| SettingsError::Poisoned)?;
        let mut next = current.clone();
        change(&mut next)?;
        next.schema_version = SETTINGS_SCHEMA_VERSION;
        next.validate()?;
        persist_settings(&self.path, &next)?;
        *current = next.clone();
        Ok(next)
    }

    pub fn set_source_enabled(
        &self,
        id: SourceId,
        enabled: bool,
    ) -> Result<AppSettings, SettingsError> {
        self.update(|settings| {
            let source = settings
                .sources
                .iter_mut()
                .find(|source| source.id == id)
                .ok_or_else(|| SettingsError::InvalidData("source not found".into()))?;
            source.enabled = enabled;
            Ok(())
        })
    }

    pub fn upsert_source(&self, source: SourceEntry) -> Result<AppSettings, SettingsError> {
        self.update(|settings| {
            if let Some(existing) = settings
                .sources
                .iter_mut()
                .find(|item| item.id == source.id)
            {
                *existing = source;
            } else {
                settings.sources.push(source);
            }
            Ok(())
        })
    }

    pub fn remove_source(&self, id: SourceId) -> Result<(AppSettings, SourceEntry), SettingsError> {
        let mut removed = None;
        let settings = self.update(|settings| {
            let index = settings
                .sources
                .iter()
                .position(|source| source.id == id)
                .ok_or_else(|| SettingsError::InvalidData("source not found".into()))?;
            removed = Some(settings.sources.remove(index));
            Ok(())
        })?;
        Ok((settings, removed.expect("source removed in update")))
    }

    #[cfg(test)]
    pub fn migration_marker(&self) -> PathBuf {
        self.path.with_extension("migration-v1-done")
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSettings {
    schema_version: Option<u32>,
    theme: Option<ThemeSettings>,
    lyrics_preferences: Option<LyricsPreferences>,
    shuffle: Option<bool>,
    repeat_mode: Option<RepeatMode>,
    track_list_columns: Option<TrackListColumnSettings>,
    now_playing_layout: Option<NowPlayingLayout>,
    now_playing_appearance_preferences: Option<NowPlayingAppearancePreferences>,
    source_registry_authoritative: Option<bool>,
    sources: Option<Vec<SourceEntry>>,
    window_geometry: Option<WindowGeometry>,
}

fn read_settings(path: &Path) -> Result<(AppSettings, bool), SettingsError> {
    let mut input = String::new();
    File::open(path)?.read_to_string(&mut input)?;
    let raw: RawSettings = serde_json::from_str(&input)?;
    let version = raw.schema_version.unwrap_or(0);
    if version > SETTINGS_SCHEMA_VERSION {
        return Err(SettingsError::UnsupportedVersion(version));
    }
    let settings = AppSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        theme: raw.theme.unwrap_or_default(),
        lyrics_preferences: raw.lyrics_preferences.unwrap_or_default(),
        shuffle: raw.shuffle.unwrap_or(false),
        repeat_mode: raw.repeat_mode.unwrap_or(RepeatMode::Off),
        track_list_columns: repair_track_list_columns(raw.track_list_columns.unwrap_or_default()),
        now_playing_layout: raw.now_playing_layout.unwrap_or(NowPlayingLayout::A),
        now_playing_appearance_preferences: raw
            .now_playing_appearance_preferences
            .unwrap_or_default(),
        source_registry_authoritative: raw.source_registry_authoritative.unwrap_or(true),
        sources: raw.sources.unwrap_or_default(),
        window_geometry: raw.window_geometry,
    };
    settings.validate()?;
    Ok((settings, version != SETTINGS_SCHEMA_VERSION))
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    let serialized = serde_json::to_vec_pretty(settings)?;
    let mut contents = serialized;
    contents.push(b'\n');
    if path.exists() {
        let backup_temp = temporary_path(&backup_path(path));
        write_synced(&backup_temp, &fs::read(path)?)?;
        fs::rename(&backup_temp, backup_path(path))?;
    }
    let temp = temporary_path(path);
    write_synced(&temp, &contents)?;
    fs::rename(&temp, path)?;
    sync_parent(path);
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), SettingsError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(path);
        return Err(SettingsError::Io(error));
    }
    Ok(())
}

fn temporary_path(target: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut name = target
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("settings"));
    name.push(format!(".tmp.{}.{}", std::process::id(), sequence));
    target.with_file_name(name)
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

fn archive_corrupt(path: &Path) -> Result<(), SettingsError> {
    let mut archive = path.as_os_str().to_os_string();
    archive.push(format!(
        ".corrupt.{}",
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::rename(path, PathBuf::from(archive))?;
    Ok(())
}

fn write_migration_marker(path: &Path) -> Result<(), SettingsError> {
    if path.exists() {
        return Ok(());
    }
    let temp = temporary_path(path);
    write_synced(&temp, format!("{SETTINGS_SCHEMA_VERSION}\n").as_bytes())?;
    fs::rename(temp, path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) {
    if let Some(parent) = path.parent() {
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) {}

fn validate_color(value: &str, field: &'static str) -> Result<(), SettingsError> {
    let bytes = value.as_bytes();
    if bytes.len() != 7 || bytes[0] != b'#' || !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
        return Err(SettingsError::InvalidData(format!(
            "{field} must be a six-digit hexadecimal color"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "moemusicplayer-settings-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        path
    }

    fn source(path: StoredPath) -> SourceEntry {
        SourceEntry {
            id: SourceId::new(),
            display_name: "source".into(),
            enabled: true,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id: PlaylistId::new(),
                path,
                tree_uri: None,
            },
        }
    }

    #[test]
    #[ignore = "isolated replay against an explicitly supplied settings copy"]
    fn preserved_settings_copy_loads_through_store_open() {
        let path = std::env::var_os("MOE_SETTINGS_STORE_FIXTURE")
            .expect("set MOE_SETTINGS_STORE_FIXTURE to an isolated settings copy");
        let path = PathBuf::from(path);
        let original_bytes = fs::read(&path).expect("read isolated settings copy");
        let (settings, migrated) = read_settings(&path).expect("parse preserved settings copy");
        assert!(
            !migrated,
            "the settings copy already uses the current schema"
        );
        assert!(settings.source_registry_authoritative);
        assert_eq!(settings.sources.len(), 2);
        assert!(matches!(
            &settings.sources[0].kind,
            SourceEntryKind::Folder { .. }
        ));
        assert!(matches!(
            &settings.sources[1].kind,
            SourceEntryKind::PlaylistFile { .. }
        ));

        let store = SettingsStore::open(&path, AppSettings::default())
            .expect("open isolated settings copy through the production store");
        assert_eq!(store.path(), path);
        assert_eq!(
            store.snapshot().expect("snapshot settings").sources.len(),
            2
        );
        assert!(store.source_registry_authoritative().unwrap());
        assert!(store.recovery_warning().is_none());
        assert_eq!(
            fs::read(&path).expect("settings remain unchanged"),
            original_bytes
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "read-only probe of logical and package-cache settings paths"]
    fn probe_live_settings_paths_without_opening_the_store() {
        fn final_handle_path(handle: std::os::windows::io::RawHandle) -> Option<String> {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetFinalPathNameByHandleW(
                    file: *mut std::ffi::c_void,
                    file_path: *mut u16,
                    file_path_size: u32,
                    flags: u32,
                ) -> u32;
            }

            let mut buffer = vec![0_u16; 32_768];
            let written = unsafe {
                GetFinalPathNameByHandleW(
                    handle,
                    buffer.as_mut_ptr(),
                    u32::try_from(buffer.len()).ok()?,
                    0,
                )
            } as usize;
            if written == 0 || written >= buffer.len() {
                return None;
            }
            Some(String::from_utf16_lossy(&buffer[..written]))
        }

        fn probe(label: &str, path: &Path) {
            let path_metadata = match fs::metadata(path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    eprintln!("settings_probe label={label} path={path:?} metadata_error={error}");
                    return;
                }
            };
            let canonical = fs::canonicalize(path).ok();
            let mut file = match File::open(path) {
                Ok(file) => file,
                Err(error) => {
                    eprintln!("settings_probe label={label} path={path:?} open_error={error}");
                    return;
                }
            };
            let handle_path = final_handle_path(file.as_raw_handle());
            let handle_metadata = file.metadata().ok();
            let mut bytes = Vec::new();
            if let Err(error) = file.read_to_end(&mut bytes) {
                eprintln!("settings_probe label={label} path={path:?} read_error={error}");
                return;
            }
            let settings_result = read_settings(path).map(|(settings, migrated)| {
                (
                    settings.sources.len(),
                    settings.source_registry_authoritative,
                    migrated,
                )
            });
            eprintln!(
                "settings_probe label={label} requested={path:?} canonical={canonical:?} handle_final={handle_path:?} path_len={} handle_len={:?} read_len={} loader={settings_result:?}",
                path_metadata.len(),
                handle_metadata.map(|metadata| metadata.len()),
                bytes.len(),
            );
        }

        use std::os::windows::io::AsRawHandle;

        let logical = PathBuf::from(
            std::env::var_os("MOE_SETTINGS_LOGICAL_PATH")
                .expect("set MOE_SETTINGS_LOGICAL_PATH to the Roaming settings path"),
        );
        let extended = PathBuf::from(
            std::env::var_os("MOE_SETTINGS_EXTENDED_PATH")
                .expect("set MOE_SETTINGS_EXTENDED_PATH to the extended Roaming path"),
        );
        let local_cache = PathBuf::from(
            std::env::var_os("MOE_SETTINGS_LOCALCACHE_PATH")
                .expect("set MOE_SETTINGS_LOCALCACHE_PATH to the LocalCache settings path"),
        );
        probe("logical-roaming", &logical);
        probe("extended-roaming", &extended);
        probe("direct-local-cache", &local_cache);
    }

    #[test]
    fn migration_from_legacy_sqlite_values_writes_a_versioned_json_file() {
        let directory = test_directory("legacy");
        let path = directory.join("settings.json");
        let root = LibraryRoot {
            id: SourceId::new(),
            kind: MediaSourceKind::WindowsFilesystem,
            display_name: "Music".into(),
            locator: MediaLocator::FileSystem(PathBuf::from(r"C:\音樂")),
            enabled: true,
        };
        let legacy = AppSettings::from_legacy(
            player_db::ThemePreferences {
                background_hex: "#102030".into(),
                accent_hex: "#ABCDEF".into(),
            },
            vec![root.clone()],
        )
        .expect("convert legacy data");
        let store = SettingsStore::open(&path, legacy).expect("migrate settings");
        let settings = store.snapshot().expect("read migrated settings");
        assert_eq!(settings.theme.background_hex, "#102030");
        assert_eq!(settings.sources[0].id, root.id);
        assert!(store.migration_marker().exists());
        let reopened = SettingsStore::open(&path, AppSettings::default())
            .expect("open migrated JSON without importing legacy values again");
        assert_eq!(reopened.snapshot().unwrap(), settings);
    }

    #[test]
    fn actual_sqlite_theme_and_roots_migrate_once_to_json() {
        let directory = test_directory("sqlite-migration");
        let database = player_db::Database::open(directory.join("music.sqlite3"))
            .expect("open isolated SQLite database");
        database
            .set_theme_preferences(&player_db::ThemePreferences {
                background_hex: "#314159".into(),
                accent_hex: "#26A69A".into(),
            })
            .expect("set legacy theme");
        let root = database
            .add_library_root(
                MediaSourceKind::WindowsFilesystem,
                "日本語 音樂",
                MediaLocator::FileSystem(PathBuf::from(r"C:\音樂🎧")),
            )
            .expect("add legacy folder source");
        let legacy = AppSettings::from_legacy(
            database.get_theme_preferences().expect("read legacy theme"),
            database.library_roots().expect("read legacy roots"),
        )
        .expect("convert SQLite settings");
        let settings_path = directory.join("settings.json");
        let store = SettingsStore::open(&settings_path, legacy).expect("persist migration");
        let migrated = store.snapshot().unwrap();
        assert_eq!(migrated.theme.background_hex, "#314159");
        assert_eq!(migrated.theme.accent_hex, "#26A69A");
        assert_eq!(migrated.sources.len(), 1);
        assert_eq!(migrated.sources[0].id, root.id);
        assert_eq!(migrated.sources[0].path().display_lossy(), r"C:\音樂🎧");

        let later_legacy = AppSettings::from_legacy(
            player_db::ThemePreferences {
                background_hex: "#000000".into(),
                accent_hex: "#55D9FF".into(),
            },
            vec![root],
        )
        .expect("legacy still contains source");
        let reopened = SettingsStore::open(&settings_path, later_legacy).expect("reopen JSON");
        assert_eq!(reopened.snapshot().unwrap(), migrated);
    }

    #[test]
    fn shuffle_and_repeat_modes_survive_restart() {
        let directory = test_directory("modes-restart");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        store
            .update(|settings| {
                settings.shuffle = true;
                settings.repeat_mode = RepeatMode::All;
                Ok(())
            })
            .expect("persist modes");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        let settings = reopened.snapshot().unwrap();
        assert!(settings.shuffle);
        assert_eq!(settings.repeat_mode, RepeatMode::All);
    }

    #[test]
    fn source_registry_upsert_enable_and_remove_are_persisted() {
        let directory = test_directory("source-crud");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let source = source(StoredPath::Utf8("D:/Playlists/憨色.m3u8".into()));
        let id = source.id;
        store
            .upsert_source(source.clone())
            .expect("register playlist file");
        let mut updated = source;
        updated.display_name = "renamed".into();
        store
            .upsert_source(updated)
            .expect("update existing source");
        store.set_source_enabled(id, false).expect("disable source");
        assert_eq!(store.snapshot().unwrap().sources.len(), 1);
        assert!(!store.snapshot().unwrap().sources[0].enabled);
        assert_eq!(store.snapshot().unwrap().sources[0].display_name, "renamed");

        let (after_remove, removed) = store.remove_source(id).expect("remove source");
        assert_eq!(removed.id, id);
        assert!(after_remove.sources.is_empty());
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.snapshot().unwrap().sources.is_empty());
    }

    #[test]
    fn old_unversioned_json_is_upgraded_without_losing_theme() {
        let directory = test_directory("old-json");
        let path = directory.join("settings.json");
        fs::write(
            &path,
            r##"{"theme":{"backgroundHex":"#123456","accentHex":"#ABCDEF"}}"##,
        )
        .expect("write legacy JSON");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("upgrade JSON");
        let settings = store.snapshot().unwrap();
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.theme.background_hex, "#123456");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn updates_replace_atomically_and_corruption_restores_the_last_valid_backup() {
        let directory = test_directory("atomic");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        store
            .update(|settings| {
                settings.theme.background_hex = "#102030".into();
                settings.shuffle = true;
                settings.repeat_mode = RepeatMode::All;
                settings
                    .sources
                    .push(source(StoredPath::Utf8("playlist.m3u8".into())));
                Ok(())
            })
            .expect("persist updated settings");
        store
            .update(|settings| {
                settings.theme.background_hex = "#405060".into();
                Ok(())
            })
            .expect("replace settings file");
        let before = store.snapshot().unwrap();
        store
            .update(|settings| {
                settings.shuffle = false;
                Ok(())
            })
            .expect("replace the existing backup atomically");
        fs::write(&path, b"{ broken json").expect("corrupt current file");
        let recovered =
            SettingsStore::open(&path, AppSettings::default()).expect("recover from valid backup");
        assert_eq!(recovered.snapshot().unwrap(), before);
        assert!(recovered.recovery_warning().is_some());
        assert!(fs::read_dir(&directory).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".corrupt.")));
        assert!(read_settings(&path).is_ok());
    }

    #[test]
    fn corruption_without_backup_does_not_revive_legacy_sources() {
        let directory = test_directory("corrupt-no-backup");
        let path = directory.join("settings.json");
        fs::write(&path, b"bad").expect("write corrupt file");
        let mut legacy = AppSettings::default();
        legacy
            .sources
            .push(source(StoredPath::Utf8("removed-folder".into())));
        let store = SettingsStore::open(&path, legacy).expect("recover with defaults");
        assert!(store.snapshot().unwrap().sources.is_empty());
        assert!(!store.snapshot().unwrap().source_registry_authoritative);
        assert!(store.recovery_warning().unwrap().contains("沒有可用備份"));
    }

    #[test]
    fn missing_primary_restores_valid_backup_before_considering_defaults() {
        let directory = test_directory("missing-primary-with-backup");
        let path = directory.join("settings.json");
        let expected = AppSettings {
            sources: vec![source(StoredPath::Utf8("D:/Music".into()))],
            shuffle: true,
            ..AppSettings::default()
        };
        fs::write(
            backup_path(&path),
            serde_json::to_vec_pretty(&expected).expect("serialize backup"),
        )
        .expect("write valid backup");
        let store = SettingsStore::open(&path, AppSettings::default())
            .expect("restore missing primary from backup");

        assert_eq!(store.snapshot().unwrap(), expected);
        assert!(store.source_registry_authoritative().unwrap());
        assert!(store.recovery_warning().unwrap().contains("備份"));
        assert!(read_settings(&path).is_ok());
    }

    #[test]
    fn missing_primary_and_invalid_backup_pause_registry_until_user_confirmation() {
        let directory = test_directory("missing-primary-no-backup");
        let path = directory.join("settings.json");
        fs::write(backup_path(&path), b"{invalid backup").expect("write invalid backup");
        fs::write(path.with_extension("migration-v1-done"), b"1\n")
            .expect("write completed migration marker");

        let store = SettingsStore::open(&path, AppSettings::default())
            .expect("recover missing primary with defaults");

        assert!(!store.source_registry_authoritative().unwrap());
        assert!(store.recovery_warning().unwrap().contains("來源同步暫停"));
        let confirmed = store
            .confirm_source_registry()
            .expect("persist explicit registry confirmation");
        assert!(confirmed.source_registry_authoritative);
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.source_registry_authoritative().unwrap());
    }

    #[test]
    fn future_schema_is_rejected_without_changing_the_file() {
        let directory = test_directory("future");
        let path = directory.join("settings.json");
        let contents = br#"{"schemaVersion":99}"#;
        fs::write(&path, contents).expect("write future settings");
        assert!(matches!(
            SettingsStore::open(&path, AppSettings::default()),
            Err(SettingsError::UnsupportedVersion(99))
        ));
        assert_eq!(fs::read(&path).unwrap(), contents);
    }

    #[test]
    fn invalid_update_does_not_replace_memory_or_json() {
        let directory = test_directory("invalid-update");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let original = fs::read(&path).expect("read original file");
        let result = store.update(|settings| {
            settings.theme.accent_hex = "not-a-color".into();
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(store.snapshot().unwrap(), AppSettings::default());
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn source_registry_round_trips_playlist_identity_and_windows_utf16_units() {
        let directory = test_directory("utf16");
        let path = directory.join("settings.json");
        let mut settings = AppSettings::default();
        let expected = vec![
            0x0043, 0x003a, 0x005c, 0xd800, 0x002e, 0x006d, 0x0033, 0x0075,
        ];
        settings
            .sources
            .push(source(StoredPath::WindowsUtf16(expected.clone())));
        let store = SettingsStore::open(&path, settings.clone()).expect("write UTF-16 path");
        let reopened =
            SettingsStore::open(&path, AppSettings::default()).expect("read UTF-16 path");
        assert_eq!(reopened.snapshot().unwrap(), settings);
        assert!(matches!(
            &reopened.snapshot().unwrap().sources[0].kind,
            SourceEntryKind::PlaylistFile { path: StoredPath::WindowsUtf16(units), .. }
                if units == &expected
        ));
        assert!(store.path().exists());
    }

    #[test]
    fn settings_json_uses_stable_camel_case_ipc_and_source_registry_names() {
        let playlist_id = PlaylistId::new();
        let mut settings = AppSettings {
            shuffle: true,
            repeat_mode: RepeatMode::One,
            ..AppSettings::default()
        };
        settings.sources.push(SourceEntry {
            id: SourceId::new(),
            display_name: "playlist".into(),
            enabled: true,
            kind: SourceEntryKind::PlaylistFile {
                playlist_id,
                path: StoredPath::Utf8("C:/music.m3u8".into()),
                tree_uri: None,
            },
        });
        let json = serde_json::to_value(&settings).expect("serialize settings");
        assert_eq!(json["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(json["sourceRegistryAuthoritative"], true);
        assert_eq!(json["repeatMode"], "one");
        assert_eq!(json["sources"][0]["kind"]["type"], "playlistFile");
        assert_eq!(
            json["sources"][0]["kind"]["playlistId"],
            playlist_id.to_string()
        );
        assert_eq!(json["sources"][0]["kind"]["path"]["encoding"], "utf8");
        let decoded: AppSettings = serde_json::from_value(json).expect("deserialize settings");
        assert_eq!(decoded, settings);
    }

    #[test]
    fn new_display_preferences_have_conventional_defaults_and_wire_names() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.now_playing_layout, NowPlayingLayout::A);
        assert_eq!(
            defaults
                .track_list_columns
                .columns
                .iter()
                .map(|column| (column.id, column.visible))
                .collect::<Vec<_>>(),
            [
                TrackListColumnId::Title,
                TrackListColumnId::Artist,
                TrackListColumnId::Album,
                TrackListColumnId::Year,
                TrackListColumnId::AudioFormat,
                TrackListColumnId::Duration,
                TrackListColumnId::PlayCount,
            ]
            .map(|id| (id, true))
        );

        let value = serde_json::to_value(defaults).expect("serialize defaults");
        assert_eq!(value["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(value["nowPlayingLayout"], "a");
        assert_eq!(serde_json::to_value(NowPlayingLayout::B).unwrap(), "b");
        assert_eq!(value["trackListColumns"]["columns"][4]["id"], "audioFormat");
        assert_eq!(value["trackListColumns"]["columns"][4]["visible"], true);
        assert!(serde_json::from_value::<AppSettings>(value).is_ok());
    }

    #[test]
    fn lyrics_preferences_defaults_and_json_use_the_ipc_contract_names() {
        let preferences = LyricsPreferences::default();
        assert_eq!(
            preferences,
            LyricsPreferences {
                show_translation: false,
                show_romanization: false,
                inactive_opacity_percent: 70,
                primary_font_size_px: 14,
                auxiliary_font_size_px: 10,
                line_gap_px: 24,
                text_effect: LyricsTextEffect::Shadow,
            }
        );

        let value = serde_json::to_value(preferences).expect("serialize lyric preferences");
        assert_eq!(value["showTranslation"], false);
        assert_eq!(value["showRomanization"], false);
        assert_eq!(value["inactiveOpacityPercent"], 70);
        assert_eq!(value["primaryFontSizePx"], 14);
        assert_eq!(value["auxiliaryFontSizePx"], 10);
        assert_eq!(value["lineGapPx"], 24);
        assert_eq!(value["textEffect"], "shadow");
        assert!(value.get("show_translation").is_none());
        assert_eq!(
            serde_json::from_value::<LyricsPreferences>(value).unwrap(),
            preferences
        );
    }

    #[test]
    fn lyrics_preference_numeric_ranges_are_inclusive_and_reject_out_of_range_values() {
        let mut minimum = LyricsPreferences {
            inactive_opacity_percent: 10,
            primary_font_size_px: 12,
            auxiliary_font_size_px: 9,
            ..LyricsPreferences::default()
        };
        minimum.validate().expect("accept lower bounds");

        let maximum = LyricsPreferences {
            inactive_opacity_percent: 100,
            primary_font_size_px: 36,
            auxiliary_font_size_px: 24,
            ..LyricsPreferences::default()
        };
        maximum.validate().expect("accept upper bounds");

        let mut line_gap = LyricsPreferences {
            line_gap_px: 0,
            ..LyricsPreferences::default()
        };
        line_gap.validate().expect("accept zero line gap");
        line_gap.line_gap_px = 64;
        line_gap.validate().expect("accept maximum line gap");
        line_gap.line_gap_px = 65;
        assert!(line_gap.validate().is_err());

        minimum.inactive_opacity_percent = 9;
        assert!(minimum.validate().is_err());
        minimum.inactive_opacity_percent = 101;
        assert!(minimum.validate().is_err());

        minimum.inactive_opacity_percent = 70;
        minimum.primary_font_size_px = 11;
        assert!(minimum.validate().is_err());
        minimum.primary_font_size_px = 37;
        assert!(minimum.validate().is_err());

        minimum.primary_font_size_px = 14;
        minimum.auxiliary_font_size_px = 8;
        assert!(minimum.validate().is_err());
        minimum.auxiliary_font_size_px = 25;
        assert!(minimum.validate().is_err());
    }

    #[test]
    fn invalid_lyrics_preference_update_does_not_change_memory_or_json() {
        let directory = test_directory("invalid-lyrics-preferences");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let original = fs::read(&path).expect("read original settings");

        let result = store.update(|settings| {
            settings.lyrics_preferences = LyricsPreferences {
                auxiliary_font_size_px: 25,
                ..LyricsPreferences::default()
            };
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(
            store.snapshot().unwrap().lyrics_preferences,
            LyricsPreferences::default()
        );
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn schema_four_without_line_gap_reads_default_and_persists_it_on_next_save() {
        let directory = test_directory("lyrics-line-gap-migration");
        let path = directory.join("settings.json");
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        json["schemaVersion"] = serde_json::json!(4);
        json["lyricsPreferences"]
            .as_object_mut()
            .unwrap()
            .remove("lineGapPx");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).expect("write old schema four JSON");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("read old settings");
        assert_eq!(store.snapshot().unwrap().lyrics_preferences.line_gap_px, 24);

        store
            .update(|settings| {
                settings.lyrics_preferences.line_gap_px = 36;
                Ok(())
            })
            .expect("persist line gap");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(persisted["lyricsPreferences"]["lineGapPx"], 36);
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert_eq!(
            reopened.snapshot().unwrap().lyrics_preferences.line_gap_px,
            36
        );
    }

    #[test]
    fn invalid_line_gap_does_not_change_memory_or_json() {
        let directory = test_directory("invalid-lyrics-line-gap");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let original = fs::read(&path).expect("read original settings");

        let result = store.update(|settings| {
            settings.lyrics_preferences.line_gap_px = 65;
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(store.snapshot().unwrap().lyrics_preferences.line_gap_px, 24);
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn schema_two_json_migrates_additively_and_preserves_existing_settings() {
        let directory = test_directory("schema-two-upgrade");
        let path = directory.join("settings.json");
        let source = source(StoredPath::Utf8("D:/Music".into()));
        let mut previous = AppSettings {
            theme: ThemeSettings {
                background_hex: "#123456".into(),
                accent_hex: "#ABCDEF".into(),
            },
            sources: vec![source.clone()],
            shuffle: true,
            repeat_mode: RepeatMode::All,
            source_registry_authoritative: false,
            ..AppSettings::default()
        };
        previous.schema_version = 2;
        let mut value = serde_json::to_value(previous).expect("serialize schema-two settings");
        value.as_object_mut().unwrap().remove("lyricsPreferences");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap())
            .expect("write schema-two settings");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("migrate schema two");
        let migrated = store.snapshot().unwrap();
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.lyrics_preferences, LyricsPreferences::default());
        assert_eq!(migrated.theme.background_hex, "#123456");
        assert_eq!(migrated.theme.accent_hex, "#ABCDEF");
        assert_eq!(migrated.sources, vec![source]);
        assert!(migrated.shuffle);
        assert_eq!(migrated.repeat_mode, RepeatMode::All);
        assert!(!migrated.source_registry_authoritative);

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(persisted["lyricsPreferences"]["showTranslation"], false);
        assert_eq!(persisted["lyricsPreferences"]["showRomanization"], false);
        assert_eq!(persisted["lyricsPreferences"]["inactiveOpacityPercent"], 70);
        assert_eq!(persisted["lyricsPreferences"]["primaryFontSizePx"], 14);
        assert_eq!(persisted["lyricsPreferences"]["auxiliaryFontSizePx"], 10);
        assert_eq!(persisted["sourceRegistryAuthoritative"], false);
    }

    #[test]
    fn schema_one_json_migrates_with_new_preferences_defaulted_and_registry_preserved() {
        let directory = test_directory("schema-one-upgrade");
        let path = directory.join("settings.json");
        let source = source(StoredPath::Utf8("D:/Music".into()));
        let mut previous = AppSettings {
            sources: vec![source.clone()],
            shuffle: true,
            repeat_mode: RepeatMode::All,
            source_registry_authoritative: false,
            ..AppSettings::default()
        };
        previous.schema_version = 1;
        let mut value = serde_json::to_value(previous).expect("serialize schema-one settings");
        value.as_object_mut().unwrap().remove("trackListColumns");
        value.as_object_mut().unwrap().remove("nowPlayingLayout");
        value.as_object_mut().unwrap().remove("lyricsPreferences");
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap())
            .expect("write schema-one settings");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("migrate schema one");
        let migrated = store.snapshot().unwrap();
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(
            migrated.track_list_columns,
            TrackListColumnSettings::default()
        );
        assert_eq!(migrated.now_playing_layout, NowPlayingLayout::A);
        assert_eq!(migrated.sources, vec![source]);
        assert!(migrated.shuffle);
        assert_eq!(migrated.repeat_mode, RepeatMode::All);
        assert!(!migrated.source_registry_authoritative);
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(persisted["sourceRegistryAuthoritative"], false);
    }

    #[test]
    fn legacy_six_column_preferences_gain_play_count_on_load() {
        let directory = test_directory("legacy-play-count-columns");
        let path = directory.join("settings.json");
        let legacy = serde_json::json!({
            "schemaVersion": SETTINGS_SCHEMA_VERSION,
            "theme": {
                "backgroundHex": "#000000",
                "accentHex": "#55D9FF"
            },
            "lyricsPreferences": {
                "showTranslation": false,
                "showRomanization": false,
                "inactiveOpacityPercent": 70,
                "primaryFontSizePx": 14,
                "auxiliaryFontSizePx": 10,
                "lineGapPx": 24
            },
            "shuffle": false,
            "repeatMode": "off",
            "trackListColumns": {
                "columns": [
                    { "id": "title", "visible": true },
                    { "id": "artist", "visible": false },
                    { "id": "album", "visible": true },
                    { "id": "year", "visible": true },
                    { "id": "audioFormat", "visible": true },
                    { "id": "duration", "visible": true }
                ]
            },
            "nowPlayingLayout": "a",
            "nowPlayingAppearancePreferences": {
                "backgroundBlurPx": 20,
                "backgroundBrightnessPercent": 40,
                "coverCornerStyle": "rounded"
            },
            "sourceRegistryAuthoritative": true,
            "sources": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).expect("write legacy");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("open repaired");
        let columns = store.snapshot().unwrap().track_list_columns.columns;
        assert_eq!(columns.len(), 7);
        assert_eq!(columns[1].id, TrackListColumnId::Artist);
        assert!(!columns[1].visible);
        assert_eq!(columns[6].id, TrackListColumnId::PlayCount);
        assert!(columns[6].visible);
        TrackListColumnSettings {
            columns: columns.clone(),
        }
        .validate()
        .expect("repaired columns validate");
    }

    #[test]
    fn column_order_can_change_but_missing_or_duplicate_ids_are_rejected() {
        let mut reordered = TrackListColumnSettings::default();
        reordered.columns.swap(0, 3);
        reordered.columns[0].visible = false;
        reordered.validate().expect("valid permutation");
        let encoded = serde_json::to_string(&reordered).unwrap();
        let decoded: TrackListColumnSettings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, reordered);

        let mut duplicate = reordered.clone();
        duplicate.columns[0].id = duplicate.columns[1].id;
        assert!(duplicate.validate().is_err());

        let mut missing = reordered;
        missing.columns.pop();
        assert!(missing.validate().is_err());

        let unknown = serde_json::json!({
            "columns": [{ "id": "rowIndex", "visible": true }]
        });
        assert!(serde_json::from_value::<TrackListColumnSettings>(unknown).is_err());
    }

    #[test]
    fn display_preferences_persist_and_reopen_without_promoting_degraded_registry() {
        let directory = test_directory("display-preferences-restart");
        let path = directory.join("settings.json");
        let source = source(StoredPath::Utf8("D:/Music".into()));
        let initial = AppSettings {
            source_registry_authoritative: false,
            sources: vec![source.clone()],
            ..AppSettings::default()
        };
        let store = SettingsStore::open(&path, initial).expect("create degraded settings");
        let lyrics_preferences = LyricsPreferences {
            show_translation: true,
            show_romanization: true,
            inactive_opacity_percent: 43,
            primary_font_size_px: 26,
            auxiliary_font_size_px: 17,
            line_gap_px: 42,
            text_effect: LyricsTextEffect::Shadow,
        };
        let mut columns = TrackListColumnSettings::default();
        columns.columns.swap(0, 2);
        columns.columns[1].visible = false;
        store
            .update(|settings| {
                settings.track_list_columns = columns.clone();
                settings.now_playing_layout = NowPlayingLayout::B;
                settings.lyrics_preferences = lyrics_preferences;
                Ok(())
            })
            .expect("persist display preferences");

        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        let settings = reopened.snapshot().unwrap();
        assert_eq!(settings.track_list_columns, columns);
        assert_eq!(settings.now_playing_layout, NowPlayingLayout::B);
        assert_eq!(settings.lyrics_preferences, lyrics_preferences);
        assert_eq!(settings.sources, vec![source]);
        assert!(!settings.source_registry_authoritative);
        assert!(!reopened.source_registry_authoritative().unwrap());
    }

    #[test]
    fn schema_three_migrates_appearance_defaults_without_losing_other_settings() {
        let directory = test_directory("appearance-migration");
        let path = directory.join("settings.json");
        let mut previous = AppSettings {
            schema_version: 3,
            sources: vec![source(StoredPath::Utf8("D:/Music".into()))],
            shuffle: true,
            repeat_mode: RepeatMode::All,
            now_playing_layout: NowPlayingLayout::B,
            ..AppSettings::default()
        };
        previous.theme.background_hex = "#123456".into();
        previous.lyrics_preferences.show_translation = true;
        let mut json = serde_json::to_value(previous.clone()).unwrap();
        json.as_object_mut()
            .unwrap()
            .remove("nowPlayingAppearancePreferences");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).expect("write schema three settings");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("migrate settings");
        let migrated = store.snapshot().unwrap();
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(
            migrated.now_playing_appearance_preferences,
            NowPlayingAppearancePreferences::default()
        );
        assert_eq!(migrated.theme, previous.theme);
        assert_eq!(migrated.lyrics_preferences, previous.lyrics_preferences);
        assert_eq!(migrated.sources, previous.sources);
        assert!(migrated.shuffle);
        assert_eq!(migrated.repeat_mode, RepeatMode::All);
        assert_eq!(migrated.now_playing_layout, NowPlayingLayout::B);

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["backgroundBlurPx"],
            20
        );
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["backgroundBrightnessPercent"],
            40
        );
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["coverCornerStyle"],
            "rounded"
        );
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["timelineStyle"],
            "line"
        );
        assert!(
            persisted["nowPlayingAppearancePreferences"]["surfaceTransparencyPercent"].is_null()
        );
    }

    #[test]
    fn appearance_preferences_roundtrip_reopen_and_enforce_ranges() {
        let directory = test_directory("appearance-restart");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let preferences = NowPlayingAppearancePreferences {
            background_blur_px: 0,
            background_brightness_percent: 100,
            cover_corner_style: CoverCornerStyle::Square,
            timeline_style: TimelineStyle::Line,
        };
        let saved = store
            .update(|settings| {
                settings.now_playing_appearance_preferences = preferences;
                Ok(())
            })
            .expect("save boundary values");
        assert_eq!(saved.now_playing_appearance_preferences, preferences);

        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert_eq!(
            reopened
                .snapshot()
                .unwrap()
                .now_playing_appearance_preferences,
            preferences
        );

        for invalid in [
            NowPlayingAppearancePreferences {
                background_blur_px: 41,
                ..preferences
            },
            NowPlayingAppearancePreferences {
                background_brightness_percent: 101,
                ..preferences
            },
        ] {
            assert!(reopened
                .update(|settings| {
                    settings.now_playing_appearance_preferences = invalid;
                    Ok(())
                })
                .is_err());
            assert_eq!(
                reopened
                    .snapshot()
                    .unwrap()
                    .now_playing_appearance_preferences,
                preferences
            );
        }
    }

    #[test]
    fn appearance_preferences_dto_uses_camel_case_json_keys() {
        let preferences = NowPlayingAppearancePreferences {
            background_blur_px: 12,
            background_brightness_percent: 67,
            cover_corner_style: CoverCornerStyle::Square,
            timeline_style: TimelineStyle::Line,
        };
        let value = serde_json::to_value(preferences).unwrap();
        assert_eq!(value["backgroundBlurPx"], 12);
        assert_eq!(value["backgroundBrightnessPercent"], 67);
        assert_eq!(value["coverCornerStyle"], "square");
        assert_eq!(value["timelineStyle"], "line");
        assert_eq!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(value).unwrap(),
            preferences
        );
        assert!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": -1,
                "backgroundBrightnessPercent": 50
            }))
            .is_err()
        );
    }

    #[test]
    fn missing_cover_corner_style_defaults_to_rounded_and_rejects_unknown_values() {
        let directory = test_directory("appearance-cover-corner");
        let path = directory.join("settings.json");
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        json["schemaVersion"] = serde_json::json!(5);
        json["nowPlayingAppearancePreferences"] = serde_json::json!({
            "backgroundBlurPx": 15,
            "backgroundBrightnessPercent": 55
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap())
            .expect("write settings without cover corner");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("open settings");
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot
                .now_playing_appearance_preferences
                .cover_corner_style,
            CoverCornerStyle::Rounded
        );

        let saved = store
            .update(|settings| {
                settings
                    .now_playing_appearance_preferences
                    .cover_corner_style = CoverCornerStyle::Square;
                Ok(())
            })
            .expect("save square corners");
        assert_eq!(
            saved.now_playing_appearance_preferences.cover_corner_style,
            CoverCornerStyle::Square
        );

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["coverCornerStyle"],
            "square"
        );

        assert!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": 12,
                "backgroundBrightnessPercent": 50,
                "coverCornerStyle": "circle"
            }))
            .is_err()
        );
    }

    #[test]
    fn missing_timeline_style_defaults_to_line_and_rejects_unknown_values() {
        let directory = test_directory("appearance-timeline-style");
        let path = directory.join("settings.json");
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        json["schemaVersion"] = serde_json::json!(5);
        json["nowPlayingAppearancePreferences"] = serde_json::json!({
            "backgroundBlurPx": 15,
            "backgroundBrightnessPercent": 55,
            "coverCornerStyle": "square"
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap())
            .expect("write settings without timeline");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("open settings");
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot.now_playing_appearance_preferences.timeline_style,
            TimelineStyle::Line
        );

        let saved = store
            .update(|settings| {
                settings.now_playing_appearance_preferences.timeline_style = TimelineStyle::Edge;
                Ok(())
            })
            .expect("save edge timeline");
        assert_eq!(
            saved.now_playing_appearance_preferences.timeline_style,
            TimelineStyle::Edge
        );

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["timelineStyle"],
            "edge"
        );

        assert_eq!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": 12,
                "backgroundBrightnessPercent": 50,
                "timelineStyle": "minimal"
            }))
            .unwrap()
            .timeline_style,
            TimelineStyle::Line
        );

        assert!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": 12,
                "backgroundBrightnessPercent": 50,
                "timelineStyle": "chunky"
            }))
            .is_err()
        );
    }

    #[test]
    fn schema_four_legacy_surface_values_migrate_to_transparent_surfaces_and_default_brightness() {
        for legacy_surface in [0, 100] {
            let directory = test_directory(&format!("appearance-v4-surface-{legacy_surface}"));
            let path = directory.join("settings.json");
            let mut json = serde_json::to_value(AppSettings::default()).unwrap();
            json["schemaVersion"] = serde_json::json!(4);
            json["nowPlayingAppearancePreferences"] = serde_json::json!({
                "backgroundBlurPx": 27,
                "surfaceTransparencyPercent": legacy_surface
            });
            json["sources"] = serde_json::json!([source(StoredPath::Utf8("D:/Music".into()))]);
            json["shuffle"] = serde_json::json!(true);
            json["theme"]["backgroundHex"] = serde_json::json!("#123456");
            json["lyricsPreferences"]["showTranslation"] = serde_json::json!(true);
            json["repeatMode"] = serde_json::json!("all");
            json["nowPlayingLayout"] = serde_json::json!("b");
            assert_eq!(
                serde_json::from_value::<NowPlayingAppearancePreferences>(
                    json["nowPlayingAppearancePreferences"].clone()
                )
                .unwrap()
                .background_blur_px,
                27
            );
            fs::write(&path, serde_json::to_vec(&json).unwrap()).expect("write legacy settings");

            let store = SettingsStore::open(&path, AppSettings::default())
                .expect("migrate legacy settings");
            let migrated = store.snapshot().unwrap();
            assert_eq!(
                migrated
                    .now_playing_appearance_preferences
                    .background_blur_px,
                27
            );
            assert_eq!(
                migrated
                    .now_playing_appearance_preferences
                    .background_brightness_percent,
                40
            );
            assert!(migrated.shuffle);
            assert_eq!(migrated.sources.len(), 1);
            assert_eq!(migrated.theme.background_hex, "#123456");
            assert!(migrated.lyrics_preferences.show_translation);
            assert_eq!(migrated.repeat_mode, RepeatMode::All);
            assert_eq!(migrated.now_playing_layout, NowPlayingLayout::B);

            let persisted: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(
                persisted["nowPlayingAppearancePreferences"]["backgroundBrightnessPercent"],
                40
            );
            assert!(
                persisted["nowPlayingAppearancePreferences"]["surfaceTransparencyPercent"]
                    .is_null()
            );
        }
    }

    #[test]
    fn window_geometry_migrates_from_v5_and_clamps_offscreen() {
        let directory = test_directory("window-geometry");
        let path = directory.join("settings.json");
        fs::write(
            &path,
            r##"{"schemaVersion":5,"theme":{"backgroundHex":"#000000","accentHex":"#55D9FF"},"shuffle":false,"repeatMode":"off","sourceRegistryAuthoritative":true,"sources":[]}"##,
        )
        .expect("write v5 settings");
        let (settings, migrated) = read_settings(&path).expect("read v5");
        assert!(migrated);
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.window_geometry.is_none());

        let offscreen = WindowGeometry {
            x: -8000,
            y: -8000,
            width: 200,
            height: 200,
            maximized: false,
        };
        let clamped = clamp_window_geometry_to_monitors(offscreen, &[(0, 0, 1920, 1080)]);
        assert_eq!(clamped.width, WindowGeometry::MIN_WIDTH);
        assert_eq!(clamped.height, WindowGeometry::MIN_HEIGHT);
        assert!(clamped.x >= 0 && clamped.y >= 0);

        let store = SettingsStore::open(&path, AppSettings::default()).expect("open");
        store
            .update(|settings| {
                settings.window_geometry = Some(WindowGeometry {
                    x: 120,
                    y: 80,
                    width: 1100,
                    height: 700,
                    maximized: true,
                });
                Ok(())
            })
            .expect("persist geometry");
        let reopened = SettingsStore::open(&path, AppSettings::default())
            .expect("reopen")
            .snapshot()
            .expect("snapshot");
        let geo = reopened.window_geometry.expect("geometry saved");
        assert_eq!(geo.x, 120);
        assert_eq!(geo.width, 1100);
        assert!(geo.maximized);
    }
}
