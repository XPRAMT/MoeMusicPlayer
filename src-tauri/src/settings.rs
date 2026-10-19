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

const SETTINGS_SCHEMA_VERSION: u32 = 9;
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
    /// Opacity multiplier for shared control and surface borders (0–100).
    #[serde(default = "default_border_opacity_percent")]
    pub border_opacity_percent: u8,
    /// Opacity multiplier for shared content separators (0–100).
    #[serde(default = "default_divider_opacity_percent")]
    pub divider_opacity_percent: u8,
    #[serde(default)]
    pub color_mode: ThemeColorMode,
    #[serde(default)]
    pub background_opacity_text_color: BackgroundOpacityTextColor,
    /// Main-window target clarity for automatic image opacity (0–100).
    #[serde(default)]
    pub main_background_clarity_percent: u8,
    /// Now Playing cover target clarity for automatic image opacity (0–100).
    #[serde(default)]
    pub cover_background_clarity_percent: u8,
    #[serde(default = "default_quick_settings_opacity_percent")]
    pub quick_settings_opacity_percent: u8,
    /// Backdrop blur for the Now Playing quick-settings drawer.
    #[serde(default = "default_quick_settings_blur_px")]
    pub quick_settings_blur_px: u8,
    /// Main-window photo background blur. The image itself lives at `UserData/background`.
    #[serde(default = "default_main_background_blur_px")]
    pub main_background_blur_px: u8,
    /// Image opacity percent (0–100). UI label is 背景透明度.
    #[serde(
        default = "default_main_background_opacity_percent",
        alias = "mainBackgroundBrightnessPercent"
    )]
    pub main_background_opacity_percent: u8,
    /// Main-window background source. Legacy settings are resolved in `SettingsStore::open`.
    #[serde(default = "legacy_main_background_mode")]
    pub main_background_mode: MainBackgroundMode,
    /// Recursive image folder for slideshow mode.
    #[serde(default)]
    pub slideshow_folder: Option<StoredPath>,
    /// Slideshow interval in minutes (1–120).
    #[serde(default = "default_slideshow_interval_minutes")]
    pub slideshow_interval_minutes: u8,
    #[serde(default)]
    pub slideshow_order: SlideshowOrder,
    /// Relative-luminance Y threshold (0–100). At/above → black text; below → white text.
    #[serde(default = "default_auto_text_color_threshold_percent")]
    pub auto_text_color_threshold_percent: u8,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeColorMode {
    #[default]
    AutoTextColor,
    AutoBackgroundOpacity,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BackgroundOpacityTextColor {
    #[default]
    White,
    Black,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MainBackgroundMode {
    /// Compatibility default for pre-v9 settings; open() resolves this based on the legacy image.
    #[default]
    SingleImage,
    Solid,
    Slideshow,
    AlbumCover,
    SystemWallpaper,
}

fn legacy_main_background_mode() -> MainBackgroundMode {
    MainBackgroundMode::SingleImage
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SlideshowOrder {
    #[default]
    Sequential,
    Random,
}

fn default_slideshow_interval_minutes() -> u8 {
    10
}

fn default_quick_settings_opacity_percent() -> u8 {
    70
}

fn default_quick_settings_blur_px() -> u8 {
    12
}

fn default_main_background_blur_px() -> u8 {
    20
}

fn default_main_background_opacity_percent() -> u8 {
    40
}

fn default_auto_text_color_threshold_percent() -> u8 {
    50
}

fn default_border_opacity_percent() -> u8 {
    100
}

fn default_divider_opacity_percent() -> u8 {
    100
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            background_hex: "#000000".to_owned(),
            accent_hex: "#55D9FF".to_owned(),
            border_opacity_percent: default_border_opacity_percent(),
            divider_opacity_percent: default_divider_opacity_percent(),
            color_mode: ThemeColorMode::default(),
            background_opacity_text_color: BackgroundOpacityTextColor::default(),
            main_background_clarity_percent: 0,
            cover_background_clarity_percent: 0,
            quick_settings_opacity_percent: default_quick_settings_opacity_percent(),
            quick_settings_blur_px: default_quick_settings_blur_px(),
            main_background_blur_px: default_main_background_blur_px(),
            main_background_opacity_percent: default_main_background_opacity_percent(),
            main_background_mode: MainBackgroundMode::Solid,
            slideshow_folder: None,
            slideshow_interval_minutes: default_slideshow_interval_minutes(),
            slideshow_order: SlideshowOrder::default(),
            auto_text_color_threshold_percent: default_auto_text_color_threshold_percent(),
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
    /// Display-only Simplified-to-Traditional conversion. Not written into lyric files.
    #[serde(default)]
    pub simplified_to_traditional: bool,
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
            simplified_to_traditional: false,
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
        if !(1..=50).contains(&self.primary_font_size_px) {
            return Err(SettingsError::InvalidData(
                "primaryFontSizePx must be between 1 and 50".into(),
            ));
        }
        if !(1..=50).contains(&self.auxiliary_font_size_px) {
            return Err(SettingsError::InvalidData(
                "auxiliaryFontSizePx must be between 1 and 50".into(),
            ));
        }
        if self.line_gap_px > 50 {
            return Err(SettingsError::InvalidData(
                "lineGapPx must be between 0 and 50".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverCornerStyle {
    #[default]
    Rounded,
    Square,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TimelineStyle {
    #[default]
    Edge,
}

impl<'de> Deserialize<'de> for TimelineStyle {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.as_str() {
            // The line look is gone. Older line/bar/minimal values stay valid and become edge.
            "edge" | "line" | "bar" | "minimal" => Ok(Self::Edge),
            other => Err(serde::de::Error::unknown_variant(other, &["edge", "line"])),
        }
    }
}

fn default_timeline_style() -> TimelineStyle {
    TimelineStyle::Edge
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LyricsTextEffect {
    #[default]
    Shadow,
    Stroke,
    None,
}

fn default_lyrics_text_effect() -> LyricsTextEffect {
    LyricsTextEffect::Shadow
}

fn default_cover_corner_style() -> CoverCornerStyle {
    CoverCornerStyle::Rounded
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlayingAppearancePreferences {
    pub background_blur_px: u8,
    /// Cover backdrop image opacity percent (0–100). UI label is 封面背景透明度.
    #[serde(
        default = "default_background_opacity_percent",
        alias = "backgroundBrightnessPercent"
    )]
    pub background_opacity_percent: u8,
    #[serde(default = "default_cover_corner_style")]
    pub cover_corner_style: CoverCornerStyle,
    #[serde(default = "default_timeline_style")]
    pub timeline_style: TimelineStyle,
}

fn default_background_opacity_percent() -> u8 {
    40
}

impl Default for NowPlayingAppearancePreferences {
    fn default() -> Self {
        Self {
            background_blur_px: 20,
            background_opacity_percent: 40,
            cover_corner_style: CoverCornerStyle::Rounded,
            timeline_style: TimelineStyle::Edge,
        }
    }
}

impl NowPlayingAppearancePreferences {
    fn validate(&self) -> Result<(), SettingsError> {
        if self.background_blur_px > 50 {
            return Err(SettingsError::InvalidData(
                "backgroundBlurPx must be between 0 and 50".into(),
            ));
        }
        if self.background_opacity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "backgroundOpacityPercent must be between 0 and 100".into(),
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
    CreatedAt,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackListFlexibleWidths {
    pub title: u32,
    pub artist: u32,
    pub album: u32,
}

impl Default for TrackListFlexibleWidths {
    fn default() -> Self {
        Self {
            title: 220,
            artist: 150,
            album: 160,
        }
    }
}

impl TrackListFlexibleWidths {
    pub(crate) fn clamp(self) -> Self {
        const MIN: u32 = 80;
        const MAX: u32 = 800;
        Self {
            title: self.title.clamp(MIN, MAX),
            artist: self.artist.clamp(MIN, MAX),
            album: self.album.clamp(MIN, MAX),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySortDirection {
    #[default]
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryListSort {
    pub field: TrackListColumnId,
    pub direction: LibrarySortDirection,
}

impl Default for LibraryListSort {
    fn default() -> Self {
        Self {
            field: TrackListColumnId::Title,
            direction: LibrarySortDirection::Asc,
        }
    }
}

/// Configurable information columns. Row index and play action are deliberately absent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackListColumnSettings {
    /// Column order is represented by array order and must contain every information column once.
    pub columns: Vec<TrackListColumnPreference>,
    #[serde(default)]
    pub widths: TrackListFlexibleWidths,
    #[serde(default)]
    pub library_sort: LibraryListSort,
    #[serde(default = "default_track_row_height_px")]
    pub row_height_px: u8,
    /// Album home cards fill the row; each card is at most this many CSS px wide.
    #[serde(default = "default_album_card_max_width_px")]
    pub album_card_max_width_px: u16,
    /// Group headings and quick-index grouping across sortable lists.
    #[serde(default = "default_grouping_enabled")]
    pub grouping_enabled: bool,
    /// How text is elided when a shared list cell is narrower than its content.
    #[serde(default = "default_track_text_overflow_mode")]
    pub text_overflow_mode: TrackTextOverflowMode,
}

/// Persisted sort selection for the artist index list.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistListSort {
    pub field: ArtistListSortField,
    pub direction: ArtistListSortDirection,
}

impl Default for ArtistListSort {
    fn default() -> Self {
        Self {
            field: ArtistListSortField::Title,
            direction: ArtistListSortDirection::Asc,
        }
    }
}

/// Persisted sort selection for the album index page.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumIndexSort {
    pub field: AlbumIndexSortField,
    pub direction: ArtistListSortDirection,
}

impl Default for AlbumIndexSort {
    fn default() -> Self {
        Self {
            field: AlbumIndexSortField::Title,
            direction: ArtistListSortDirection::Asc,
        }
    }
}

/// Persisted sort selection for tracks inside an album detail page.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumDetailSort {
    pub field: AlbumDetailSortField,
    pub direction: LibrarySortDirection,
}

impl Default for AlbumDetailSort {
    fn default() -> Self {
        Self {
            field: AlbumDetailSortField::Position,
            direction: LibrarySortDirection::Asc,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlbumDetailSortField {
    Position,
    Title,
    Artist,
    Album,
    Year,
    CreatedAt,
    AudioFormat,
    Duration,
    PlayCount,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlbumIndexSortField {
    Title,
    Duration,
    Played,
    TrackCount,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtistListSortField {
    Title,
    TrackCount,
    AlbumCount,
    /// Legacy wire name retained: `duration` means actual accumulated play time.
    Duration,
    TrackDuration,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtistListSortDirection {
    Asc,
    Desc,
}

fn default_track_row_height_px() -> u8 {
    50
}

pub(crate) const MIN_ALBUM_CARD_MAX_WIDTH_PX: u16 = 100;
pub(crate) const MAX_ALBUM_CARD_MAX_WIDTH_PX: u16 = 500;

fn default_album_card_max_width_px() -> u16 {
    180
}

pub(crate) fn clamp_album_card_max_width_px(value: u16) -> u16 {
    value.clamp(MIN_ALBUM_CARD_MAX_WIDTH_PX, MAX_ALBUM_CARD_MAX_WIDTH_PX)
}

fn default_grouping_enabled() -> bool {
    true
}

fn default_track_text_overflow_mode() -> TrackTextOverflowMode {
    TrackTextOverflowMode::Middle
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TrackTextOverflowMode {
    #[default]
    Middle,
    End,
}

impl Default for TrackListColumnSettings {
    fn default() -> Self {
        Self {
            columns: [
                TrackListColumnId::Title,
                TrackListColumnId::Artist,
                TrackListColumnId::Album,
                TrackListColumnId::Year,
                TrackListColumnId::CreatedAt,
                TrackListColumnId::AudioFormat,
                TrackListColumnId::Duration,
                TrackListColumnId::PlayCount,
            ]
            .into_iter()
            .map(|id| TrackListColumnPreference { id, visible: true })
            .collect(),
            widths: TrackListFlexibleWidths::default(),
            library_sort: LibraryListSort::default(),
            row_height_px: default_track_row_height_px(),
            album_card_max_width_px: default_album_card_max_width_px(),
            grouping_enabled: default_grouping_enabled(),
            text_overflow_mode: default_track_text_overflow_mode(),
        }
    }
}

impl TrackListColumnSettings {
    fn validate(&self) -> Result<(), SettingsError> {
        use TrackListColumnId::{
            Album, Artist, AudioFormat, CreatedAt, Duration, PlayCount, Title, Year,
        };

        const REQUIRED: [TrackListColumnId; 8] = [
            Title,
            Artist,
            Album,
            Year,
            CreatedAt,
            AudioFormat,
            Duration,
            PlayCount,
        ];
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
                "track list columns must contain title, artist, album, year, createdAt, audioFormat, duration, and playCount exactly once".into(),
            ));
        }
        Ok(())
    }

    fn normalize(mut self) -> Self {
        self.widths = self.widths.clamp();
        self.row_height_px = self.row_height_px.clamp(30, 60);
        self.album_card_max_width_px = clamp_album_card_max_width_px(self.album_card_max_width_px);
        self
    }
}

/// Accept persisted preferences that predate the createdAt column.
fn repair_track_list_columns(settings: TrackListColumnSettings) -> TrackListColumnSettings {
    use TrackListColumnId::{
        Album, Artist, AudioFormat, CreatedAt, Duration, PlayCount, Title, Year,
    };

    if settings.validate().is_ok() {
        return settings.normalize();
    }

    const LEGACY_WITH_PLAY_COUNT: [TrackListColumnId; 7] =
        [Title, Artist, Album, Year, AudioFormat, Duration, PlayCount];
    const LEGACY_WITHOUT_PLAY_COUNT: [TrackListColumnId; 6] =
        [Title, Artist, Album, Year, AudioFormat, Duration];
    let ids: Vec<_> = settings.columns.iter().map(|column| column.id).collect();
    let unique: std::collections::HashSet<_> = ids.iter().copied().collect();
    let matches_legacy = (ids.len() == LEGACY_WITH_PLAY_COUNT.len()
        && unique.len() == LEGACY_WITH_PLAY_COUNT.len()
        && LEGACY_WITH_PLAY_COUNT.iter().all(|id| unique.contains(id)))
        || (ids.len() == LEGACY_WITHOUT_PLAY_COUNT.len()
            && unique.len() == LEGACY_WITHOUT_PLAY_COUNT.len()
            && LEGACY_WITHOUT_PLAY_COUNT
                .iter()
                .all(|id| unique.contains(id)));
    if matches_legacy && !unique.contains(&CreatedAt) {
        let mut columns = settings.columns;
        // Preserve the user's existing column order and visibility while introducing
        // the new field at its documented default position.
        let insertion = columns
            .iter()
            .position(|column| column.id == AudioFormat)
            .unwrap_or(columns.len());
        columns.insert(
            insertion,
            TrackListColumnPreference {
                id: CreatedAt,
                visible: true,
            },
        );
        if !unique.contains(&PlayCount) {
            columns.push(TrackListColumnPreference {
                id: PlayCount,
                visible: true,
            });
        }
        return TrackListColumnSettings {
            columns,
            widths: settings.widths.clamp(),
            library_sort: settings.library_sort,
            row_height_px: settings.row_height_px.clamp(30, 60),
            album_card_max_width_px: clamp_album_card_max_width_px(
                settings.album_card_max_width_px,
            ),
            grouping_enabled: settings.grouping_enabled,
            text_overflow_mode: settings.text_overflow_mode,
        };
    }

    TrackListColumnSettings::default()
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NowPlayingLayout {
    A,
    B,
}

/// How Windows playback converts tracks whose sample rate differs from the
/// output. Both modes use WASAPI shared mode; neither is bit-perfect.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResamplingMode {
    /// Open the stream at the track's rate; the Windows audio engine converts.
    WindowsBuiltin,
    /// Keep the mix rate and convert in-app with the FFT resampler.
    #[default]
    HighQuality,
}

/// An unknown or malformed stored value falls back to the default instead of
/// rejecting the whole settings file.
/// Now Playing cover size remembered across restarts. Only a **shrunk** cover is remembered (0.5 to 1, 1 = original
/// size): an enlarged cover, and any pan, is not saved, so the next start begins at the original size. Within a run the
/// zoom (enlarged or shrunk) stays until the user double-clicks the cover.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverZoomSettings {
    pub scale: f32,
}

impl CoverZoomSettings {
    pub const SCALE_RANGE: (f32, f32) = (0.5, 1.0);

    /// A scale above 1 (an enlarged cover) or a non-finite one is not remembered: it becomes 1.
    pub fn normalized(self) -> Self {
        let scale = if self.scale.is_finite() {
            self.scale.clamp(Self::SCALE_RANGE.0, Self::SCALE_RANGE.1)
        } else {
            1.0
        };
        Self { scale }
    }
}

impl Default for CoverZoomSettings {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

/// Missing or malformed values (and the pan fields an earlier build wrote) are ignored.
fn lenient_cover_zoom(value: Option<serde_json::Value>) -> CoverZoomSettings {
    let defaults = CoverZoomSettings::default();
    let scale = value
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .and_then(|object| object.get("scale"))
        .and_then(serde_json::Value::as_f64)
        .map_or(defaults.scale, |scale| scale as f32);
    CoverZoomSettings { scale }.normalized()
}

/// Where the text sits inside that stretch.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskbarLyricsAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Interface language (ADR 0019): follow the system, or a fixed language.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum UiLanguage {
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "zh-TW")]
    ZhTw,
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "en")]
    En,
}

/// An unknown or malformed value falls back to following the system.
fn lenient_language(value: Option<serde_json::Value>) -> UiLanguage {
    value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

/// Lyrics drawn on the free part of the Windows taskbar (ADR 0018). Stored everywhere so the file stays portable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskbarLyricsSettings {
    pub enabled: bool,
    pub align: TaskbarLyricsAlign,
    /// CSS px of the main line.
    pub font_size_px: u8,
    /// Size of the translation line as a percentage of the main line (100 = the same size).
    pub translation_scale_percent: u8,
    /// False: black or white by the taskbar's own light/dark theme.
    pub use_custom_color: bool,
    /// `#rrggbb`.
    pub custom_color: String,
    /// A second, smaller line with the translation when the lyrics have one.
    pub show_translation: bool,
    /// Shadow, outline or nothing, the same choices as the lyrics on the Now Playing page. The effect colour is the
    /// opposite of the text colour (black behind light text, white behind dark text). Default none: the text as it was.
    #[serde(default = "default_taskbar_lyrics_text_effect")]
    pub text_effect: LyricsTextEffect,
}

impl TaskbarLyricsSettings {
    pub const FONT_SIZE_RANGE_PX: (u8, u8) = (1, 50);
    pub const DEFAULT_FONT_SIZE_PX: u8 = 16;
    pub const TRANSLATION_SCALE_RANGE_PERCENT: (u8, u8) = (1, 100);
    pub const DEFAULT_TRANSLATION_SCALE_PERCENT: u8 = 75;
    pub const DEFAULT_CUSTOM_COLOR: &'static str = "#ffffff";

    pub fn normalized(mut self) -> Self {
        self.font_size_px = self
            .font_size_px
            .clamp(Self::FONT_SIZE_RANGE_PX.0, Self::FONT_SIZE_RANGE_PX.1);
        self.translation_scale_percent = self.translation_scale_percent.clamp(
            Self::TRANSLATION_SCALE_RANGE_PERCENT.0,
            Self::TRANSLATION_SCALE_RANGE_PERCENT.1,
        );
        if !is_hex_color(&self.custom_color) {
            self.custom_color = Self::DEFAULT_CUSTOM_COLOR.to_owned();
        }
        self.custom_color.make_ascii_lowercase();
        self
    }
}

impl Default for TaskbarLyricsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            align: TaskbarLyricsAlign::default(),
            font_size_px: Self::DEFAULT_FONT_SIZE_PX,
            translation_scale_percent: Self::DEFAULT_TRANSLATION_SCALE_PERCENT,
            use_custom_color: false,
            custom_color: Self::DEFAULT_CUSTOM_COLOR.to_owned(),
            show_translation: false,
            text_effect: default_taskbar_lyrics_text_effect(),
        }
    }
}

fn default_taskbar_lyrics_text_effect() -> LyricsTextEffect {
    LyricsTextEffect::None
}

fn is_hex_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Missing, malformed or out-of-range values fall back to the defaults field by field.
fn lenient_taskbar_lyrics(value: Option<serde_json::Value>) -> TaskbarLyricsSettings {
    let defaults = TaskbarLyricsSettings::default();
    let Some(object) = value.as_ref().and_then(serde_json::Value::as_object) else {
        return defaults;
    };
    let field = |name: &str| object.get(name).cloned();
    TaskbarLyricsSettings {
        enabled: field("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.enabled),
        align: field("align")
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or(defaults.align),
        font_size_px: field("fontSizePx")
            .and_then(|value| value.as_u64())
            .map_or(defaults.font_size_px, |size| size.min(255) as u8),
        translation_scale_percent: field("translationScalePercent")
            .and_then(|value| value.as_u64())
            .map_or(defaults.translation_scale_percent, |scale| scale.min(255) as u8),
        use_custom_color: field("useCustomColor")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.use_custom_color),
        custom_color: field("customColor")
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or(defaults.custom_color),
        show_translation: field("showTranslation")
            .and_then(|value| value.as_bool())
            .unwrap_or(defaults.show_translation),
        text_effect: field("textEffect")
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or(defaults.text_effect),
    }
    .normalized()
}

/// DSD→PCM playback settings (Windows; stored everywhere so the file stays portable): the digital gain in dB
/// (1 to 6) and the PCM rate DSD is converted to (176.4 or 352.8 kHz).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DsdPlaybackSettings {
    pub gain_db: f32,
    pub output_rate_hz: u32,
    /// Send DSD to the device as DoP in exclusive mode instead of converting it to PCM (the gain and the rate above
    /// then do not apply). Default off.
    #[serde(default)]
    pub dop: bool,
}

impl DsdPlaybackSettings {
    pub const GAIN_RANGE_DB: (f32, f32) = (0.0, 6.0);
    pub const OUTPUT_RATES_HZ: [u32; 2] = [176_400, 352_800];
    pub const DEFAULT_GAIN_DB: f32 = 3.0;

    /// A gain outside the range is clamped (a non-finite one becomes the default); an unsupported rate becomes 176.4 kHz.
    pub fn normalized(self) -> Self {
        let gain_db = if self.gain_db.is_finite() {
            self.gain_db.clamp(Self::GAIN_RANGE_DB.0, Self::GAIN_RANGE_DB.1)
        } else {
            Self::DEFAULT_GAIN_DB
        };
        let output_rate_hz = if Self::OUTPUT_RATES_HZ.contains(&self.output_rate_hz) {
            self.output_rate_hz
        } else {
            Self::OUTPUT_RATES_HZ[0]
        };
        Self { gain_db, output_rate_hz, dop: self.dop }
    }
}

impl Default for DsdPlaybackSettings {
    fn default() -> Self {
        Self {
            gain_db: Self::DEFAULT_GAIN_DB,
            output_rate_hz: Self::OUTPUT_RATES_HZ[0],
            dop: false,
        }
    }
}

/// Missing, malformed or out-of-range values fall back to the defaults field by field.
fn lenient_dsd_settings(value: Option<serde_json::Value>) -> DsdPlaybackSettings {
    let defaults = DsdPlaybackSettings::default();
    let Some(object) = value.as_ref().and_then(serde_json::Value::as_object) else {
        return defaults;
    };
    DsdPlaybackSettings {
        gain_db: object
            .get("gainDb")
            .and_then(serde_json::Value::as_f64)
            .map_or(defaults.gain_db, |gain| gain as f32),
        output_rate_hz: object
            .get("outputRateHz")
            .and_then(serde_json::Value::as_u64)
            .and_then(|rate| u32::try_from(rate).ok())
            .unwrap_or(defaults.output_rate_hz),
        dop: object
            .get("dop")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(defaults.dop),
    }
    .normalized()
}

/// Where one output channel of a device takes its sound from (Settings -> output -> device details).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRouteEntry {
    /// 0-based source channel, or `None` for silence.
    pub source: Option<u8>,
    pub gain_percent: u8,
}

/// One chosen output device. `id` is `None` for "the system default". The first enabled entry is the main output; the
/// others play the same music at the same time. An empty list means the system default alone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDeviceEntry {
    pub id: Option<String>,
    /// The device's name when it was last seen, so the list can name a device that is unplugged right now.
    #[serde(default)]
    pub name: String,
    pub enabled: bool,
    pub delay_ms: u32,
    /// One route per output channel of the device; `None` is automatic.
    pub channel_map: Option<Vec<ChannelRouteEntry>>,
}

impl OutputDeviceEntry {
    pub const MAX_DELAY_MS: u32 = 2000;
    pub const MAX_ENTRIES: usize = 16;
    pub const MAX_CHANNELS: usize = 32;

    pub fn normalized(mut self) -> Self {
        self.id = self.id.filter(|id| !id.trim().is_empty());
        self.name = self.name.trim().chars().take(200).collect();
        self.delay_ms = self.delay_ms.min(Self::MAX_DELAY_MS);
        if let Some(routes) = self.channel_map.as_mut() {
            routes.truncate(Self::MAX_CHANNELS);
            for route in routes.iter_mut() {
                route.gain_percent = route.gain_percent.min(100);
            }
        }
        self
    }
}

/// Normalise, drop repeated devices (the first entry of a device wins) and cap the list.
pub fn normalize_output_devices(devices: Vec<OutputDeviceEntry>) -> Vec<OutputDeviceEntry> {
    let mut result: Vec<OutputDeviceEntry> = Vec::new();
    for device in devices {
        let device = device.normalized();
        if result.iter().all(|existing| existing.id != device.id) && result.len() < OutputDeviceEntry::MAX_ENTRIES {
            result.push(device);
        }
    }
    result
}

/// Missing or malformed entries and fields fall back to the defaults one by one.
fn lenient_output_devices(value: Option<serde_json::Value>) -> Vec<OutputDeviceEntry> {
    let Some(items) = value.as_ref().and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let devices = items
        .iter()
        .filter_map(serde_json::Value::as_object)
        .map(|object| OutputDeviceEntry {
            id: object.get("id").and_then(serde_json::Value::as_str).map(str::to_owned),
            name: object
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            enabled: object.get("enabled").and_then(serde_json::Value::as_bool).unwrap_or(true),
            delay_ms: object
                .get("delayMs")
                .and_then(serde_json::Value::as_u64)
                .map_or(0, |value| value.min(u64::from(OutputDeviceEntry::MAX_DELAY_MS)) as u32),
            channel_map: object.get("channelMap").and_then(serde_json::Value::as_array).map(|routes| {
                routes
                    .iter()
                    .map(|route| ChannelRouteEntry {
                        source: route
                            .get("source")
                            .and_then(serde_json::Value::as_u64)
                            .and_then(|source| u8::try_from(source).ok()),
                        gain_percent: route
                            .get("gainPercent")
                            .and_then(serde_json::Value::as_u64)
                            .map_or(100, |gain| gain.min(100) as u8),
                    })
                    .collect()
            }),
        })
        .collect();
    normalize_output_devices(devices)
}

/// The output-device presets: always five slots, each empty or a saved set of devices.
pub const OUTPUT_PRESET_SLOTS: usize = 5;

pub fn normalize_output_presets(
    presets: Vec<Option<Vec<OutputDeviceEntry>>>,
) -> Vec<Option<Vec<OutputDeviceEntry>>> {
    let mut slots: Vec<Option<Vec<OutputDeviceEntry>>> = presets
        .into_iter()
        .take(OUTPUT_PRESET_SLOTS)
        .map(|slot| slot.map(normalize_output_devices).filter(|devices| !devices.is_empty()))
        .collect();
    slots.resize(OUTPUT_PRESET_SLOTS, None);
    slots
}

fn default_output_presets() -> Vec<Option<Vec<OutputDeviceEntry>>> {
    vec![None; OUTPUT_PRESET_SLOTS]
}

fn lenient_output_presets(value: Option<serde_json::Value>) -> Vec<Option<Vec<OutputDeviceEntry>>> {
    let slots = value
        .as_ref()
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.is_array().then(|| lenient_output_devices(Some(item.clone()))))
                .collect()
        })
        .unwrap_or_default();
    normalize_output_presets(slots)
}

