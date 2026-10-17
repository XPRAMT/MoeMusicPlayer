import { invoke, isTauri } from '@tauri-apps/api/core';
import { tr } from './i18n.js';

export type FeatureState = 'ready' | 'notReady' | 'unavailable';

export interface FeatureCapability {
  state: FeatureState;
  detail: string | null;
}

export interface RuntimeCapabilities {
  platform: 'windows' | 'android' | 'unsupported';
  desktopRuntime: FeatureCapability;
  library: FeatureCapability;
  sourceSync: FeatureCapability;
  playback: FeatureCapability;
  playbackNavigation: FeatureCapability;
  playbackModes: FeatureCapability;
  playlistExchange: FeatureCapability;
  systemMediaControls: FeatureCapability;
}

export interface OpenSourceCredit {
  name: string;
  version: string;
  license: string;
  repositoryUrl: string;
}

export interface AboutInfo {
  name: string;
  author: string;
  authorUrl: string;
  repositoryUrl: string;
  buildVersion: string;
  buildTimestampUtc: string;
  gitCommit: string;
  architecture: string;
  credits: OpenSourceCredit[];
}

export interface UpdateRelease {
  buildId: string;
  version: string;
  publishedAt: string;
  notes: string;
  downloadBytes: number;
}

export interface UpdateState {
  status: 'idle' | 'checking' | 'current' | 'available' | 'downloading' | 'ready' | 'deferred' | 'restarting' | 'installPermission' | 'installing' | 'unpublished' | 'unsupported' | 'error';
  currentBuildId: string;
  latest: UpdateRelease | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
}

export type UpdateInstallMode = 'restartNow' | 'nextLaunch';

export interface ThemePreferences {
  backgroundHex: string;
  accentHex: string;
  /** Opacity multiplier for shared control and surface borders (0–100). */
  borderOpacityPercent: number;
  /** Opacity multiplier for shared content separators (0–100). */
  dividerOpacityPercent: number;
  colorMode: 'autoTextColor' | 'autoBackgroundOpacity';
  backgroundOpacityTextColor: 'white' | 'black';
  /** Main-window background target clarity (0–100), used by autoBackgroundOpacity. */
  mainBackgroundClarityPercent: number;
  /** Now Playing cover-background target clarity (0–100), used by autoBackgroundOpacity. */
  coverBackgroundClarityPercent: number;
  quickSettingsOpacityPercent: number;
  /** Backdrop blur for the Now Playing quick-settings drawer (0–50 px). */
  quickSettingsBlurPx: number;
  mainBackgroundBlurPx: number;
  /** Image opacity percent (0–100); UI label is 背景透明度, same convention as quickSettingsOpacityPercent. */
  mainBackgroundOpacityPercent: number;
  mainBackgroundMode: 'solid' | 'singleImage' | 'slideshow' | 'albumCover' | 'systemWallpaper';
  slideshowFolder: string | null;
  slideshowIntervalMinutes: number;
  slideshowOrder: 'sequential' | 'random';
  /**
   * Relative-luminance Y threshold (0–100). At/above → black text; below → white text.
   * UI label is 自動切換文字顏色閥值.
   */
  autoTextColorThresholdPercent: number;
}

export interface BackdropOpacityCalculation {
  kind: 'ratio' | 'inverseRatio' | 'noAdjustment';
  textColor: 'white' | 'black';
  targetPercent: number;
  imagePercent: number;
  opacityPercent: number;
  displayText: string;
}

export type TrackListColumnId = 'title' | 'artist' | 'album' | 'year' | 'createdAt' | 'audioFormat' | 'duration' | 'playCount';

export interface TrackListColumnPreference {
  id: TrackListColumnId;
  visible: boolean;
}

export interface TrackColumnWidths {
  title: number;
  artist: number;
  album: number;
}

export type LibrarySortDirection = 'asc' | 'desc';

export interface LibraryListSort {
  field: TrackListColumnId;
  direction: LibrarySortDirection;
}

export interface TrackListColumnSettings {
  columns: TrackListColumnPreference[];
  widths?: TrackColumnWidths;
  librarySort?: LibraryListSort;
  rowHeightPx?: number;
  albumCardMaxWidthPx?: number;
  groupingEnabled: boolean;
  textOverflowMode: 'middle' | 'end';
}

