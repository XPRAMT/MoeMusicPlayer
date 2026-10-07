import { invoke, isTauri } from '@tauri-apps/api/core';

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

export interface ThemePreferences {
  backgroundHex: string;
  accentHex: string;
  quickSettingsOpacityPercent: number;
}

export type TrackListColumnId = 'title' | 'artist' | 'album' | 'year' | 'audioFormat' | 'duration' | 'playCount';

export interface TrackListColumnPreference {
  id: TrackListColumnId;
  visible: boolean;
}

export interface TrackListColumnSettings {
  columns: TrackListColumnPreference[];
}

export type NowPlayingLayout = 'a' | 'b';
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

export interface MediaStoreVolumeOption {
  volumeName: string;
  displayName: string;
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
  year?: number | null;
  bitDepth?: number | null;
  /** Cumulative credited listening time in milliseconds. */
  playedMs?: number | null;
}

export interface TrackPage {
  items: TrackSummary[];
  offset: number;
  limit: number;
  totalCount: number;
}

export interface TrackPageRequest {
  query: string | null;
  fieldFilter?: TrackFieldFilter | null;
  offset: number;
  limit: number;
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
  year?: number | null;
  codec?: string | null;
  sampleRateHz?: number | null;
  bitDepth?: number | null;
  bitrateBps?: number | null;
  playedMs?: number | null;
}

export interface PlaylistPage {
  items: PlaylistEntrySummary[];
  offset: number;
  limit: number;
  totalCount: number;
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

export type RepeatMode = 'off' | 'one' | 'all';

export type PlaybackQueueSource =
  | { kind: 'library'; query: string | null; fieldFilter?: TrackFieldFilter | null }
  | { kind: 'playlist'; playlistId: string; entryPosition: number };
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
}

export type LyricsResultStatus = 'ready' | 'empty' | 'candidates' | 'error';

export type ShortcutAction = 'fullscreen' | 'playPause' | 'seekBack' | 'seekForward' | 'previous' | 'next';
export type ShortcutDevice = 'keyboard' | 'mouse';
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
  backgroundBrightnessPercent: number;
  coverCornerStyle: CoverCornerStyle;
  timelineStyle: TimelineStyle;
}

interface IpcContract {
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
  settings_get_track_list_columns: {
    args: Record<string, never>;
    result: TrackListColumnSettings;
  };
  settings_set_track_list_columns: {
    args: { preferences: TrackListColumnSettings };
    result: TrackListColumnSettings;
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
  library_set_source_enabled: {
    args: { sourceId: string; enabled: boolean };
    result: LibrarySource[];
  };
  library_remove_source: {
    args: { sourceId: string };
    result: LibrarySource[];
  };
  android_media_request_permission: {
    args: Record<string, never>;
    result: boolean;
  };
  android_media_list_volumes: {
    args: Record<string, never>;
    result: MediaStoreVolumeOption[];
  };
  android_media_add_volume: {
    args: { volumeName: string };
    result: LibrarySource;
  };
  android_saf_pick_source: {
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
    args: { playlistId: string; offset: number; limit: number };
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
  lyrics_get_track: {
    args: { trackId: string };
    result: LyricsTrackResult;
  };
  lyrics_search: {
    args: { trackId: string; requestId: string; manual?: boolean; query?: string };
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

export function invokeCommand<Command extends IpcCommand>(
  command: Command,
  args: IpcContract[Command]['args'],
): Promise<IpcContract[Command]['result']> {
  if (!isTauri()) {
    return Promise.reject(
      new Error('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。'),
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
      new Error('目前是瀏覽器預覽；請從 MoeMusicPlayer 桌面程式開啟本機曲庫。'),
    );
  }
  return invoke<ArrayBuffer>('library_get_track_artwork', { trackId });
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
  return '桌面服務目前無法回應。';
}
