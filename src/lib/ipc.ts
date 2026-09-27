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

export interface LibrarySource {
  id: string;
  kind: string;
  displayName: string;
  enabled: boolean;
  syncState: string | null;
  lastAttemptUtcMs: number | null;
  lastSuccessUtcMs: number | null;
  errorCount: number;
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
}

export interface TrackPage {
  items: TrackSummary[];
  offset: number;
  limit: number;
  totalCount: number;
}

export interface TrackPageRequest {
  query: string | null;
  offset: number;
  limit: number;
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
}

export interface PlaylistPage {
  items: PlaylistEntrySummary[];
  offset: number;
  limit: number;
  totalCount: number;
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
}

interface IpcContract {
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
  playback_play: {
    args: { trackId: string };
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

export function isReady(capability: FeatureCapability | undefined): boolean {
  return capability?.state === 'ready';
}

export function getErrorText(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  return '桌面服務目前無法回應。';
}