export type NowPlayingLayout = 'a' | 'b';
/** DSD→PCM playback settings: digital gain (1 to 6 dB) and the PCM rate DSD is converted to. */
export interface DsdPlaybackSettings {
  gainDb: number;
  outputRateHz: number;
}

/** Interface language choice (ADR 0019). */
export type LanguageChoice = 'system' | 'zh-TW' | 'zh-CN' | 'ja' | 'en';

/** Lyrics drawn on the free part of the Windows taskbar (ADR 0018). */
export interface TaskbarLyricsSettings {
  enabled: boolean;
  align: 'left' | 'center' | 'right';
  fontSizePx: number;
  /** The translation line as a percentage of the main line (1 to 100). */
  translationScalePercent: number;
  useCustomColor: boolean;
  /** `#rrggbb` */
  customColor: string;
  showTranslation: boolean;
}

/** Now Playing cover size remembered across restarts (shrunk only). */
export interface CoverZoomSettings {
  /** 0.4 to 1; only a shrunk cover is remembered (an enlarged one is stored as 1). */
  scale: number;
}

export type ResamplingMode = 'windowsBuiltin' | 'highQuality';
export type CoverCornerStyle = 'rounded' | 'square';
export type TimelineStyle = 'edge';
export type LyricsTextEffect = 'shadow' | 'stroke' | 'none';

export interface LibrarySource {
  id: string;
  kind: string;
  displayName: string;
  location: string;
  enabled: boolean;
  syncState: string | null;
  lastAttemptUtcMs: number | null;
  lastSuccessUtcMs: number | null;
  errorCount: number;
  lastError: string | null;
}

export interface ExcludedFolder {
  id: string;
  displayName: string;
  location: string;
}

export interface SourceSyncResult {
  sourceId: string;
  displayName: string;
  state: 'complete' | 'incomplete' | 'unavailable' | 'permissionRevoked' | 'unknown';
  observed: number;
  metadataReads: number;
  unchanged: number;
  addedOrUpdated: number;
  removedMappings: number;
  errorCount: number;
  errors: SourceSyncError[];
  errorsTruncated: boolean;
}

export interface SourceSyncError {
  item: string | null;
  message: string;
}

export interface LibrarySyncResult {
  sources: SourceSyncResult[];
}

export type LibrarySyncProgressStage = 'enumerating' | 'metadata' | 'persisting' | 'finished';
export type LibrarySyncProgressUnit = 'filesystemEntries' | 'tracks';
export type LibrarySyncProgressOutcome =
  | 'complete'
  | 'incomplete'
  | 'unavailable'
  | 'permissionRevoked'
  | 'cancelled'
  | 'failed';

export interface LibrarySyncProgressEvent {
  runId: string;
  sourceIndex: number;
  sourceCount: number;
  displayName: string;
  sourceId: string;
  stage: LibrarySyncProgressStage;
  processed: number;
  total: number | null;
  unit: LibrarySyncProgressUnit;
  observed: number;
  metadataReads: number;
  unchanged: number;
  errorCount: number;
  outcome: LibrarySyncProgressOutcome | null;
}

export interface LibrarySyncFinishedEvent {
  runId: string;
  sourceCount: number;
  sources: SourceSyncResult[];
  error: string | null;
}

export interface TrackSummary {
  id: string;
  title: string | null;
  artist: string | null;
  album: string | null;
  albumArtist: string | null;
  trackNumber: number | null;
  discNumber: number | null;
  durationMs: number | null;
  codec: string | null;
  bitrateBps: number | null;
  sampleRateHz: number | null;
  /** Normalized YYYY, YYYY-MM, or YYYY-MM-DD when known. */
  releaseDate?: string | null;
  year?: number | null;
  /** Filesystem creation time as UTC Unix milliseconds; display is local YYYY-MM-DD. */
  createdAtUtcMs?: number | null;
  bitDepth?: number | null;
  /** Cumulative credited listening time in milliseconds. */
  playedMs?: number | null;
  /** File stem used when `title` is missing. Never a directory or URI query. */
  fileName?: string | null;
}

export interface TrackPage {
  items: TrackSummary[];
  offset: number;
  limit: number;
  totalCount: number;
}

export interface TrackPageRequest {
  query: string | null;
  queryAlternates?: string[];
  fieldFilter?: TrackFieldFilter | null;
  sort?: LibraryListSort | null;
  offset: number;
  limit: number;
}

export interface AlbumDiscCount {
  discNumber: number;
  count: number;
}