fn lenient_resampling_mode(value: Option<serde_json::Value>) -> ResamplingMode {
    value
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
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

/// A filesystem folder skipped during Windows library walks (and playlist media under it).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedFolderEntry {
    pub id: SourceId,
    pub display_name: String,
    pub path: StoredPath,
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

    /// Minimized Windows snapshots use a sentinel position and tiny dimensions.
    /// They must not be treated as the user's normal launch size.
    pub fn is_valid_restore_snapshot(&self) -> bool {
        self.width >= Self::MIN_WIDTH
            && self.height >= Self::MIN_HEIGHT
            && !(self.x <= -30_000 && self.y <= -30_000)
    }

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
        width: geometry
            .width
            .min(u32::try_from(mw.max(1)).unwrap_or(geometry.width)),
        height: geometry
            .height
            .min(u32::try_from(mh.max(1)).unwrap_or(geometry.height)),
        maximized: geometry.maximized,
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutDevice {
    Keyboard,
    Mouse,
    Gamepad,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub device: ShortcutDevice,
    pub code: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutSettings {
    pub fullscreen: Vec<ShortcutBinding>,
    pub play_pause: Vec<ShortcutBinding>,
    pub seek_back: Vec<ShortcutBinding>,
    pub seek_forward: Vec<ShortcutBinding>,
    pub previous: Vec<ShortcutBinding>,
    pub next: Vec<ShortcutBinding>,
}

fn keyboard_binding(code: &str) -> ShortcutBinding {
    ShortcutBinding {
        device: ShortcutDevice::Keyboard,
        code: code.to_owned(),
    }
}

fn mouse_binding(code: &str) -> ShortcutBinding {
    ShortcutBinding {
        device: ShortcutDevice::Mouse,
        code: code.to_owned(),
    }
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        Self {
            fullscreen: vec![keyboard_binding("F11")],
            play_pause: vec![keyboard_binding("Space")],
            seek_back: vec![keyboard_binding("ArrowLeft"), mouse_binding("wheelDown")],
            seek_forward: vec![keyboard_binding("ArrowRight"), mouse_binding("wheelUp")],
            previous: vec![keyboard_binding("PageUp"), mouse_binding("back")],
            next: vec![keyboard_binding("PageDown"), mouse_binding("forward")],
        }
    }
}

impl ShortcutSettings {
    fn validate(&self) -> Result<(), SettingsError> {
        for bindings in [
            &self.fullscreen,
            &self.play_pause,
            &self.seek_back,
            &self.seek_forward,
            &self.previous,
            &self.next,
        ] {
            if bindings.len() > 4 {
                return Err(SettingsError::InvalidData(
                    "each shortcut action accepts at most 4 bindings".into(),
                ));
            }
            for binding in bindings {
                let ok = match binding.device {
                    ShortcutDevice::Keyboard => {
                        !binding.code.is_empty()
                            && binding.code.len() <= 32
                            && binding.code.chars().all(|ch| ch.is_ascii_alphanumeric())
                    }
                    ShortcutDevice::Mouse => {
                        matches!(
                            binding.code.as_str(),
                            "wheelUp" | "wheelDown" | "back" | "forward"
                        )
                    }
                    ShortcutDevice::Gamepad => {
                        binding
                            .code
                            .strip_prefix("button")
                            .and_then(|index| index.parse::<u8>().ok())
                            .is_some_and(|index| index <= 15)
                            || matches!(
                                binding.code.as_str(),
                                "stickLeftXMinus"
                                    | "stickLeftXPlus"
                                    | "stickLeftYMinus"
                                    | "stickLeftYPlus"
                                    | "stickRightXMinus"
                                    | "stickRightXPlus"
                                    | "stickRightYMinus"
                                    | "stickRightYPlus"
                            )
                    }
                };
                if !ok {
                    return Err(SettingsError::InvalidData(
                        "shortcut binding is not a supported key or mouse control".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn lenient_shortcuts(value: Option<serde_json::Value>) -> ShortcutSettings {
    value
        .and_then(|value| serde_json::from_value::<ShortcutSettings>(value).ok())
        .filter(|shortcuts| shortcuts.validate().is_ok())
        .map(correct_legacy_wheel_direction)
        .unwrap_or_default()
}

/// The first release bound wheel-up to seek backward. Flip that saved pair once.
fn correct_legacy_wheel_direction(mut shortcuts: ShortcutSettings) -> ShortcutSettings {
    let back_up = shortcuts
        .seek_back
        .iter()
        .any(|binding| binding.device == ShortcutDevice::Mouse && binding.code == "wheelUp");
    let forward_down = shortcuts
        .seek_forward
        .iter()
        .any(|binding| binding.device == ShortcutDevice::Mouse && binding.code == "wheelDown");
    if !(back_up && forward_down) {
        return shortcuts;
    }
    for binding in shortcuts
        .seek_back
        .iter_mut()
        .chain(shortcuts.seek_forward.iter_mut())
    {
        if binding.device != ShortcutDevice::Mouse {
            continue;
        }
        if binding.code == "wheelUp" {
            binding.code = "wheelDown".to_owned();
        } else if binding.code == "wheelDown" {
            binding.code = "wheelUp".to_owned();
        }
    }
    shortcuts
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub schema_version: u32,
    pub theme: ThemeSettings,
    pub lyrics_preferences: LyricsPreferences,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub track_list_columns: TrackListColumnSettings,
    /// Sort field and direction for the artist index page.
    #[serde(default)]
    pub artist_list_sort: ArtistListSort,
    /// Sort field and direction for the album index page.
    #[serde(default)]
    pub album_index_sort: AlbumIndexSort,
    /// Sort field and direction for tracks inside an album detail page.
    #[serde(default)]
    pub album_detail_sort: AlbumDetailSort,
    pub now_playing_layout: NowPlayingLayout,
    pub now_playing_appearance_preferences: NowPlayingAppearancePreferences,
    /// Windows playback sample-rate conversion (schema 7).
    #[serde(default)]
    pub resampling_mode: ResamplingMode,
    /// Windows-only Sony DSEE HX streaming switch (schema 8). Default off.
    #[serde(default)]
    pub dsee_hx: bool,
    /// False means the source registry came from defaults after both JSON copies failed.
    /// Sync must remain paused until the user rebuilds and confirms the registry.
    pub source_registry_authoritative: bool,
    pub sources: Vec<SourceEntry>,
    /// Windows folders omitted from source walks. Missing values mean none excluded.
    #[serde(default)]
    pub excluded_folders: Vec<ExcludedFolderEntry>,
    /// Last main-window outer position / inner size / maximized flag (desktop).
    #[serde(default)]
    pub window_geometry: Option<WindowGeometry>,
    /// Keyboard and mouse bindings. Missing values keep the player defaults.
    #[serde(default)]
    pub shortcuts: ShortcutSettings,
    /// True when the last run ended (normal close or update restart) while audio was playing. Set at
    /// shutdown, consumed (reset to false) on the next launch to resume playback. Windows only.
    #[serde(default)]
    pub resume_playback: bool,
    /// The update version the user chose Ignore for: the same version is not offered again on startup, only when the
    /// user checks for updates by hand.
    #[serde(default)]
    pub ignored_update_version: Option<String>,
    /// DSD→PCM gain and output rate (Settings → output).
    #[serde(default)]
    pub dsd: DsdPlaybackSettings,
    /// The chosen output devices (Settings -> output); empty is the system default alone.
    #[serde(default)]
    pub output_devices: Vec<OutputDeviceEntry>,
    /// Three saved sets of output devices the settings page switches between (`None`: an empty slot).
    #[serde(default = "default_output_presets")]
    pub output_presets: Vec<Option<Vec<OutputDeviceEntry>>>,
    /// WASAPI exclusive output (Windows): one device, bit-perfect, no mixing with other programs; the multi-device
    /// output is off while it is on (ADR 0020).
    #[serde(default)]
    pub output_exclusive: bool,
    /// "音量同步": the chosen playback devices' real volumes move together in proportion (works all the time, in the
    /// backend, whether or not a settings UI is open).
    pub output_volume_sync: bool,
    /// Now Playing cover size remembered across restarts (shrunk only; see `CoverZoomSettings`).
    #[serde(default)]
    pub cover_zoom: CoverZoomSettings,
    /// Lyrics on the Windows taskbar (Settings → lyrics).
    #[serde(default)]
    pub taskbar_lyrics: TaskbarLyricsSettings,
    /// Interface language (Settings → main).
    #[serde(default)]
    pub language: UiLanguage,
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
            artist_list_sort: ArtistListSort::default(),
            album_index_sort: AlbumIndexSort::default(),
            album_detail_sort: AlbumDetailSort::default(),
            now_playing_layout: NowPlayingLayout::A,
            now_playing_appearance_preferences: NowPlayingAppearancePreferences::default(),
            resampling_mode: ResamplingMode::default(),
            dsee_hx: false,
            source_registry_authoritative: true,
            sources: Vec::new(),
            excluded_folders: Vec::new(),
            window_geometry: None,
            shortcuts: ShortcutSettings::default(),
            resume_playback: false,
            ignored_update_version: None,
            dsd: DsdPlaybackSettings::default(),
            output_devices: Vec::new(),
            output_presets: default_output_presets(),
            output_exclusive: false,
            output_volume_sync: false,
            cover_zoom: CoverZoomSettings::default(),
            taskbar_lyrics: TaskbarLyricsSettings::default(),
            language: UiLanguage::default(),
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
                ..ThemeSettings::default()
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
        if self.theme.quick_settings_opacity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "quickSettingsOpacityPercent must be between 0 and 100".into(),
            ));
        }
        if self.theme.quick_settings_blur_px > 50 {
            return Err(SettingsError::InvalidData(
                "quickSettingsBlurPx must be between 0 and 50".into(),
            ));
        }
        if self.theme.main_background_blur_px > 50 {
            return Err(SettingsError::InvalidData(
                "mainBackgroundBlurPx must be between 0 and 50".into(),
            ));
        }
        if self.theme.main_background_opacity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "mainBackgroundOpacityPercent must be between 0 and 100".into(),
            ));
        }
        if !(1..=120).contains(&self.theme.slideshow_interval_minutes) {
            return Err(SettingsError::InvalidData(
                "slideshowIntervalMinutes must be between 1 and 120".into(),
            ));
        }
        if self.theme.auto_text_color_threshold_percent > 100 {
            return Err(SettingsError::InvalidData(
                "autoTextColorThresholdPercent must be between 0 and 100".into(),
            ));
        }
        if self.theme.border_opacity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "borderOpacityPercent must be between 0 and 100".into(),
            ));
        }
        if self.theme.divider_opacity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "dividerOpacityPercent must be between 0 and 100".into(),
            ));
        }
        if self.theme.main_background_clarity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "mainBackgroundClarityPercent must be between 0 and 100".into(),
            ));
        }
        if self.theme.cover_background_clarity_percent > 100 {
            return Err(SettingsError::InvalidData(
                "coverBackgroundClarityPercent must be between 0 and 100".into(),
            ));
        }
        self.lyrics_preferences.validate()?;
        self.now_playing_appearance_preferences.validate()?;
        self.track_list_columns.validate()?;
        self.shortcuts.validate()?;
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
        for excluded in &self.excluded_folders {
            if !ids.insert(excluded.id) {
                return Err(SettingsError::InvalidData(format!(
                    "duplicate excluded folder id {}",
                    excluded.id
                )));
            }
            if excluded.display_name.trim().is_empty() {
                return Err(SettingsError::InvalidData(
                    "excluded folder display name cannot be empty".into(),
                ));
            }
            if excluded.path.display_lossy().is_empty() {
                return Err(SettingsError::InvalidData(
                    "excluded folder path cannot be empty".into(),
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
        let (mut settings, mut recovery_warning, mut must_persist) = if path.exists() {
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
        if must_persist
            && settings.theme.main_background_mode == MainBackgroundMode::SingleImage
            && !path
                .parent()
                .is_some_and(|directory| directory.join("background").is_file())
        {
            settings.theme.main_background_mode = MainBackgroundMode::Solid;
            must_persist = true;
        }
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

    pub(crate) fn data_directory(&self) -> Option<&Path> {
        self.path.parent()
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

    pub fn upsert_excluded_folder(
        &self,
        excluded: ExcludedFolderEntry,
    ) -> Result<AppSettings, SettingsError> {
        self.update(|settings| {
            if let Some(existing) = settings
                .excluded_folders
                .iter_mut()
                .find(|item| item.id == excluded.id)
            {
                *existing = excluded;
            } else {
                settings.excluded_folders.push(excluded);
            }
            Ok(())
        })
    }

    pub fn remove_excluded_folder(
        &self,
        id: SourceId,
    ) -> Result<(AppSettings, ExcludedFolderEntry), SettingsError> {
        let mut removed = None;
        let settings = self.update(|settings| {
            let index = settings
                .excluded_folders
                .iter()
                .position(|entry| entry.id == id)
                .ok_or_else(|| SettingsError::InvalidData("excluded folder not found".into()))?;
            removed = Some(settings.excluded_folders.remove(index));
            Ok(())
        })?;
        Ok((
            settings,
            removed.expect("excluded folder removed in update"),
        ))
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
    artist_list_sort: Option<ArtistListSort>,
    album_index_sort: Option<AlbumIndexSort>,
    album_detail_sort: Option<AlbumDetailSort>,
    now_playing_layout: Option<NowPlayingLayout>,
    now_playing_appearance_preferences: Option<NowPlayingAppearancePreferences>,
    resampling_mode: Option<serde_json::Value>,
    dsee_hx: Option<serde_json::Value>,
    source_registry_authoritative: Option<bool>,
    sources: Option<Vec<SourceEntry>>,
    excluded_folders: Option<Vec<ExcludedFolderEntry>>,
    window_geometry: Option<WindowGeometry>,
    shortcuts: Option<serde_json::Value>,
    resume_playback: Option<bool>,
    ignored_update_version: Option<String>,
    dsd: Option<serde_json::Value>,
    output_devices: Option<serde_json::Value>,
    output_presets: Option<serde_json::Value>,
    output_exclusive: Option<bool>,
    output_volume_sync: Option<bool>,
    cover_zoom: Option<serde_json::Value>,
    taskbar_lyrics: Option<serde_json::Value>,
    language: Option<serde_json::Value>,
}

fn read_settings(path: &Path) -> Result<(AppSettings, bool), SettingsError> {
    let mut input = String::new();
    File::open(path)?.read_to_string(&mut input)?;
    let mut value: serde_json::Value = serde_json::from_str(&input)?;
    let legacy_clarity_migrated = value
        .get_mut("theme")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|theme| {
            let legacy = theme.remove("backgroundClarityPercent")?;
            theme
                .entry("mainBackgroundClarityPercent")
                .or_insert_with(|| legacy.clone());
            theme
                .entry("coverBackgroundClarityPercent")
                .or_insert(legacy);
            Some(())
        })
        .is_some();
    let raw: RawSettings = serde_json::from_value(value)?;
    let version = raw.schema_version.unwrap_or(0);
    if version > SETTINGS_SCHEMA_VERSION {
        return Err(SettingsError::UnsupportedVersion(version));
    }
    let mut settings = AppSettings {
        schema_version: SETTINGS_SCHEMA_VERSION,
        theme: raw.theme.unwrap_or_default(),
        lyrics_preferences: raw.lyrics_preferences.unwrap_or_default(),
        shuffle: raw.shuffle.unwrap_or(false),
        repeat_mode: raw.repeat_mode.unwrap_or(RepeatMode::Off),
        track_list_columns: repair_track_list_columns(raw.track_list_columns.unwrap_or_default()),
        artist_list_sort: raw.artist_list_sort.unwrap_or_default(),
        album_index_sort: raw.album_index_sort.unwrap_or_default(),
        album_detail_sort: raw.album_detail_sort.unwrap_or_default(),
        now_playing_layout: raw.now_playing_layout.unwrap_or(NowPlayingLayout::A),
        now_playing_appearance_preferences: raw
            .now_playing_appearance_preferences
            .unwrap_or_default(),
        resampling_mode: lenient_resampling_mode(raw.resampling_mode),
        dsee_hx: matches!(raw.dsee_hx, Some(serde_json::Value::Bool(true))),
        source_registry_authoritative: raw.source_registry_authoritative.unwrap_or(true),
        sources: raw.sources.unwrap_or_default(),
        excluded_folders: raw.excluded_folders.unwrap_or_default(),
        window_geometry: raw.window_geometry,
        shortcuts: lenient_shortcuts(raw.shortcuts),
        resume_playback: raw.resume_playback.unwrap_or(false),
        ignored_update_version: raw.ignored_update_version.filter(|version| !version.is_empty()),
        dsd: lenient_dsd_settings(raw.dsd),
        output_devices: lenient_output_devices(raw.output_devices),
        output_presets: lenient_output_presets(raw.output_presets),
        output_exclusive: raw.output_exclusive.unwrap_or(false),
        output_volume_sync: raw.output_volume_sync.unwrap_or(false),
        cover_zoom: lenient_cover_zoom(raw.cover_zoom),
        taskbar_lyrics: lenient_taskbar_lyrics(raw.taskbar_lyrics),
        language: lenient_language(raw.language),
    };
    // Former max was 64; clamp so older saved values still load.
    if settings.lyrics_preferences.line_gap_px > 50 {
        settings.lyrics_preferences.line_gap_px = 50;
    }
    // The font sizes used to start at 0; they now start at 1.
    settings.lyrics_preferences.primary_font_size_px = settings.lyrics_preferences.primary_font_size_px.max(1);
    settings.lyrics_preferences.auxiliary_font_size_px = settings.lyrics_preferences.auxiliary_font_size_px.max(1);
    settings.validate()?;
    Ok((
        settings,
        version != SETTINGS_SCHEMA_VERSION || legacy_clarity_migrated,
    ))
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
    fn legacy_main_background_zoom_is_ignored_and_not_serialized() {
        let settings: ThemeSettings = serde_json::from_value(serde_json::json!({
            "backgroundHex": "#000000",
            "accentHex": "#55D9FF",
            "mainBackgroundZoomPercent": 8
        }))
        .expect("load settings written by the version with background zoom");
        let saved = serde_json::to_value(settings).expect("serialize current theme settings");
        assert!(saved.get("mainBackgroundZoomPercent").is_none());
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
    fn dsd_settings_default_normalise_and_roundtrip() {
        let defaults = DsdPlaybackSettings::default();
        assert_eq!((defaults.gain_db, defaults.output_rate_hz), (3.0, 176_400));
        let odd = DsdPlaybackSettings { gain_db: 40.0, output_rate_hz: 192_000, dop: false }.normalized();
        assert_eq!((odd.gain_db, odd.output_rate_hz), (6.0, 176_400));
        assert_eq!(DsdPlaybackSettings { gain_db: -0.2, output_rate_hz: 352_800, dop: false }.normalized().gain_db, 0.0);
        assert_eq!(DsdPlaybackSettings { gain_db: f32::NAN, output_rate_hz: 352_800, dop: false }.normalized().gain_db, 3.0);

        let directory = test_directory("dsd-settings");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        assert_eq!(store.snapshot().unwrap().dsd, defaults);
        store
            .update(|settings| {
                settings.dsd = DsdPlaybackSettings { gain_db: 5.0, output_rate_hz: 352_800, dop: false };
                Ok(())
            })
            .expect("persist DSD settings");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert_eq!(reopened.snapshot().unwrap().dsd, DsdPlaybackSettings { gain_db: 5.0, output_rate_hz: 352_800, dop: false });
        let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["dsd"]["gainDb"], 5.0);
        assert_eq!(persisted["dsd"]["outputRateHz"], 352_800);
        assert_eq!(persisted["dsd"]["dop"], false);
    }

    #[test]
    fn the_volume_link_is_off_by_default_and_survives_restart() {
        let directory = test_directory("volume-sync");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        assert!(!store.snapshot().unwrap().output_volume_sync);
        store
            .update(|settings| {
                settings.output_volume_sync = true;
                Ok(())
            })
            .expect("persist");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.snapshot().unwrap().output_volume_sync);
        let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["outputVolumeSync"], true);
    }

    #[test]
    fn dop_is_off_by_default_and_survives_restart() {
        assert!(!DsdPlaybackSettings::default().dop);
        assert!(!lenient_dsd_settings(Some(serde_json::json!({"gainDb": 3}))).dop, "an old file has no dop key");
        assert!(lenient_dsd_settings(Some(serde_json::json!({"dop": true}))).dop);
        assert!(!lenient_dsd_settings(Some(serde_json::json!({"dop": "yes"}))).dop, "junk is off");
        let directory = test_directory("dsd-dop");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        store
            .update(|settings| {
                settings.dsd.dop = true;
                Ok(())
            })
            .expect("persist");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.snapshot().unwrap().dsd.dop);
    }

    #[test]
    fn only_a_shrunk_cover_is_remembered() {
        assert_eq!(CoverZoomSettings::default().scale, 1.0);
        assert_eq!(CoverZoomSettings { scale: 0.6 }.normalized().scale, 0.6);
        assert_eq!(CoverZoomSettings { scale: 0.1 }.normalized().scale, 0.5, "the minimum is 0.5");
        assert_eq!(CoverZoomSettings { scale: 2.5 }.normalized().scale, 1.0, "an enlarged cover is not remembered");
        assert_eq!(CoverZoomSettings { scale: f32::NAN }.normalized().scale, 1.0);

        let directory = test_directory("cover-zoom");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        assert_eq!(store.snapshot().unwrap().cover_zoom, CoverZoomSettings::default());
        store
            .update(|settings| {
                settings.cover_zoom = CoverZoomSettings { scale: 0.55 };
                Ok(())
            })
            .expect("persist cover zoom");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert_eq!(reopened.snapshot().unwrap().cover_zoom, CoverZoomSettings { scale: 0.55 });
        // A value written by the earlier build (with a pan) is read as its scale, repaired.
        assert_eq!(
            lenient_cover_zoom(Some(serde_json::json!({"scale": 2.0, "offsetX": 0.3, "offsetY": 0.1}))).scale,
            1.0
        );
        assert_eq!(lenient_cover_zoom(Some(serde_json::json!({"scale": 0.7, "offsetX": 0.3}))).scale, 0.7);
        assert_eq!(lenient_cover_zoom(Some(serde_json::json!({"scale": "big"}))), CoverZoomSettings::default());
        assert_eq!(lenient_cover_zoom(None), CoverZoomSettings::default());
    }

    #[test]
    fn output_devices_default_to_none_roundtrip_and_clamp() {
        let directory = test_directory("output-devices");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert!(store.snapshot().unwrap().output_devices.is_empty());
        let chosen = vec![
            OutputDeviceEntry { id: None, name: String::new(), enabled: true, delay_ms: 0, channel_map: None },
            OutputDeviceEntry {
                id: Some("wasapi:{abc}".to_owned()),
                name: "Speakers".to_owned(),
                enabled: false,
                delay_ms: 250,
                channel_map: Some(vec![
                    ChannelRouteEntry { source: Some(1), gain_percent: 100 },
                    ChannelRouteEntry { source: None, gain_percent: 40 },
                ]),
            },
        ];
        store
            .update(|settings| {
                settings.output_devices = chosen.clone();
                Ok(())
            })
            .unwrap();
        let reopened = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().output_devices, chosen);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["outputDevices"][1]["delayMs"], 250);
        assert_eq!(persisted["outputDevices"][1]["name"], "Speakers");
        assert_eq!(persisted["outputDevices"][1]["channelMap"][0]["source"], 1);
        assert!(persisted["outputDevices"][0]["id"].is_null());
    }

    #[test]
    fn the_exclusive_output_flag_defaults_off_and_survives_restart() {
        let directory = test_directory("output-exclusive");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert!(!store.snapshot().unwrap().output_exclusive);
        store
            .update(|settings| {
                settings.output_exclusive = true;
                Ok(())
            })
            .unwrap();
        let reopened = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert!(reopened.snapshot().unwrap().output_exclusive);
        let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["outputExclusive"], true);
    }

    #[test]
    fn output_presets_are_five_slots_that_survive_restart_and_repair_themselves() {
        let directory = test_directory("output-presets");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert_eq!(store.snapshot().unwrap().output_presets, vec![None; 5]);
        let saved = vec![OutputDeviceEntry {
            id: Some("wasapi:{a}".to_owned()),
            name: "A".to_owned(),
            enabled: true,
            delay_ms: 120,
            channel_map: None,
        }];
        store
            .update(|settings| {
                settings.output_presets = vec![None, Some(saved.clone())];
                Ok(())
            })
            .unwrap();
        let reopened = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap().output_presets,
            vec![None, Some(saved.clone()), None, None, None]
        );
        let parsed = lenient_output_presets(Some(serde_json::json!([[], "x", [{"id": "b"}], [{"id": "c"}]])));
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[0], None, "an empty slot stays empty");
        assert_eq!(parsed[1], None, "junk is an empty slot");
        assert_eq!(parsed[2].as_ref().map(|slot| slot[0].id.clone()), Some(Some("b".to_owned())));
        assert_eq!(lenient_output_presets(None), vec![None; 5]);
    }

    #[test]
    fn malformed_output_devices_fall_back_entry_by_entry() {
        let parsed = lenient_output_devices(Some(serde_json::json!([
            {"id": "a", "volumePercent": 500, "delayMs": 99999, "channelMap": [{"source": 300, "gainPercent": 250}]},
            "junk",
            {"id": "a", "enabled": false},
            {"enabled": "yes"},
        ])));
        assert_eq!(parsed.len(), 2, "the repeated device and the junk entry are dropped: {parsed:?}");
        assert_eq!(parsed[0].delay_ms, 2000);
        assert_eq!(parsed[0].channel_map, Some(vec![ChannelRouteEntry { source: None, gain_percent: 100 }]));
        assert_eq!(parsed[1], OutputDeviceEntry { id: None, name: String::new(), enabled: true, delay_ms: 0, channel_map: None });
        assert!(lenient_output_devices(Some(serde_json::json!(7))).is_empty());
        assert!(lenient_output_devices(None).is_empty());
    }

    #[test]
    fn malformed_dsd_settings_fall_back_field_by_field() {
        let parsed = lenient_dsd_settings(Some(serde_json::json!({"gainDb": "loud", "outputRateHz": 352_800})));
        assert_eq!((parsed.gain_db, parsed.output_rate_hz), (3.0, 352_800));
        let parsed = lenient_dsd_settings(Some(serde_json::json!({"gainDb": 99, "outputRateHz": 44_100})));
        assert_eq!((parsed.gain_db, parsed.output_rate_hz), (6.0, 176_400));
        assert_eq!(lenient_dsd_settings(Some(serde_json::json!(7))), DsdPlaybackSettings::default());
        assert_eq!(lenient_dsd_settings(None), DsdPlaybackSettings::default());
    }

    #[test]
    fn the_interface_language_defaults_to_system_and_survives_restart() {
        assert_eq!(UiLanguage::default(), UiLanguage::System);
        assert_eq!(lenient_language(Some(serde_json::json!("zh-CN"))), UiLanguage::ZhCn);
        assert_eq!(lenient_language(Some(serde_json::json!("zh-TW"))), UiLanguage::ZhTw);
        assert_eq!(lenient_language(Some(serde_json::json!("ja"))), UiLanguage::Ja);
        assert_eq!(lenient_language(Some(serde_json::json!("en"))), UiLanguage::En);
        assert_eq!(lenient_language(Some(serde_json::json!("fr"))), UiLanguage::System);
        assert_eq!(lenient_language(Some(serde_json::json!(3))), UiLanguage::System);
        assert_eq!(lenient_language(None), UiLanguage::System);

        let directory = test_directory("language");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        store
            .update(|settings| {
                settings.language = UiLanguage::ZhCn;
                Ok(())
            })
            .expect("save");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen");
        assert_eq!(reopened.snapshot().unwrap().language, UiLanguage::ZhCn);
        let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["language"], "zh-CN");
    }

    #[test]
    fn taskbar_lyrics_settings_repair_field_by_field_and_roundtrip() {
        let defaults = TaskbarLyricsSettings::default();
        assert!(!defaults.enabled);
        let parsed = lenient_taskbar_lyrics(Some(serde_json::json!({
            "enabled": true, "area": "right", "align": "sideways", "fontSizePx": 99,
            "useCustomColor": true, "customColor": "#ABCDEF", "showTranslation": "yes"
        })));
        assert!(parsed.enabled);
        assert_eq!(parsed.align, TaskbarLyricsAlign::Left);
        assert_eq!(parsed.font_size_px, 50);
        assert_eq!(parsed.translation_scale_percent, 75, "a missing scale is the default");
        assert_eq!(parsed.custom_color, "#abcdef");
        assert!(!parsed.show_translation);
        let bad = lenient_taskbar_lyrics(Some(serde_json::json!({"fontSizePx": 2, "customColor": "red"})));
        assert_eq!((bad.font_size_px, bad.custom_color.as_str()), (2, "#ffffff"));
        let tiny = lenient_taskbar_lyrics(Some(serde_json::json!({"fontSizePx": 0, "translationScalePercent": 0})));
        assert_eq!((tiny.font_size_px, tiny.translation_scale_percent), (1, 1));
        let big = lenient_taskbar_lyrics(Some(serde_json::json!({"translationScalePercent": 250})));
        assert_eq!(big.translation_scale_percent, 100);
        assert_eq!(lenient_taskbar_lyrics(Some(serde_json::json!(7))), defaults);
        assert_eq!(lenient_taskbar_lyrics(None), defaults);

        let directory = test_directory("taskbar-lyrics");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let chosen = TaskbarLyricsSettings {
            enabled: true,
            align: TaskbarLyricsAlign::Center,
            font_size_px: 20,
            translation_scale_percent: 60,
            use_custom_color: true,
            custom_color: "#112233".into(),
            show_translation: true,
            text_effect: LyricsTextEffect::Stroke,
        };
        store
            .update(|settings| {
                settings.taskbar_lyrics = chosen.clone();
                Ok(())
            })
            .expect("save");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen");
        assert_eq!(reopened.snapshot().unwrap().taskbar_lyrics, chosen);
        let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert!(persisted["taskbarLyrics"].get("area").is_none());
        assert_eq!(persisted["taskbarLyrics"]["fontSizePx"], 20);
        assert_eq!(persisted["taskbarLyrics"]["textEffect"], "stroke");
        // An old file has no effect: the text stays as it was. Junk falls back the same way.
        assert_eq!(lenient_taskbar_lyrics(Some(serde_json::json!({"enabled": true}))).text_effect, LyricsTextEffect::None);
        assert_eq!(lenient_taskbar_lyrics(Some(serde_json::json!({"textEffect": "glow"}))).text_effect, LyricsTextEffect::None);
        assert_eq!(lenient_taskbar_lyrics(Some(serde_json::json!({"textEffect": "shadow"}))).text_effect, LyricsTextEffect::Shadow);
    }

    #[test]
    fn ignored_update_version_defaults_to_none_and_survives_restart() {
        let directory = test_directory("ignored-update");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        assert_eq!(store.snapshot().unwrap().ignored_update_version, None);
        store
            .update(|settings| {
                settings.ignored_update_version = Some("26.10.20".into());
                Ok(())
            })
            .expect("persist version");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert_eq!(
            reopened.snapshot().unwrap().ignored_update_version.as_deref(),
            Some("26.10.20")
        );
    }

    #[test]
    fn resume_playback_flag_defaults_off_and_survives_restart() {
        let directory = test_directory("resume-playback");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        assert!(!store.snapshot().unwrap().resume_playback);
        store
            .update(|settings| {
                settings.resume_playback = true;
                Ok(())
            })
            .expect("persist flag");
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.snapshot().unwrap().resume_playback);
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
    fn excluded_folders_upsert_and_remove_are_persisted() {
        let directory = test_directory("excluded-folders");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let excluded = ExcludedFolderEntry {
            id: SourceId::new(),
            display_name: "Podcasts".into(),
            path: StoredPath::Utf8(r"D:/Music/Podcasts".into()),
        };
        let id = excluded.id;
        store
            .upsert_excluded_folder(excluded.clone())
            .expect("register excluded folder");
        assert_eq!(store.snapshot().unwrap().excluded_folders, vec![excluded]);

        let (after_remove, removed) = store
            .remove_excluded_folder(id)
            .expect("remove excluded folder");
        assert_eq!(removed.id, id);
        assert!(after_remove.excluded_folders.is_empty());
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("reopen settings");
        assert!(reopened.snapshot().unwrap().excluded_folders.is_empty());
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
    fn artist_list_sort_is_persisted_and_legacy_settings_default_to_title_ascending() {
        let settings = AppSettings {
            artist_list_sort: ArtistListSort {
                field: ArtistListSortField::Duration,
                direction: ArtistListSortDirection::Desc,
            },
            ..AppSettings::default()
        };
        let mut json = serde_json::to_value(&settings).expect("serialize settings");
        assert_eq!(json["artistListSort"]["field"], "duration");
        assert_eq!(json["artistListSort"]["direction"], "desc");
        let decoded: AppSettings = serde_json::from_value(json.clone()).expect("restore settings");
        assert_eq!(decoded.artist_list_sort, settings.artist_list_sort);

        json["artistListSort"]["field"] = serde_json::json!("trackDuration");
        let track_duration: AppSettings =
            serde_json::from_value(json.clone()).expect("restore track duration sort");
        assert_eq!(
            track_duration.artist_list_sort.field,
            ArtistListSortField::TrackDuration
        );

        json.as_object_mut()
            .expect("settings object")
            .remove("artistListSort");
        let legacy: AppSettings = serde_json::from_value(json).expect("load old settings");
        assert_eq!(legacy.artist_list_sort, ArtistListSort::default());
    }

    #[test]
    fn album_index_sort_is_persisted_and_legacy_settings_default_to_title_ascending() {
        let settings = AppSettings {
            album_index_sort: AlbumIndexSort {
                field: AlbumIndexSortField::Played,
                direction: ArtistListSortDirection::Desc,
            },
            ..AppSettings::default()
        };
        let mut json = serde_json::to_value(&settings).expect("serialize settings");
        assert_eq!(json["albumIndexSort"]["field"], "played");
        assert_eq!(json["albumIndexSort"]["direction"], "desc");
        let restored: AppSettings = serde_json::from_value(json.clone()).expect("restore sort");
        assert_eq!(restored.album_index_sort, settings.album_index_sort);

        json.as_object_mut()
            .expect("settings object")
            .remove("albumIndexSort");
        let legacy: AppSettings = serde_json::from_value(json).expect("load previous settings");
        assert_eq!(legacy.album_index_sort, AlbumIndexSort::default());
    }

    #[test]
    fn album_detail_sort_defaults_to_track_number_and_persists_the_last_choice() {
        let defaults = AppSettings::default();
        assert_eq!(
            defaults.album_detail_sort,
            AlbumDetailSort {
                field: AlbumDetailSortField::Position,
                direction: LibrarySortDirection::Asc,
            }
        );

        let settings = AppSettings {
            album_detail_sort: AlbumDetailSort {
                field: AlbumDetailSortField::Duration,
                direction: LibrarySortDirection::Desc,
            },
            ..defaults
        };
        let mut json = serde_json::to_value(&settings).expect("serialize album detail sort");
        assert_eq!(json["albumDetailSort"]["field"], "duration");
        assert_eq!(json["albumDetailSort"]["direction"], "desc");
        let restored: AppSettings =
            serde_json::from_value(json.clone()).expect("restore album detail sort");
        assert_eq!(restored.album_detail_sort, settings.album_detail_sort);

        json.as_object_mut()
            .expect("settings object")
            .remove("albumDetailSort");
        let legacy: AppSettings = serde_json::from_value(json).expect("load previous settings");
        assert_eq!(legacy.album_detail_sort, AlbumDetailSort::default());
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
                TrackListColumnId::CreatedAt,
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
        assert_eq!(value["trackListColumns"]["columns"][4]["id"], "createdAt");
        assert_eq!(value["trackListColumns"]["columns"][5]["id"], "audioFormat");
        assert_eq!(value["trackListColumns"]["columns"][5]["visible"], true);
        assert_eq!(value["trackListColumns"]["rowHeightPx"], 50);
        assert_eq!(value["trackListColumns"]["groupingEnabled"], true);
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
                simplified_to_traditional: false,
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
        assert_eq!(value["simplifiedToTraditional"], false);
        assert!(value.get("show_translation").is_none());
        let mut without_conversion = value.clone();
        without_conversion
            .as_object_mut()
            .unwrap()
            .remove("simplifiedToTraditional");
        assert!(
            !serde_json::from_value::<LyricsPreferences>(without_conversion)
                .unwrap()
                .simplified_to_traditional
        );
        assert_eq!(
            serde_json::from_value::<LyricsPreferences>(value).unwrap(),
            preferences
        );
    }

    #[test]
    fn lyrics_preference_numeric_ranges_are_inclusive_and_reject_out_of_range_values() {
        let mut minimum = LyricsPreferences {
            inactive_opacity_percent: 10,
            primary_font_size_px: 1,
            auxiliary_font_size_px: 1,
            ..LyricsPreferences::default()
        };
        minimum.validate().expect("accept lower bounds");
        minimum.primary_font_size_px = 0;
        assert!(minimum.validate().is_err(), "the font sizes start at 1");
        minimum.primary_font_size_px = 1;
        minimum.auxiliary_font_size_px = 0;
        assert!(minimum.validate().is_err(), "the font sizes start at 1");
        minimum.auxiliary_font_size_px = 1;

        let maximum = LyricsPreferences {
            inactive_opacity_percent: 100,
            primary_font_size_px: 50,
            auxiliary_font_size_px: 50,
            ..LyricsPreferences::default()
        };
        maximum.validate().expect("accept upper bounds");

        let mut line_gap = LyricsPreferences {
            line_gap_px: 0,
            ..LyricsPreferences::default()
        };
        line_gap.validate().expect("accept zero line gap");
        line_gap.line_gap_px = 50;
        line_gap.validate().expect("accept maximum line gap");
        line_gap.line_gap_px = 51;
        assert!(line_gap.validate().is_err());

        minimum.inactive_opacity_percent = 9;
        assert!(minimum.validate().is_err());
        minimum.inactive_opacity_percent = 101;
        assert!(minimum.validate().is_err());

        minimum.inactive_opacity_percent = 70;
        minimum.primary_font_size_px = 51;
        assert!(minimum.validate().is_err());

        minimum.primary_font_size_px = 14;
        minimum.auxiliary_font_size_px = 51;
        assert!(minimum.validate().is_err());
    }

    #[test]
    fn lyrics_font_sizes_saved_as_zero_load_as_one() {
        let directory = test_directory("lyrics-font-zero");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        drop(store);
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        value["lyricsPreferences"]["primaryFontSizePx"] = serde_json::json!(0);
        value["lyricsPreferences"]["auxiliaryFontSizePx"] = serde_json::json!(0);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let reopened = SettingsStore::open(&path, AppSettings::default()).expect("old file still loads");
        let preferences = reopened.snapshot().unwrap().lyrics_preferences;
        assert_eq!((preferences.primary_font_size_px, preferences.auxiliary_font_size_px), (1, 1));
    }

    #[test]
    fn invalid_lyrics_preference_update_does_not_change_memory_or_json() {
        let directory = test_directory("invalid-lyrics-preferences");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("create settings");
        let original = fs::read(&path).expect("read original settings");

        let result = store.update(|settings| {
            settings.lyrics_preferences = LyricsPreferences {
                auxiliary_font_size_px: 51,
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
            settings.lyrics_preferences.line_gap_px = 51;
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(store.snapshot().unwrap().lyrics_preferences.line_gap_px, 24);
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn legacy_line_gap_above_fifty_clamps_on_load() {
        let directory = test_directory("legacy-lyrics-line-gap-clamp");
        let path = directory.join("settings.json");
        let mut value = serde_json::to_value(AppSettings::default()).expect("serialize defaults");
        value["lyricsPreferences"]["lineGapPx"] = serde_json::json!(64);
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap())
            .expect("write legacy line gap");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("open clamped");
        assert_eq!(store.snapshot().unwrap().lyrics_preferences.line_gap_px, 50);
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
                quick_settings_opacity_percent: 70,
                ..ThemeSettings::default()
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
                "backgroundOpacityPercent": 40,
                "coverCornerStyle": "rounded"
            },
            "sourceRegistryAuthoritative": true,
            "sources": []
        });
        fs::write(&path, serde_json::to_vec_pretty(&legacy).unwrap()).expect("write legacy");
        let store = SettingsStore::open(&path, AppSettings::default()).expect("open repaired");
        let columns = store.snapshot().unwrap().track_list_columns.columns;
        assert_eq!(
            store.snapshot().unwrap().track_list_columns.row_height_px,
            50
        );
        assert_eq!(columns.len(), 8);
        assert_eq!(columns[1].id, TrackListColumnId::Artist);
        assert!(!columns[1].visible);
        assert_eq!(columns[4].id, TrackListColumnId::CreatedAt);
        assert!(columns[4].visible);
        assert_eq!(columns[7].id, TrackListColumnId::PlayCount);
        assert!(columns[7].visible);
        TrackListColumnSettings {
            columns: columns.clone(),
            ..TrackListColumnSettings::default()
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
    fn track_row_height_defaults_for_old_settings_and_clamps_to_supported_limits() {
        let old: TrackListColumnSettings = serde_json::from_value(serde_json::json!({
            "columns": TrackListColumnSettings::default().columns,
        }))
        .expect("old track list settings remain readable");
        assert_eq!(old.row_height_px, 50);
        assert!(old.grouping_enabled);
        assert_eq!(old.text_overflow_mode, TrackTextOverflowMode::Middle);

        let end_elision: TrackListColumnSettings = serde_json::from_value(serde_json::json!({
            "columns": TrackListColumnSettings::default().columns,
            "textOverflowMode": "end",
        }))
        .expect("tail elision preference should deserialize");
        assert_eq!(end_elision.text_overflow_mode, TrackTextOverflowMode::End);
        let encoded = serde_json::to_value(end_elision).unwrap();
        assert_eq!(encoded["textOverflowMode"], "end");

        let grouping_disabled: TrackListColumnSettings =
            serde_json::from_value(serde_json::json!({
                "columns": TrackListColumnSettings::default().columns,
                "groupingEnabled": false,
            }))
            .expect("grouping preference should persist");
        assert!(!grouping_disabled.grouping_enabled);
        let encoded = serde_json::to_value(grouping_disabled).unwrap();
        assert_eq!(encoded["groupingEnabled"], false);

        let too_short = TrackListColumnSettings {
            row_height_px: 0,
            ..TrackListColumnSettings::default()
        };
        assert_eq!(repair_track_list_columns(too_short).row_height_px, 30);

        let too_tall = TrackListColumnSettings {
            row_height_px: u8::MAX,
            ..TrackListColumnSettings::default()
        };
        assert_eq!(repair_track_list_columns(too_tall).row_height_px, 60);
    }

    #[test]
    fn album_card_max_width_defaults_for_old_settings_and_clamps() {
        let old: TrackListColumnSettings = serde_json::from_value(serde_json::json!({
            "columns": TrackListColumnSettings::default().columns,
        }))
        .expect("old track list settings remain readable");
        assert_eq!(old.album_card_max_width_px, 180);
        let encoded = serde_json::to_value(&old).unwrap();
        assert_eq!(encoded["albumCardMaxWidthPx"], 180);

        let narrow = TrackListColumnSettings {
            album_card_max_width_px: 10,
            ..TrackListColumnSettings::default()
        };
        assert_eq!(
            repair_track_list_columns(narrow).album_card_max_width_px,
            100
        );
        let wide = TrackListColumnSettings {
            album_card_max_width_px: u16::MAX,
            ..TrackListColumnSettings::default()
        };
        assert_eq!(repair_track_list_columns(wide).album_card_max_width_px, 500);
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
            simplified_to_traditional: true,
        };
        let mut columns = TrackListColumnSettings::default();
        columns.columns.swap(0, 2);
        columns.columns[1].visible = false;
        columns.row_height_px = 57;
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
        assert_eq!(settings.track_list_columns.row_height_px, 57);
        assert_eq!(settings.now_playing_layout, NowPlayingLayout::B);
        assert_eq!(settings.lyrics_preferences, lyrics_preferences);
        assert_eq!(settings.sources, vec![source]);
        assert!(!settings.source_registry_authoritative);
        assert!(!reopened.source_registry_authoritative().unwrap());
    }

    #[test]
    fn schema_six_migrates_to_high_quality_resampling_without_losing_other_settings() {
        let directory = test_directory("resampling-migration");
        let path = directory.join("settings.json");
        let mut previous = AppSettings {
            schema_version: 6,
            sources: vec![source(StoredPath::Utf8("D:/Music".into()))],
            shuffle: true,
            repeat_mode: RepeatMode::One,
            now_playing_layout: NowPlayingLayout::B,
            ..AppSettings::default()
        };
        previous.theme.accent_hex = "#ABCDEF".into();
        previous
            .now_playing_appearance_preferences
            .background_blur_px = 8;
        let mut json = serde_json::to_value(previous.clone()).unwrap();
        json.as_object_mut().unwrap().remove("resamplingMode");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).expect("write schema six settings");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("migrate settings");
        let migrated = store.snapshot().unwrap();
        assert_eq!(migrated.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.resampling_mode, ResamplingMode::HighQuality);
        assert_eq!(migrated.theme, previous.theme);
        assert_eq!(
            migrated.now_playing_appearance_preferences,
            previous.now_playing_appearance_preferences
        );
        assert_eq!(migrated.sources, previous.sources);
        assert!(migrated.shuffle);
        assert_eq!(migrated.repeat_mode, RepeatMode::One);
        assert_eq!(migrated.now_playing_layout, NowPlayingLayout::B);

        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
        assert_eq!(persisted["resamplingMode"], "highQuality");
        assert_eq!(persisted["dseeHx"], false);
    }

    #[test]
    fn unknown_resampling_mode_falls_back_to_high_quality() {
        for stored in [
            serde_json::json!("highestQuality"),
            serde_json::json!(3),
            serde_json::json!(null),
            serde_json::json!({ "mode": "sinc" }),
        ] {
            let directory = test_directory("resampling-unknown");
            let path = directory.join("settings.json");
            let mut json = serde_json::to_value(AppSettings {
                shuffle: true,
                ..AppSettings::default()
            })
            .unwrap();
            json["resamplingMode"] = stored.clone();
            fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

            let store = SettingsStore::open(&path, AppSettings::default())
                .unwrap_or_else(|error| panic!("{stored}: {error:?}"));
            let settings = store.snapshot().unwrap();
            assert_eq!(
                settings.resampling_mode,
                ResamplingMode::HighQuality,
                "{stored}"
            );
            assert!(settings.shuffle, "other settings survive {stored}");
        }
    }

    #[test]
    fn resampling_mode_roundtrips_through_the_store() {
        let directory = test_directory("resampling-roundtrip");
        let path = directory.join("settings.json");
        let store = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert_eq!(
            store.snapshot().unwrap().resampling_mode,
            ResamplingMode::HighQuality
        );
        store
            .update(|settings| {
                settings.resampling_mode = ResamplingMode::WindowsBuiltin;
                Ok(())
            })
            .unwrap();
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted["resamplingMode"], "windowsBuiltin");
        drop(store);

        let reopened = SettingsStore::open(&path, AppSettings::default()).unwrap();
        assert_eq!(
            reopened.snapshot().unwrap().resampling_mode,
            ResamplingMode::WindowsBuiltin
        );
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
            persisted["nowPlayingAppearancePreferences"]["backgroundOpacityPercent"],
            40
        );
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["coverCornerStyle"],
            "rounded"
        );
        assert_eq!(
            persisted["nowPlayingAppearancePreferences"]["timelineStyle"],
            "edge"
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
            background_opacity_percent: 100,
            cover_corner_style: CoverCornerStyle::Square,
            timeline_style: TimelineStyle::Edge,
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
                background_blur_px: 51,
                ..preferences
            },
            NowPlayingAppearancePreferences {
                background_opacity_percent: 101,
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
            background_opacity_percent: 67,
            cover_corner_style: CoverCornerStyle::Square,
            timeline_style: TimelineStyle::Edge,
        };
        let value = serde_json::to_value(preferences).unwrap();
        assert_eq!(value["backgroundBlurPx"], 12);
        assert_eq!(value["backgroundOpacityPercent"], 67);
        assert_eq!(value["coverCornerStyle"], "square");
        assert_eq!(value["timelineStyle"], "edge");
        assert_eq!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(value).unwrap(),
            preferences
        );
        assert!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": -1,
                "backgroundOpacityPercent": 50
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
            "backgroundOpacityPercent": 55
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
                "backgroundOpacityPercent": 50,
                "coverCornerStyle": "circle"
            }))
            .is_err()
        );
    }

    #[test]
    fn missing_timeline_style_defaults_to_edge_and_rejects_unknown_values() {
        let directory = test_directory("appearance-timeline-style");
        let path = directory.join("settings.json");
        let mut json = serde_json::to_value(AppSettings::default()).unwrap();
        json["schemaVersion"] = serde_json::json!(5);
        json["nowPlayingAppearancePreferences"] = serde_json::json!({
            "backgroundBlurPx": 15,
            "backgroundOpacityPercent": 55,
            "coverCornerStyle": "square"
        });
        fs::write(&path, serde_json::to_vec(&json).unwrap())
            .expect("write settings without timeline");

        let store = SettingsStore::open(&path, AppSettings::default()).expect("open settings");
        let snapshot = store.snapshot().unwrap();
        assert_eq!(
            snapshot.now_playing_appearance_preferences.timeline_style,
            TimelineStyle::Edge
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
                "backgroundOpacityPercent": 50,
                "timelineStyle": "minimal"
            }))
            .unwrap()
            .timeline_style,
            TimelineStyle::Edge
        );
        assert_eq!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": 12,
                "backgroundOpacityPercent": 50,
                "timelineStyle": "line"
            }))
            .unwrap()
            .timeline_style,
            TimelineStyle::Edge
        );

        assert!(
            serde_json::from_value::<NowPlayingAppearancePreferences>(serde_json::json!({
                "backgroundBlurPx": 12,
                "backgroundOpacityPercent": 50,
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
                    .background_opacity_percent,
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
                persisted["nowPlayingAppearancePreferences"]["backgroundOpacityPercent"],
                40
            );
            assert!(
                persisted["nowPlayingAppearancePreferences"]["surfaceTransparencyPercent"]
                    .is_null()
            );
        }
    }

    #[test]
    fn appearance_reads_legacy_background_brightness_as_opacity() {
        let preferences: NowPlayingAppearancePreferences =
            serde_json::from_value(serde_json::json!({
                "backgroundBlurPx": 18,
                "backgroundBrightnessPercent": 58
            }))
            .expect("deserialize legacy brightness key");
        assert_eq!(preferences.background_opacity_percent, 58);

        let written = serde_json::to_value(preferences).expect("serialize appearance");
        assert_eq!(written["backgroundOpacityPercent"], 58);
        assert!(written.get("backgroundBrightnessPercent").is_none());
    }

    #[test]
    fn theme_reads_legacy_main_background_brightness_as_opacity() {
        let theme: ThemeSettings = serde_json::from_value(serde_json::json!({
            "backgroundHex": "#000000",
            "accentHex": "#55D9FF",
            "mainBackgroundBrightnessPercent": 62
        }))
        .expect("deserialize legacy brightness key");
        assert_eq!(theme.main_background_opacity_percent, 62);

        let written = serde_json::to_value(&theme).expect("serialize theme");
        assert_eq!(written["mainBackgroundOpacityPercent"], 62);
        assert!(written.get("mainBackgroundBrightnessPercent").is_none());
    }

    #[test]
    fn theme_color_mode_defaults_for_old_settings_and_round_trips_new_selection() {
        let old: ThemeSettings = serde_json::from_value(serde_json::json!({
            "backgroundHex": "#000000",
            "accentHex": "#55D9FF"
        }))
        .expect("deserialize old theme settings");
        assert_eq!(old.color_mode, ThemeColorMode::AutoTextColor);
        assert_eq!(
            old.background_opacity_text_color,
            BackgroundOpacityTextColor::White
        );
        assert_eq!(old.main_background_clarity_percent, 0);
        assert_eq!(old.cover_background_clarity_percent, 0);
        assert_eq!(old.border_opacity_percent, 100);
        assert_eq!(old.divider_opacity_percent, 100);

        let selected = ThemeSettings {
            border_opacity_percent: 63,
            divider_opacity_percent: 42,
            color_mode: ThemeColorMode::AutoBackgroundOpacity,
            background_opacity_text_color: BackgroundOpacityTextColor::Black,
            main_background_clarity_percent: 35,
            cover_background_clarity_percent: 72,
            ..ThemeSettings::default()
        };
        let json = serde_json::to_value(&selected).expect("serialize selected theme");
        let restored: ThemeSettings = serde_json::from_value(json).expect("restore selected theme");
        assert_eq!(restored.color_mode, ThemeColorMode::AutoBackgroundOpacity);
        assert_eq!(
            restored.background_opacity_text_color,
            BackgroundOpacityTextColor::Black
        );
        assert_eq!(restored.main_background_clarity_percent, 35);
        assert_eq!(restored.cover_background_clarity_percent, 72);
        assert_eq!(restored.border_opacity_percent, 63);
        assert_eq!(restored.divider_opacity_percent, 42);
    }

    #[test]
    fn legacy_shared_background_clarity_migrates_to_both_independent_settings() {
        let directory = test_directory("legacy-shared-background-clarity");
        let path = directory.join("settings.json");
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schemaVersion": SETTINGS_SCHEMA_VERSION,
                "theme": {
                    "backgroundHex": "#000000",
                    "accentHex": "#55D9FF",
                    "backgroundClarityPercent": 64
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let (settings, migrated) = read_settings(&path).expect("read legacy clarity settings");
        assert!(migrated);
        assert_eq!(settings.theme.main_background_clarity_percent, 64);
        assert_eq!(settings.theme.cover_background_clarity_percent, 64);

        let _store = SettingsStore::open(&path, AppSettings::default())
            .expect("open and persist migrated clarity settings");
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read migrated settings"))
                .expect("parse migrated settings");
        assert_eq!(persisted["theme"]["mainBackgroundClarityPercent"], 64);
        assert_eq!(persisted["theme"]["coverBackgroundClarityPercent"], 64);
        assert!(persisted["theme"].get("backgroundClarityPercent").is_none());
    }

    #[test]
    fn schema_eight_background_migration_preserves_image_and_defaults_to_solid_without_one() {
        for (label, has_image, expected_mode) in [
            ("with-image", true, MainBackgroundMode::SingleImage),
            ("without-image", false, MainBackgroundMode::Solid),
        ] {
            let directory = test_directory(&format!("background-v8-{label}"));
            let path = directory.join("settings.json");
            let mut json =
                serde_json::to_value(AppSettings::default()).expect("serialize defaults");
            json["schemaVersion"] = serde_json::json!(8);
            let theme = json["theme"].as_object_mut().expect("theme object");
            theme.remove("mainBackgroundMode");
            theme.remove("slideshowFolder");
            theme.remove("slideshowIntervalMinutes");
            theme.remove("slideshowOrder");
            fs::write(
                &path,
                serde_json::to_vec(&json).expect("serialize legacy settings"),
            )
            .expect("write legacy settings");
            if has_image {
                fs::write(directory.join("background"), b"legacy background marker")
                    .expect("write legacy background marker");
            }

            let store =
                SettingsStore::open(&path, AppSettings::default()).expect("open v8 settings");
            let migrated = store.snapshot().expect("snapshot migrated settings");
            assert_eq!(migrated.theme.main_background_mode, expected_mode);
            assert_eq!(migrated.theme.slideshow_interval_minutes, 10);
            assert_eq!(migrated.theme.slideshow_order, SlideshowOrder::Sequential);
            let persisted: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).expect("read migrated settings"))
                    .expect("parse migrated settings");
            assert_eq!(persisted["schemaVersion"], SETTINGS_SCHEMA_VERSION);
            assert_eq!(
                persisted["theme"]["mainBackgroundMode"],
                match expected_mode {
                    MainBackgroundMode::SingleImage => "singleImage",
                    MainBackgroundMode::Solid => "solid",
                    _ => unreachable!(),
                }
            );
            fs::remove_dir_all(directory).expect("remove test directory");
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

    #[test]
    fn minimized_window_geometry_is_not_a_valid_restore_snapshot() {
        let minimized = WindowGeometry {
            x: -32_000,
            y: -32_000,
            width: 359,
            height: 42,
            maximized: false,
        };
        assert!(!minimized.is_valid_restore_snapshot());

        let valid_negative_monitor_position = WindowGeometry {
            x: -1_920,
            y: 0,
            width: 1_360,
            height: 900,
            maximized: false,
        };
        assert!(valid_negative_monitor_position.is_valid_restore_snapshot());

        let too_small = WindowGeometry {
            width: WindowGeometry::MIN_WIDTH,
            height: WindowGeometry::MIN_HEIGHT - 1,
            ..valid_negative_monitor_position
        };
        assert!(!too_small.is_valid_restore_snapshot());
    }
}