export interface AlbumDetailFacets {
  discCounts: AlbumDiscCount[];
  trackCount: number;
  unknownDiscTrackCount: number;
  quickIndexAscending: QuickIndexSnapshot;
  quickIndexDescending: QuickIndexSnapshot;
}

export interface ArtistNameCount {
  name: string;
  count: number;
}

export interface ArtistIndexEntry {
  key: string;
  names: ArtistNameCount[];
  artistNames: string[];
  displayName: string;
  trackCount: number;
  albumCount: number;
  /** Actual cumulative listening time credited to this artist. */
  playedMs: number;
  /** Sum of the durations of this artist's distinct visible tracks. */
  durationMs: number;
}

export interface AlbumIndexEntry {
  key: string;
  displayName: string;
  aliases: string[];
  trackCount: number;
  durationMs: number;
  playedMs: number;
  unmarked: boolean;
}

export interface IndexSnapshotInfo {
  snapshotId: string;
  quickIndex: QuickIndexSnapshot;
}

export interface QuickIndexSnapshot {
  labels: string[];
  groups: Array<{ label: string; beforeIndex: number }>;
  totalCount: number;
}

export interface IndexPage<T> {
  items: T[];
  offset: number;
  limit: number;
  totalCount: number;
}

export interface AlbumArtworkSelection {
  trackCount: number;
  trackIds: string[];
  representative: boolean;
}

export type ArtistListSortField = 'title' | 'trackCount' | 'albumCount' | 'duration' | 'trackDuration';
export type ArtistListSortDirection = 'asc' | 'desc';
export interface ArtistListSort {
  field: ArtistListSortField;
  direction: ArtistListSortDirection;
}

export type AlbumIndexSortField = 'title' | 'duration' | 'played' | 'trackCount';
export interface AlbumIndexSort {
  field: AlbumIndexSortField;
  direction: ArtistListSortDirection;
}

export type AlbumDetailSortField = 'position' | TrackListColumnId;
export interface AlbumDetailSort {
  field: AlbumDetailSortField;
  direction: LibrarySortDirection;
}

export interface TrackFieldFilter {
  field: 'title' | 'artist' | 'album';
  value: string;
}

export interface PlaylistSummary {
  id: string;
  name: string;
  entryCount: number;
}

export interface PlaylistEntrySummary {
  position: number;
  trackId: string | null;
  title: string | null;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  hasEnabledMapping: boolean;
  releaseDate?: string | null;
  year?: number | null;
  createdAtUtcMs?: number | null;
  codec?: string | null;
  sampleRateHz?: number | null;
  bitDepth?: number | null;
  bitrateBps?: number | null;
  playedMs?: number | null;
  /** File stem from the playlist locator when the entry has no title. */
  fileName?: string | null;
}

export interface PlaylistPage {
  items: PlaylistEntrySummary[];
  offset: number;
  limit: number;
  totalCount: number;
}

export type PlaylistSortField = 'position' | TrackListColumnPreference['id'];
export interface PlaylistListSort {
  field: PlaylistSortField;
  direction: 'asc' | 'desc';
}

export interface PlaybackQueuePageItem {
  traversalPosition: number;
  entryPosition: number;
  sourcePosition: number | null;
  trackId: string;
  track: TrackSummary | null;
  isCurrent: boolean;
}

export interface PlaybackQueuePage {
  revision: number;
  total: number;
  offset: number;
  cursor: number | null;
  currentEntryPosition: number | null;
  items: PlaybackQueuePageItem[];
}

export interface PlaylistImportResult {
  playlist: PlaylistSummary;
  matchedEntries: number;
}

export interface PlaylistExportResult {
  playlistName: string;
  entryCount: number;
}

export interface FavoritePlaylist {
  sourceId: string;
  displayName: string;
  selected: boolean;
  isMyLove: boolean;
  writable: boolean;
}

export interface FavoritePlaylists {
  playlists: FavoritePlaylist[];
  myLoveSetupRequired: boolean;
}

export type RepeatMode = 'off' | 'one' | 'all';

export type PlaybackQueueSource =
  | { kind: 'library'; query: string | null; queryAlternates?: string[]; fieldFilter?: TrackFieldFilter | null; sort?: LibraryListSort | null }
  | { kind: 'playlist'; playlistId: string; entryPosition: number }
  | { kind: 'queue'; traversalPosition: number };
export type PlaybackState =
  | 'initializing'
  | 'empty'
  | 'loading'
  | 'ready'
  | 'playing'
  | 'paused'
  | 'stopped'
  | 'ended'
  | 'error';

/** Sleep timer owned by the Rust core; `endsAtMs` is a Unix epoch millisecond deadline. */
export interface SleepTimerSnapshot {
  endsAtMs: number | null;
}

export interface PlaybackSnapshot {
  currentTrack: TrackSummary | null;
  state: PlaybackState;
  isPlaying: boolean;
  positionMs: number;
  durationMs: number | null;
  volume: number;
  lastError: string | null;
  repeatMode: RepeatMode;
  shuffle: boolean;
  queueSort?: LibraryListSort | null;
  canNext: boolean;
  canPrevious: boolean;
  /** Windows only: the output's sample-rate path. */
  outputFormat?: PlaybackOutputFormat | null;
}

export interface PlaybackOutputFormat {
  mode: ResamplingMode;
  sourceRateHz: number | null;
  outputRateHz: number;
  deviceRateHz: number;
  conversion: 'none' | 'highQuality' | 'windows' | 'basic';
  fallbackReason: string | null;
  dseeHx: 'off' | 'active' | 'bypassed' | 'unavailable';
  dseeNotice: string | null;
  dseeOutputRateHz: number | null;
}

export interface LyricLine {
  startMs: number | null;
  text: string;
  translation: string | null;
  romanization: string | null;
}

export type LyricsSource = 'local' | 'embedded' | 'netease' | 'qqmusic' | 'manual';

export interface TrackLyrics {
  trackId: string;
  source: LyricsSource;
  title: string;
  artist: string | null;
  album: string | null;
  durationMs: number | null;
  offsetMs: number;
  synced: boolean;
  lines: LyricLine[];
}

export type LyricsProvider = 'netease' | 'qqmusic' | 'local' | 'embedded';
export type LyricsConfidence = 'high' | 'medium' | 'low';

export interface LyricsCandidate {
  id: string;
  provider: LyricsProvider;
  title: string;
  artist: string;
  album: string | null;
  durationMs: number | null;
  score: number;
  confidence: LyricsConfidence;
  reasons: string[];
  previewLines: string[];
  hasSyncedLyrics: boolean;
  hasWordSyncedLyrics: boolean;
  hasTranslation: boolean;
  hasRomanization: boolean;
}

export type LyricsResultStatus = 'ready' | 'empty' | 'cleared' | 'candidates' | 'error';

export type ShortcutAction = 'fullscreen' | 'playPause' | 'seekBack' | 'seekForward' | 'previous' | 'next';
export type ShortcutDevice = 'keyboard' | 'mouse' | 'gamepad';
export interface ShortcutBinding {
  device: ShortcutDevice;
  code: string;
}
export type ShortcutSettings = Record<ShortcutAction, ShortcutBinding[]>;

export interface LyricsTrackResult {
  lyrics: TrackLyrics | null;
  candidates: LyricsCandidate[];
  status: LyricsResultStatus;
  error: string | null;
  hasMore?: boolean;
}

export interface LyricsPreferences {
  showTranslation: boolean;
  showRomanization: boolean;
  inactiveOpacityPercent: number;
  primaryFontSizePx: number;
  auxiliaryFontSizePx: number;
  lineGapPx: number;
  textEffect: LyricsTextEffect;
  simplifiedToTraditional: boolean;
}

export interface NowPlayingAppearancePreferences {
  backgroundBlurPx: number;
  /** Cover backdrop image opacity percent (0–100); UI label is 封面背景透明度. */
  backgroundOpacityPercent: number;
  coverCornerStyle: CoverCornerStyle;
  timelineStyle: TimelineStyle;
}

interface IpcContract {
  library_get_quick_index_for_tracks: {
    args: { query: string | null; queryAlternates: string[]; fieldFilter: TrackFieldFilter | null; sort: LibraryListSort };
    result: { labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number };
  };
  library_get_quick_index_for_artists: {
    args: { artistNames: string[]; sort: LibraryListSort };
    result: { labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number };
  };
  library_get_quick_index_for_album_tracks: {
    args: { albumNames: string[]; sort: LibraryListSort };
    result: { labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number };
  };
  playlist_get_quick_index_groupings: {
    args: { playlistId: string; sort: PlaylistListSort };
    result: { labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number };
  };
  playback_get_queue_quick_index_groupings: {
    args: { field: 'title' | 'artist' | 'album' };
    result: { labels: string[]; groups: { label: string; beforeIndex: number }[]; totalCount: number };
  };
  about_get_info: { args: Record<string, never>; result: AboutInfo };
  windows_register_audio_associations: { args: Record<string, never>; result: void };
  windows_open_default_apps_settings: { args: Record<string, never>; result: void };
  app_open_external_url: { args: { url: string }; result: void };
  update_get_state: { args: Record<string, never>; result: UpdateState };
  update_check: { args: Record<string, never>; result: UpdateState };
  update_ignore: { args: Record<string, never>; result: UpdateState };
  update_download: { args: { mode: UpdateInstallMode }; result: UpdateState };
  settings_get_recovery_warning: {
    args: Record<string, never>;
    result: string | null;
  };
  settings_source_registry_authoritative: {
    args: Record<string, never>;
    result: boolean;
  };
  settings_confirm_source_registry: {
    args: Record<string, never>;
    result: boolean;
  };
  theme_get_preferences: {
    args: Record<string, never>;
    result: ThemePreferences;
  };
  theme_set_preferences: {
    args: { preferences: ThemePreferences };
    result: ThemePreferences;
  };
  appearance_pick_main_background: {
    args: Record<string, never>;
    result: boolean | null;
  };
  appearance_pick_slideshow_folder: {
    args: Record<string, never>;
    result: boolean | null;
  };
  appearance_clear_main_background: {
    args: Record<string, never>;
    result: void;
  };
  appearance_get_main_background: {
    args: Record<string, never>;
    result: ArrayBuffer;
  };
  appearance_next_slideshow_background: {
    args: Record<string, never>;
    result: ArrayBuffer;
  };
  android_all_files_access_status: {
    args: Record<string, never>;
    result: AndroidAllFilesAccessStatus | null;
  };
  android_request_all_files_access: {
    args: Record<string, never>;
    result: void;
  };
  appearance_set_android_wallpaper_backdrop: {
    args: { enabled: boolean; blurPx: number };
    result: AndroidWallpaperBackdropStatus | null;
  };
  settings_get_track_list_columns: {
    args: Record<string, never>;
    result: TrackListColumnSettings;
  };
  settings_set_track_list_columns: {
    args: { preferences: TrackListColumnSettings };
    result: TrackListColumnSettings;
  };
  settings_get_artist_list_sort: {
    args: Record<string, never>;
    result: ArtistListSort;
  };
  settings_set_artist_list_sort: {
    args: { preference: ArtistListSort };
    result: ArtistListSort;
  };
  settings_get_album_index_sort: {
    args: Record<string, never>;
    result: AlbumIndexSort;
  };
  settings_set_album_index_sort: {
    args: { preference: AlbumIndexSort };
    result: AlbumIndexSort;
  };
  settings_get_album_detail_sort: {
    args: Record<string, never>;
    result: AlbumDetailSort;
  };
  settings_set_album_detail_sort: {
    args: { preference: AlbumDetailSort };
    result: AlbumDetailSort;
  };
  settings_get_now_playing_layout: {
    args: Record<string, never>;
    result: NowPlayingLayout;
  };
  settings_set_now_playing_layout: {
    args: { layout: NowPlayingLayout };
    result: NowPlayingLayout;
  };
  settings_get_resampling_mode: {
    args: Record<string, never>;
    result: ResamplingMode;
  };
  settings_set_resampling_mode: {
    args: { mode: ResamplingMode };
    result: ResamplingMode;
  };
  settings_get_dsee_hx: {
    args: Record<string, never>;
    result: boolean;
  };
  settings_set_dsee_hx: {
    args: { enabled: boolean };
    result: boolean;
  };
  settings_get_language: {
    args: Record<string, never>;
    result: LanguageChoice;
  };
  settings_set_language: {
    args: { language: LanguageChoice };
    result: LanguageChoice;
  };
  settings_get_taskbar_lyrics: {
    args: Record<string, never>;
    result: TaskbarLyricsSettings;
  };
  settings_set_taskbar_lyrics: {
    args: { settings: TaskbarLyricsSettings };
    result: TaskbarLyricsSettings;
  };
  taskbar_lyrics_theme: {
    args: Record<string, never>;
    result: boolean;
  };
  settings_get_dsd: {
    args: Record<string, never>;
    result: DsdPlaybackSettings;
  };
  settings_set_dsd: {
    args: { gainDb: number; outputRateHz: number };
    result: DsdPlaybackSettings;
  };
  settings_get_cover_zoom: {
    args: Record<string, never>;
    result: CoverZoomSettings;
  };
  settings_set_cover_zoom: {
    args: { scale: number };
    result: CoverZoomSettings;
  };
  settings_get_shortcuts: {
    args: Record<string, never>;
    result: ShortcutSettings;
  };
  settings_set_shortcuts: {
    args: { shortcuts: ShortcutSettings };
    result: ShortcutSettings;
  };
  settings_get_lyrics_preferences: {
    args: Record<string, never>;
    result: LyricsPreferences;
  };
  settings_set_lyrics_preferences: {
    args: { preferences: LyricsPreferences };
    result: LyricsPreferences;
  };
  settings_get_now_playing_appearance_preferences: {
    args: Record<string, never>;
    result: NowPlayingAppearancePreferences;
  };
  settings_set_now_playing_appearance_preferences: {
    args: { preferences: NowPlayingAppearancePreferences };
    result: NowPlayingAppearancePreferences;
  };
  get_runtime_capabilities: {
    args: Record<string, never>;
    result: RuntimeCapabilities;
  };
  library_get_page: {
    args: TrackPageRequest;
    result: TrackPage;
  };
  library_create_artist_index_snapshot: {
    args: { sortField: string; direction: 'asc' | 'desc'; groupingEnabled: boolean };
    result: IndexSnapshotInfo;
  };
  library_get_artist_index_snapshot_page: {
    args: { snapshotId: string; offset: number; limit: number };
    result: IndexPage<ArtistIndexEntry>;
  };
  library_resolve_artist_index_entry: {
    args: { identityKey: string };
    result: ArtistIndexEntry | null;
  };
  library_resolve_artist_index_entry_by_name: {
    args: { name: string };
    result: ArtistIndexEntry | null;
  };
  library_create_album_index_snapshot: {
    args: { sortField: string; direction: 'asc' | 'desc'; groupingEnabled: boolean };
    result: IndexSnapshotInfo;
  };
  library_get_album_index_snapshot_page: {
    args: { snapshotId: string; offset: number; limit: number };
    result: IndexPage<AlbumIndexEntry>;
  };
  library_resolve_album_index_entry: {
    args: { identityKey: string };
    result: AlbumIndexEntry | null;
  };
  library_resolve_album_index_entry_by_name: {
    args: { name: string };
    result: AlbumIndexEntry | null;
  };
  library_get_album_page: {
    args: { albumNames: string[]; sort?: LibraryListSort | null; indexSort?: LibrarySortDirection | null; offset: number; limit: number };
    result: TrackPage;
  };
  library_get_album_detail_facets: {
    args: { albumNames: string[] };
    result: AlbumDetailFacets;
  };
  library_get_album_detail_artists: {
    args: { albumNames: string[] };
    result: string[];
  };
  album_artwork_selection_ensure: {
    args: { key: string; albumNames: string[]; trackCount: number; representative: boolean; forceRebuild: boolean };
    result: AlbumArtworkSelection | null;
  };
  album_artwork_cache_get_for_track: {
    args: { trackId: string };
    result: ArrayBuffer;
  };
  album_artwork_cache_put_for_track: {
    args: { trackId: string; sha256: string; dataBase64: string };
    result: void;
  };
  album_artwork_cache_mark_missing_for_track: {
    args: { trackId: string };
    result: void;
  };
  library_get_artist_page: {
    args: { query: string | null; queryAlternates: string[]; sort: LibraryListSort; offset: number; limit: number; artistNames: string[] };
    result: TrackPage;
  };
  library_list_sources: {
    args: Record<string, never>;
    result: LibrarySource[];
  };
  library_add_windows_folder: {
    args: { path: string };
    result: LibrarySource;
  };
  library_pick_windows_folder: {
    args: Record<string, never>;
    result: LibrarySource | null;
  };
  library_list_excluded_folders: {
    args: Record<string, never>;
    result: ExcludedFolder[];
  };
  library_pick_excluded_folder: {
    args: Record<string, never>;
    result: ExcludedFolder | null;
  };
  library_remove_excluded_folder: {
    args: { excludedId: string };
    result: ExcludedFolder[];
  };
  library_set_source_enabled: {
    args: { sourceId: string; enabled: boolean };
    result: LibrarySource[];
  };
  library_remove_source: {
    args: { sourceId: string };
    result: LibrarySource[];
  };
  android_pick_folder_source: {
    args: Record<string, never>;
    result: LibrarySource | null;
  };
  library_sync: {
    args: Record<string, never>;
    result: LibrarySyncResult;
  };
  playlist_list: {
    args: Record<string, never>;
    result: PlaylistSummary[];
  };
  playlist_get_page: {
    args: { playlistId: string; sort?: PlaylistListSort; offset: number; limit: number };
    result: PlaylistPage;
  };
  playlist_import_m3u: {
    args: Record<string, never>;
    result: PlaylistImportResult | null;
  };
  playlist_export_m3u: {
    args: { playlistId: string; format: 'm3u' | 'm3u8'; relativePaths: boolean };
    result: PlaylistExportResult | null;
  };
  favorites_get_playlists: {
    args: { trackId: string };
    result: FavoritePlaylists;
  };
  favorites_set_membership: {
    args: { sourceId: string; trackId: string; selected: boolean };
    result: void;
  };
  favorites_create_my_love: {
    args: Record<string, never>;
    result: void;
  };
  favorites_create_playlist: {
    args: Record<string, never>;
    result: string | null;
  };
  /** Reload the current track's summary, cover and system-media metadata after its tags were saved. */
  playback_refresh_current_track: {
    args: Record<string, never>;
    result: PlaybackSnapshot;
  };
  playback_get_snapshot: {
    args: Record<string, never>;
    result: PlaybackSnapshot;
  };
  playback_get_queue_page: {
    args: { offset: number; limit: number };
    result: PlaybackQueuePage;
  };
  playback_play: {
    args: { trackId: string; queueSource?: PlaybackQueueSource };
    result: PlaybackSnapshot;
  };
  playback_pause: {
    args: Record<string, never>;
    result: PlaybackSnapshot;
  };
  sleep_timer_get: {
    args: Record<string, never>;
    result: SleepTimerSnapshot;
  };
  sleep_timer_set: {
    args: { durationMs: number };
    result: SleepTimerSnapshot;
  };
  playback_next: {
    args: Record<string, never>;
    result: PlaybackSnapshot;
  };
  playback_previous: {
    args: Record<string, never>;
    result: PlaybackSnapshot;
  };
  playback_seek: {
    args: { positionMs: number };
    result: PlaybackSnapshot;
  };
  playback_set_volume: {
    args: { volume: number };
    result: PlaybackSnapshot;
  };
  playback_set_repeat: {
    args: { mode: RepeatMode };
    result: PlaybackSnapshot;
  };
  playback_set_shuffle: {
    args: { enabled: boolean };
    result: PlaybackSnapshot;
  };
  playback_reorder_queue: {
    args: { from: number; to: number };
    result: PlaybackSnapshot;
  };
  playback_sort_queue: {
    args: { sort: LibraryListSort };
    result: PlaybackSnapshot;
  };
  lyrics_get_track: {
    args: { trackId: string };
    result: LyricsTrackResult;
  };
  lyrics_search: {
    args: { trackId: string; requestId: string; manual?: boolean; query?: string; queryAlternates?: string[] };
    result: LyricsTrackResult;
  };
  lyrics_load_more: {
    args: { trackId: string };
    result: LyricsTrackResult;
  };
  lyrics_select_candidate: {
    args: { trackId: string; candidateId: string };
    result: TrackLyrics;
  };
  lyrics_cancel_search: {
    args: { requestId: string };
    result: void;
  };
  lyrics_clear_track: {
    args: { trackId: string };
    result: LyricsTrackResult;
  };
}

export type IpcCommand = keyof IpcContract;

/** Android MANAGE_EXTERNAL_STORAGE ("All files access") state (ADR 0014). */
export type AndroidAllFilesAccessStatus = {
  supported: boolean;
  /** Android 10 or older: legacy READ/WRITE_EXTERNAL_STORAGE runtime permission. */
  legacy?: boolean;
  granted: boolean;
};

/** Android FLAG_SHOW_WALLPAPER backdrop state reported by the native plugin. */
export type AndroidWallpaperBackdropStatus = {
  active: boolean;
  blurApplied: boolean;
  blurSupported: boolean;
};

export function invokeCommand<Command extends IpcCommand>(
  command: Command,
  args: IpcContract[Command]['args'],
): Promise<IpcContract[Command]['result']> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(tr('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。')),
    );
  }

  return invoke<IpcContract[Command]['result']>(
    command,
    args as Record<string, unknown>,
  );
}

/** Fetch one active track's raw binary cover payload without JSON/base64 conversion. */
export function getTrackArtworkBytes(trackId: string): Promise<ArrayBuffer> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(tr('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。')),
    );
  }
  return invoke<ArrayBuffer>('library_get_track_artwork', { trackId });
}

/** Retrieve a previously generated square album-card thumbnail by source-artwork SHA-256. */
export function getAlbumThumbnailBytes(sha256: string): Promise<ArrayBuffer> {
  if (!isTauri()) return Promise.reject(new Error(tr('專輯縮圖快取需要 MoeMusicPlayer 桌面程式。')));
  return invoke<ArrayBuffer>('album_artwork_cache_get', { sha256 });
}

/** Save a generated JPEG thumbnail to the rebuildable platform cache. */
export function putAlbumThumbnailBytes(sha256: string, bytes: ArrayBuffer): Promise<void> {
  if (!isTauri()) return Promise.reject(new Error(tr('專輯縮圖快取需要 MoeMusicPlayer 桌面程式。')));
  const binary = new Uint8Array(bytes);
  let binaryText = '';
  for (let offset = 0; offset < binary.length; offset += 0x8000) {
    binaryText += String.fromCharCode(...binary.subarray(offset, offset + 0x8000));
  }
  const data = btoa(binaryText);
  return invoke<void>('album_artwork_cache_put', { sha256, dataBase64: data });
}

/** Save thumbnail bytes already base64-encoded by the background artwork worker. */
export function putAlbumThumbnailBase64(sha256: string, dataBase64: string): Promise<void> {
  if (!isTauri()) return Promise.reject(new Error(tr('專輯縮圖快取需要 MoeMusicPlayer 桌面程式。')));
  return invoke<void>('album_artwork_cache_put', { sha256, dataBase64 });
}

/** Retrieve an album card thumbnail using the current track file fingerprint. */
export function getAlbumThumbnailBytesForTrack(trackId: string): Promise<ArrayBuffer> {
  return invokeCommand('album_artwork_cache_get_for_track', { trackId });
}

/** Save the content-addressed thumbnail and associate it with the track fingerprint. */
export function putAlbumThumbnailBytesForTrack(trackId: string, sha256: string, bytes: ArrayBuffer): Promise<void> {
  const binary = new Uint8Array(bytes);
  let binaryText = '';
  for (let offset = 0; offset < binary.length; offset += 0x8000) {
    binaryText += String.fromCharCode(...binary.subarray(offset, offset + 0x8000));
  }
  return invokeCommand('album_artwork_cache_put_for_track', { trackId, sha256, dataBase64: btoa(binaryText) });
}

/** Save thumbnail bytes already base64-encoded by the background artwork worker. */
export function putAlbumThumbnailBase64ForTrack(trackId: string, sha256: string, dataBase64: string): Promise<void> {
  return invokeCommand('album_artwork_cache_put_for_track', { trackId, sha256, dataBase64 });
}

export function markAlbumArtworkMissing(trackId: string): Promise<void> {
  return invokeCommand('album_artwork_cache_mark_missing_for_track', { trackId });
}

export function getMainBackgroundBytes(): Promise<ArrayBuffer> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(tr('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。')),
    );
  }
  return invoke<ArrayBuffer>('appearance_get_main_background', {});
}

export function nextSlideshowBackgroundBytes(): Promise<ArrayBuffer> {
  if (!isTauri()) {
    return Promise.reject(
      new Error(tr('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。')),
    );
  }
  return invoke<ArrayBuffer>('appearance_next_slideshow_background', {});
}

export function getNowPlayingAppearancePreferences(): Promise<NowPlayingAppearancePreferences> {
  return invokeCommand('settings_get_now_playing_appearance_preferences', {});
}

export function setNowPlayingAppearancePreferences(
  preferences: NowPlayingAppearancePreferences,
): Promise<NowPlayingAppearancePreferences> {
  return invokeCommand('settings_set_now_playing_appearance_preferences', { preferences });
}

export function isReady(capability: FeatureCapability | undefined): boolean {
  return capability?.state === 'ready';
}

export function getErrorText(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return tr('桌面服務目前無法回應。');
}
