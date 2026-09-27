<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import {
    getErrorText,
    getTrackArtworkBytes,
    invokeCommand,
    isReady,
    type LibrarySyncFinishedEvent,
    type LibrarySyncProgressEvent,
    type LibrarySource,
    type MediaStoreVolumeOption,
    type FeatureCapability,
    type PlaylistEntrySummary,
    type PlaylistPage,
    type PlaylistSummary,
    type PlaybackSnapshot,
    type PlaybackQueueSource,
    type PlaybackState,
    type RuntimeCapabilities,
    type ThemePreferences,
    type TrackSummary,
  } from './lib/ipc';
  import {
    applyTheme,
    DEFAULT_THEME_PREFERENCES,
    isHexColor,
    normalizeThemePreferences,
  } from './lib/theme';
  import { formatDuration, formatVolume } from './lib/format';
  import TrackList from './lib/TrackList.svelte';
  import {
    beginPlaybackSeekDraft,
    commitPlaybackSeekDraft,
    isPlaybackSeekableDuration,
    playbackSeekDisplayPosition,
    shouldClearPendingPlaybackSeek,
    updatePlaybackSeekDraft,
    type PendingPlaybackSeek,
    type PlaybackSeekDraft,
  } from './lib/playback-scrubber';
  import {
    createActiveTrackArtworkController,
    type ActiveArtworkState,
  } from './lib/active-track-artwork';

  type View = 'library' | 'now-playing' | 'playlists' | 'settings';
  type SettingsSection = 'appearance' | 'sources';
  type SyncProgressViewState = {
    runId: string;
    sourceCount: number;
    sources: Record<string, LibrarySyncProgressEvent>;
    active: boolean;
    summary: string | null;
    error: string | null;
  };

  const PAGE_SIZE = 40;

  let activeView = $state<View>('library');
  let settingsSection = $state<SettingsSection>('appearance');
  let themePreferences = $state<ThemePreferences>({ ...DEFAULT_THEME_PREFERENCES });
  let themeSaveState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let themeSaveError = $state<string | null>(null);
  let capabilities = $state<RuntimeCapabilities | null>(null);
  let runtimeError = $state<string | null>(null);
  let libraryTrackCount = $state<number | null>(null);
  let libraryListRevision = $state(0);
  let playbackError = $state<string | null>(null);
  let playback = $state<PlaybackSnapshot | null>(null);
  let activeArtwork = $state<ActiveArtworkState>({ trackId: null, status: 'empty', objectUrl: null });
  let playbackSeekDraft = $state<PlaybackSeekDraft | null>(null);
  let pendingPlaybackSeek = $state<PendingPlaybackSeek | null>(null);
  let sources = $state<LibrarySource[]>([]);
  let sourceError = $state<string | null>(null);
  let sourceSyncSummary = $state<string | null>(null);
  let syncProgress = $state<SyncProgressViewState | null>(null);
  let mediaStoreVolumes = $state<MediaStoreVolumeOption[]>([]);
  let mediaPermissionGranted = $state<boolean | null>(null);
  let playlists = $state<PlaylistSummary[]>([]);
  let playlistPage = $state<PlaylistPage | null>(null);
  let selectedPlaylistId = $state<string | null>(null);
  let playlistError = $state<string | null>(null);
  let playlistMessage = $state<string | null>(null);
  let playlistExportFormat = $state<'m3u' | 'm3u8'>('m3u8');
  let playlistExportRelativePaths = $state(false);
  let query = $state('');
  let selectedTrackId = $state<string | null>(null);
  let isSyncing = $state(false);
  let isLoadingSources = $state(false);
  let isUpdatingSource = $state(false);
  let isLoadingVolumes = $state(false);
  let isLoadingPlaylists = $state(false);
  let isLoadingPlaylistPage = $state(false);
  let isPlaylistOperation = $state(false);
  let isLoadingPlaybackSnapshot = false;
  let isSendingPlaybackCommand = $state(false);
  let playbackSnapshotRequestVersion = 0;
  let themeSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let themeSaveQueue: Promise<void> = Promise.resolve();
  let themeRevision = 0;
  let playbackPollTimer: ReturnType<typeof setInterval> | undefined;
  let playlistPageRequestVersion = 0;
  let capabilityRefreshInFlight = false;
  let nextCapabilityRefreshAt = 0;
  let unlistenSyncFinished: UnlistenFn | undefined;
  let unlistenSyncProgress: UnlistenFn | undefined;

  const artworkController = createActiveTrackArtworkController({
    fetchBytes: getTrackArtworkBytes,
    createObjectUrl(bytes, mimeType) {
      return URL.createObjectURL(new Blob([bytes], { type: mimeType }));
    },
    revokeObjectUrl(objectUrl) {
      URL.revokeObjectURL(objectUrl);
    },
    onChange(state) {
      activeArtwork = state;
    },
  });

  const libraryReady = $derived(isReady(capabilities?.library));
  const sourceSyncReady = $derived(isReady(capabilities?.sourceSync));
  const playbackReady = $derived(isReady(capabilities?.playback));
  const playbackNavigationReady = $derived(isReady(capabilities?.playbackNavigation));
  const playbackModesReady = $derived(isReady(capabilities?.playbackModes));
  const playbackSeekPositionMs = $derived(playbackSeekDisplayPosition(
    playback?.positionMs ?? 0,
    playback?.durationMs ?? null,
    playback?.currentTrack?.id ?? null,
    playbackSeekDraft,
    pendingPlaybackSeek,
  ));
  const playlistExchangeReady = $derived(isReady(capabilities?.playlistExchange));
  const selectedPlaylist = $derived(
    playlists.find((playlist) => playlist.id === selectedPlaylistId) ?? null,
  );
  const hasPreviousPlaylistPage = $derived((playlistPage?.offset ?? 0) > 0);
  const hasNextPlaylistPage = $derived(
    playlistPage !== null && playlistPage.offset + playlistPage.items.length < playlistPage.totalCount,
  );
  const playlistPageRangeLabel = $derived.by(() => {
    if (!playlistPage || playlistPage.totalCount === 0) return '0 個項目';
    const first = playlistPage.offset + 1;
    const last = Math.min(playlistPage.offset + playlistPage.items.length, playlistPage.totalCount);
    return `${first}–${last} 項，共 ${playlistPage.totalCount.toLocaleString()} 項`;
  });
  const syncProgressSources = $derived.by(() =>
    syncProgress
      ? Object.values(syncProgress.sources).sort((left, right) => left.sourceIndex - right.sourceIndex)
      : [],
  );
  const syncProgressDoneCount = $derived(
    syncProgressSources.filter((source) => source.outcome !== null).length,
  );

  $effect(() => {
    const snapshot = playback;
    if (!snapshot?.currentTrack || snapshot.state === 'stopped' || snapshot.state === 'empty') {
      artworkController.setTrack(null);
    } else {
      artworkController.setTrack(snapshot.currentTrack.id);
    }
  });

  onMount(() => {
    let disposed = false;
    if (isTauri()) {
      playbackPollTimer = setInterval(() => {
        if (playbackReady && !isSendingPlaybackCommand) {
          void loadPlaybackSnapshot(false);
        }
      }, 250);
      void initializeTauri();
    } else {
      themeSaveState = 'preview';
      void loadCapabilities();
    }

    async function initializeTauri(): Promise<void> {
      let unlistenProgress: (() => void) | null = null;
      let unlistenFinished: (() => void) | null = null;
      try {
        unlistenProgress = await listen<LibrarySyncProgressEvent>('library-sync-progress', (event) => {
          handleLibrarySyncProgress(event.payload);
        });
        unlistenFinished = await listen<LibrarySyncFinishedEvent>('library-sync-finished', (event) => {
          handleLibrarySyncFinished(event.payload);
          libraryListRevision += 1;
          if (libraryReady) void loadPlaylists();
          if (sourceSyncReady) void loadSources();
        });
      } catch (error) {
        unlistenProgress?.();
        unlistenFinished?.();
        await loadCapabilities();
        if (!disposed) runtimeError = `無法訂閱曲庫同步狀態：${getErrorText(error)}`;
        return;
      }

      if (disposed) {
        unlistenProgress();
        unlistenFinished();
        return;
      }

      unlistenSyncProgress = unlistenProgress;
      unlistenSyncFinished = unlistenFinished;
      await Promise.all([loadCapabilities(), loadThemePreferences()]);
      if (!disposed && sourceSyncReady) void syncLibrary();
    }

    return () => {
      disposed = true;
      unlistenSyncProgress?.();
      unlistenSyncFinished?.();
    };
  });

  onDestroy(() => {
    artworkController.dispose();
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeRevision += 1;
    if (playbackPollTimer !== undefined) clearInterval(playbackPollTimer);
    unlistenSyncProgress?.();
    unlistenSyncFinished?.();
    playlistPageRequestVersion += 1;
  });

  async function loadThemePreferences(): Promise<void> {
    const revision = themeRevision;
    try {
      const stored = await invokeCommand('theme_get_preferences', {});
      if (revision !== themeRevision) return;
      themePreferences = normalizeThemePreferences(stored);
      applyTheme(document.documentElement, themePreferences);
      themeSaveState = 'saved';
      themeSaveError = null;
    } catch (error) {
      if (revision !== themeRevision) return;
      themeSaveState = 'error';
      themeSaveError = `無法讀取已保存的外觀設定：${getErrorText(error)}`;
    }
  }

  function updateThemeColor(key: keyof ThemePreferences, value: string): void {
    if (!isHexColor(value)) return;
    const next = normalizeThemePreferences({ ...themePreferences, [key]: value });
    themePreferences = next;
    applyTheme(document.documentElement, next);
    themeSaveError = null;
    themeSaveState = isTauri() ? 'saving' : 'preview';
    const revision = ++themeRevision;
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = setTimeout(() => {
      themeSaveTimer = undefined;
      queueThemeSave(next, revision);
    }, 300);
  }

  function saveThemePreferencesNow(): void {
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = undefined;
    queueThemeSave({ ...themePreferences }, themeRevision);
  }

  function queueThemeSave(preferences: ThemePreferences, revision: number): void {
    if (!isTauri()) {
      themeSaveState = 'preview';
      themeSaveError = '瀏覽器預覽不會保存外觀設定。';
      return;
    }

    themeSaveQueue = themeSaveQueue.then(async () => {
      if (revision !== themeRevision) return;
      themeSaveState = 'saving';
      try {
        const saved = await invokeCommand('theme_set_preferences', { preferences });
        if (revision !== themeRevision) return;
        themePreferences = normalizeThemePreferences(saved);
        applyTheme(document.documentElement, themePreferences);
        themeSaveState = 'saved';
        themeSaveError = null;
      } catch (error) {
        if (revision !== themeRevision) return;
        themeSaveState = 'error';
        themeSaveError = `無法保存外觀設定：${getErrorText(error)}`;
      }
    });
  }

  function resetThemePreferences(): void {
    themePreferences = { ...DEFAULT_THEME_PREFERENCES };
    applyTheme(document.documentElement, themePreferences);
    themeSaveError = null;
    themeSaveState = isTauri() ? 'saving' : 'preview';
    const revision = ++themeRevision;
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = undefined;
    queueThemeSave({ ...themePreferences }, revision);
  }

  const themeSaveMessage = $derived.by(() => {
    if (themeSaveError) return themeSaveError;
    if (themeSaveState === 'loading') return '正在讀取外觀設定…';
    if (themeSaveState === 'saving') return '正在保存到此裝置…';
    if (themeSaveState === 'preview') return '目前為預覽；桌面版會將設定保存到此裝置。';
    return '外觀設定已保存到此裝置。';
  });

  function handleLibrarySyncProgress(event: LibrarySyncProgressEvent): void {
    const current = syncProgress?.runId === event.runId
      ? syncProgress
      : {
          runId: event.runId,
          sourceCount: event.sourceCount,
          sources: {},
          active: true,
          summary: null,
          error: null,
        };
    const sources = { ...current.sources, [event.sourceId]: event };
    const completedSources = Object.values(sources).filter((source) => source.outcome !== null);
    const isComplete = event.sourceCount > 0 && completedSources.length >= event.sourceCount;
    const summary = isComplete ? formatProgressSummary(completedSources) : null;
    syncProgress = {
      ...current,
      sourceCount: event.sourceCount,
      sources,
      active: !isComplete,
      summary,
      error: null,
    };
    isSyncing = !isComplete;
    if (summary) sourceSyncSummary = summary;
  }

  function handleLibrarySyncFinished(event: LibrarySyncFinishedEvent): void {
    const current = syncProgress?.runId === event.runId
      ? syncProgress
      : {
          runId: event.runId,
          sourceCount: event.sourceCount,
          sources: {},
          active: false,
          summary: null,
          error: null,
        };
    const sources = { ...current.sources };
    event.sources.forEach((source, sourceIndex) => {
      if (sources[source.sourceId]) return;
      const outcome = source.state === 'complete'
        ? 'complete'
        : source.state === 'incomplete'
          ? 'incomplete'
          : source.state === 'unavailable'
            ? 'unavailable'
            : source.state === 'permissionRevoked'
              ? 'permissionRevoked'
              : 'failed';
      sources[source.sourceId] = {
        runId: event.runId,
        sourceIndex,
        sourceCount: event.sourceCount,
        displayName: source.displayName,
        sourceId: source.sourceId,
        stage: 'finished',
        processed: source.observed,
        total: source.observed,
        unit: 'tracks',
        observed: source.observed,
        metadataReads: source.metadataReads,
        unchanged: source.unchanged,
        errorCount: source.errorCount,
        outcome,
      };
    });
    const summary = event.error
      ? `同步失敗：${event.error}`
      : event.sources.length === 0
        ? '尚未加入可同步的音樂來源。'
        : formatProgressSummary(Object.values(sources));
    syncProgress = {
      ...current,
      sourceCount: event.sourceCount,
      sources,
      active: false,
      summary,
      error: event.error,
    };
    sourceSyncSummary = summary;
    if (event.error) sourceError = event.error;
    isSyncing = false;
  }

  function formatProgressSummary(sources: LibrarySyncProgressEvent[]): string {
    const complete = sources.filter((source) => source.outcome === 'complete').length;
    const errorCount = sources.reduce((sum, source) => sum + source.errorCount, 0);
    return `${sources.length} 個來源已檢查；${complete} 個完整，${sources.length - complete} 個需要留意；${errorCount} 個項目錯誤。`;
  }

  function syncProgressLine(progress: LibrarySyncProgressEvent): string {
    if (progress.stage === 'finished') {
      const outcome = progress.outcome === 'complete'
        ? '同步完成'
        : progress.outcome === 'cancelled'
          ? '已取消'
          : progress.outcome === 'unavailable'
            ? '來源暫不可用'
            : progress.outcome === 'permissionRevoked'
              ? '需要重新授權'
              : progress.outcome === 'failed'
                ? '同步失敗'
                : '掃描未完成';
      return `${outcome}；${progress.errorCount.toLocaleString()} 個項目錯誤`;
    }
    if (progress.stage === 'enumerating') {
      const count = progress.processed.toLocaleString();
      return progress.total === null
        ? `掃描中，已檢查 ${count} 個檔案系統項目；總數尚未確定`
        : `掃描中，已檢查 ${count} / ${progress.total.toLocaleString()} 個檔案系統項目`;
    }
    const stage = progress.stage === 'metadata' ? '讀取曲目資訊' : '寫入曲庫';
    const total = progress.total === null ? '—' : progress.total.toLocaleString();
    return `${stage}，${progress.processed.toLocaleString()} / ${total} 首曲目`;
  }

  async function loadCapabilities(): Promise<void> {
    runtimeError = null;
    try {
      capabilities = await invokeCommand('get_runtime_capabilities', {});
      if (isReady(capabilities.library)) void loadPlaylists();
      if (isReady(capabilities.sourceSync)) void loadSources();
      if (isReady(capabilities.playback)) void loadPlaybackSnapshot();
    } catch (error) {
      capabilities = null;
      runtimeError = getErrorText(error);
    }
  }

  async function refreshCapabilitiesInBackground(): Promise<void> {
    const now = Date.now();
    if (capabilityRefreshInFlight || now < nextCapabilityRefreshAt) return;
    capabilityRefreshInFlight = true;
    nextCapabilityRefreshAt = now + 3000;
    try {
      capabilities = await invokeCommand('get_runtime_capabilities', {});
    } catch {
      // Playback polling remains authoritative for playback errors.
    } finally {
      capabilityRefreshInFlight = false;
    }
  }

  function handleSearchInput(event: Event): void {
    query = (event.currentTarget as HTMLInputElement).value;
  }

  async function syncLibrary(): Promise<void> {
    if (!sourceSyncReady) return;
    isSyncing = true;
    sourceError = null;
    try {
      const result = await invokeCommand('library_sync', {});
      const completed = result.sources.filter((source) => source.state === 'complete').length;
      const partial = result.sources.length - completed;
      sourceSyncSummary = result.sources.length === 0
        ? '尚未加入可同步的音樂來源。'
        : `${result.sources.length} 個來源已檢查；${completed} 個完整，${partial} 個需要留意。`;
      await loadSources();
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isSyncing = false;
    }
  }

  async function loadSources(): Promise<void> {
    if (!sourceSyncReady) return;
    isLoadingSources = true;
    sourceError = null;
    try {
      sources = await invokeCommand('library_list_sources', {});
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isLoadingSources = false;
    }
  }

  async function loadPlaylists(preferredId?: string): Promise<void> {
    if (!libraryReady) return;
    isLoadingPlaylists = true;
    playlistError = null;
    try {
      playlists = await invokeCommand('playlist_list', {});
      const requestedId = preferredId ?? selectedPlaylistId;
      const nextId = playlists.some((playlist) => playlist.id === requestedId)
        ? requestedId
        : playlists[0]?.id ?? null;
      selectedPlaylistId = nextId;
      if (nextId) await loadPlaylistPage(nextId, 0);
      else playlistPage = null;
    } catch (error) {
      playlistError = getErrorText(error);
    } finally {
      isLoadingPlaylists = false;
    }
  }

  async function loadPlaylistPage(playlistId: string, nextOffset: number): Promise<void> {
    const requestVersion = ++playlistPageRequestVersion;
    isLoadingPlaylistPage = true;
    playlistError = null;
    try {
      const nextPage = await invokeCommand('playlist_get_page', {
        playlistId,
        offset: nextOffset,
        limit: PAGE_SIZE,
      });
      if (requestVersion !== playlistPageRequestVersion || selectedPlaylistId !== playlistId) return;
      playlistPage = nextPage;
    } catch (error) {
      if (requestVersion === playlistPageRequestVersion) {
        playlistPage = null;
        playlistError = getErrorText(error);
      }
    } finally {
      if (requestVersion === playlistPageRequestVersion) isLoadingPlaylistPage = false;
    }
  }

  async function selectPlaylist(playlistId: string): Promise<void> {
    selectedPlaylistId = playlistId;
    playlistMessage = null;
    await loadPlaylistPage(playlistId, 0);
  }

  async function importPlaylist(): Promise<void> {
    if (!playlistExchangeReady || isPlaylistOperation) return;
    isPlaylistOperation = true;
    playlistError = null;
    playlistMessage = null;
    try {
      const result = await invokeCommand('playlist_import_m3u', {});
      if (!result) return;
      activeView = 'playlists';
      await loadPlaylists(result.playlist.id);
      playlistMessage = `已匯入「${result.playlist.name}」：${result.playlist.entryCount.toLocaleString()} 個項目，其中 ${result.matchedEntries.toLocaleString()} 個對應到曲庫。`;
    } catch (error) {
      playlistError = getErrorText(error);
    } finally {
      isPlaylistOperation = false;
    }
  }

  async function exportPlaylist(playlistId: string): Promise<void> {
    if (!playlistExchangeReady || isPlaylistOperation) return;
    isPlaylistOperation = true;
    playlistError = null;
    playlistMessage = null;
    try {
      const result = await invokeCommand('playlist_export_m3u', {
        playlistId,
        format: playlistExportFormat,
        relativePaths: playlistExportRelativePaths,
      });
      if (result) {
        playlistMessage = `已匯出「${result.playlistName}」的 ${result.entryCount.toLocaleString()} 個項目。`;
      }
    } catch (error) {
      playlistError = getErrorText(error);
    } finally {
      isPlaylistOperation = false;
    }
  }

  async function playPlaylistEntry(entry: PlaylistEntrySummary): Promise<void> {
    if (!entry.trackId || !entry.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand) return;
    const queueSource: PlaybackQueueSource = {
      kind: 'playlist',
      playlistId: selectedPlaylistId!,
      entryPosition: entry.position,
    };
    await sendPlaybackCommand(() => invokeCommand('playback_play', { trackId: entry.trackId!, queueSource }));
    if (!playbackError) activeView = 'now-playing';
  }

  async function pickWindowsFolder(): Promise<void> {
    if (!sourceSyncReady || isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      const source = await invokeCommand('library_pick_windows_folder', {});
      if (source) {
        await loadSources();
        await syncLibrary();
      }
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
    }
  }

  async function requestMediaStorePermission(): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      mediaPermissionGranted = await invokeCommand('android_media_request_permission', {});
      if (mediaPermissionGranted) await loadMediaStoreVolumes();
      else sourceError = '尚未授權讀取共享音樂；既有曲庫仍會保留。';
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
    }
  }

  async function loadMediaStoreVolumes(): Promise<void> {
    isLoadingVolumes = true;
    sourceError = null;
    try {
      mediaStoreVolumes = await invokeCommand('android_media_list_volumes', {});
      mediaPermissionGranted = true;
    } catch (error) {
      mediaStoreVolumes = [];
      sourceError = getErrorText(error);
    } finally {
      isLoadingVolumes = false;
    }
  }

  async function addMediaStoreVolume(volume: MediaStoreVolumeOption): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      await invokeCommand('android_media_add_volume', { volumeName: volume.volumeName });
      await loadSources();
      await syncLibrary();
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
    }
  }

  async function pickSafSource(): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      const source = await invokeCommand('android_saf_pick_source', {});
      if (source) {
        await loadSources();
        await syncLibrary();
      }
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
    }
  }

  function sourceStateLabel(state: string | null): string {
    switch (state) {
      case 'complete': return '同步完成';
      case 'incomplete': return '掃描未完成，保留既有曲目';
      case 'unavailable': return '來源暫不可用，保留既有曲目';
      case 'permission_revoked':
      case 'permissionRevoked': return '需要重新授權，保留既有曲目';
      default: return '尚未同步';
    }
  }

  async function loadPlaybackSnapshot(clearError = true): Promise<void> {
    if (!playbackReady || isLoadingPlaybackSnapshot) return;
    const requestVersion = ++playbackSnapshotRequestVersion;
    isLoadingPlaybackSnapshot = true;
    if (clearError) playbackError = null;
    try {
      applyPlaybackSnapshot(
        await invokeCommand('playback_get_snapshot', {}),
        requestVersion,
      );
      void refreshCapabilitiesInBackground();
    } catch (error) {
      playbackError = getErrorText(error);
    } finally {
      isLoadingPlaybackSnapshot = false;
    }
  }

  async function playTrack(track: TrackSummary): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = track.id;
    const queueSource: PlaybackQueueSource = {
      kind: 'library',
      query: query.trim() || null,
    };
    await sendPlaybackCommand(() => invokeCommand('playback_play', { trackId: track.id, queueSource }));
    if (!playbackError) activeView = 'now-playing';
  }

  async function togglePlayback(): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    if (playback?.isPlaying) {
      await sendPlaybackCommand(() => invokeCommand('playback_pause', {}));
    } else if (playback?.currentTrack?.id) {
      const trackId = playback.currentTrack.id;
      await sendPlaybackCommand(() =>
        invokeCommand('playback_play', { trackId }),
      );
    }
  }

  async function controlPlayback(
    command: 'playback_pause' | 'playback_next' | 'playback_previous',
  ): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    await sendPlaybackCommand(() => invokeCommand(command, {}));
  }

  function beginPlaybackSeek(): void {
    playbackSeekDraft = beginPlaybackSeekDraft(
      playback?.currentTrack?.id ?? null,
      playback?.positionMs ?? 0,
      playback?.durationMs ?? null,
    );
  }

  function updatePlaybackSeek(event: Event): void {
    playbackSeekDraft = updatePlaybackSeekDraft(
      playbackSeekDraft,
      playback?.currentTrack?.id ?? null,
      playback?.positionMs ?? 0,
      playback?.durationMs ?? null,
      Number((event.currentTarget as HTMLInputElement).value),
    );
  }

  function handlePlaybackSeekPointerEnd(): void {
    if (playbackSeekDraft) void commitPlaybackSeek();
  }

  async function commitPlaybackSeek(): Promise<void> {
    const draft = playbackSeekDraft;
    playbackSeekDraft = null;
    if (!playbackReady || isSendingPlaybackCommand) return;

    const trackId = playback?.currentTrack?.id ?? null;
    const positionMs = commitPlaybackSeekDraft(
      draft,
      trackId,
      playback?.durationMs ?? null,
    );
    if (positionMs === null || !trackId) return;

    pendingPlaybackSeek = {
      trackId,
      positionMs,
      requestedAtMs: Date.now(),
      snapshotVersionAtRequest: playbackSnapshotRequestVersion,
    };
    await sendPlaybackCommand(() => invokeCommand('playback_seek', { positionMs }));
    if (playbackError) pendingPlaybackSeek = null;
  }

  function applyPlaybackSnapshot(next: PlaybackSnapshot, snapshotVersion?: number): void {
    playback = next;
    const pending = pendingPlaybackSeek;
    if (pending) {
      if (pending.trackId !== next.currentTrack?.id) {
        pendingPlaybackSeek = null;
      } else if (snapshotVersion !== undefined && shouldClearPendingPlaybackSeek(pending, {
        trackId: next.currentTrack?.id ?? null,
        positionMs: next.positionMs,
        durationMs: next.durationMs,
        isPlaying: next.isPlaying,
      }, snapshotVersion, Date.now())) {
        pendingPlaybackSeek = null;
      }
    }
    if (playbackSeekDraft && playbackSeekDraft.trackId !== next.currentTrack?.id) {
      playbackSeekDraft = null;
    }
  }

  async function setPlaybackVolume(event: Event): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    const volume = Number((event.currentTarget as HTMLInputElement).value);
    await sendPlaybackCommand(() => invokeCommand('playback_set_volume', { volume }));
  }

  async function setRepeatMode(): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    const modes = ['off', 'all', 'one'] as const;
    const current = playback?.repeatMode ?? 'off';
    const next = modes[(modes.indexOf(current) + 1) % modes.length];
    await sendPlaybackCommand(() => invokeCommand('playback_set_repeat', { mode: next }));
  }

  async function toggleShuffle(): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    await sendPlaybackCommand(() =>
      invokeCommand('playback_set_shuffle', { enabled: !playback?.shuffle }),
    );
  }

  async function sendPlaybackCommand(
    request: () => Promise<PlaybackSnapshot>,
  ): Promise<void> {
    isSendingPlaybackCommand = true;
    playbackError = null;
    try {
      applyPlaybackSnapshot(await request());
    } catch (error) {
      playbackError = getErrorText(error);
    } finally {
      isSendingPlaybackCommand = false;
    }
  }

  function capabilityLabel(capability: FeatureCapability | undefined): string {
    if (!capability) return '等待桌面服務';
    if (capability.state === 'ready') return '已就緒';
    if (capability.state === 'unavailable') return '目前無法使用';
    return '尚未就緒';
  }

  function showCapabilityDetail(capability: FeatureCapability | undefined): string {
    return capability?.detail ?? runtimeError ?? '請由 MoeMusicPlayer 桌面程式開啟。';
  }

  function currentTrackTitle(track: TrackSummary | null | undefined): string {
    return track?.title?.trim() || '尚未選擇曲目';
  }

  function currentTrackArtist(track: TrackSummary | null | undefined): string {
    return track?.artist?.trim() || '選取曲庫中的曲目開始播放';
  }

  function playbackStateLabel(state: PlaybackState | undefined): string {
    switch (state) {
      case 'initializing': return '啟動播放引擎';
      case 'empty': return '尚未選擇曲目';
      case 'loading': return '載入中';
      case 'ready': return '已就緒';
      case 'playing': return '播放中';
      case 'paused': return '已暫停';
      case 'stopped': return '已停止';
      case 'ended': return '播放完畢';
      case 'error': return '播放發生錯誤';
      default: return '等待播放狀態';
    }
  }

  function nextPlaylistPage(): void {
    if (hasNextPlaylistPage && playlistPage && selectedPlaylistId) {
      void loadPlaylistPage(selectedPlaylistId, playlistPage.offset + PAGE_SIZE);
    }
  }

  function previousPlaylistPage(): void {
    if (hasPreviousPlaylistPage && playlistPage && selectedPlaylistId) {
      void loadPlaylistPage(selectedPlaylistId, Math.max(0, playlistPage.offset - PAGE_SIZE));
    }
  }
</script>

<svelte:window
  onpointerup={handlePlaybackSeekPointerEnd}
  onpointercancel={handlePlaybackSeekPointerEnd}
/>

<div class="app-shell">
  <aside class="sidebar" aria-label="主要導覽">
    <div class="brand-lockup">
      <div class="brand-mark" aria-hidden="true">
        <svg viewBox="0 0 40 40" fill="none">
          <path d="M16 8v19.2a5.1 5.1 0 1 1-3-4.65V13.7L29 10v13.2a5.1 5.1 0 1 1-3-4.65v-12L16 8Z" fill="currentColor" />
          <path d="M10 32.5h20" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" opacity=".46" />
        </svg>
      </div>
      <div class="brand-wordmark">
        <strong>MOE</strong>
        <span>LOCAL SOUND</span>
      </div>
    </div>

    <div class="sidebar-caption">你的音樂空間</div>
    <nav class="primary-nav">
      <button
        class="nav-link"
        class:active={activeView === 'library'}
        aria-current={activeView === 'library' ? 'page' : undefined}
        onclick={() => (activeView = 'library')}
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4.5 5.5h15v13h-15zM8 9h8M8 12.5h8M8 16h4" stroke="currentColor" stroke-width="1.55" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <span class="nav-label">曲庫</span>
        <span class="nav-arrow" aria-hidden="true">›</span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'now-playing'}
        aria-current={activeView === 'now-playing' ? 'page' : undefined}
        onclick={() => (activeView = 'now-playing')}
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M9 18V5l10-2v13M9 18a3 3 0 1 1-3-3 3 3 0 0 1 3 3Zm10-2a3 3 0 1 1-3-3 3 3 0 0 1 3 3Z" stroke="currentColor" stroke-width="1.55" stroke-linecap="round" stroke-linejoin="round" /></svg>
        <span class="nav-label">正在播放</span>
        <span class="nav-arrow" aria-hidden="true">›</span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'playlists'}
        aria-current={activeView === 'playlists' ? 'page' : undefined}
        onclick={() => (activeView = 'playlists')}
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M5 5.5h14M5 10h14M5 14.5h9M5 19h7" stroke="currentColor" stroke-width="1.55" stroke-linecap="round" /></svg>
        <span class="nav-label">播放清單</span>
        <span class="nav-arrow" aria-hidden="true">›</span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'settings'}
        aria-current={activeView === 'settings' ? 'page' : undefined}
        onclick={() => (activeView = 'settings')}
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 8.7a3.3 3.3 0 1 0 0 6.6 3.3 3.3 0 0 0 0-6.6Z" stroke="currentColor" stroke-width="1.55" /><path d="m19.4 13.6 1.2.9-1.5 2.6-1.4-.7a7.5 7.5 0 0 1-1.5.9l-.2 1.6h-3l-.2-1.6a7.5 7.5 0 0 1-1.5-.9l-1.4.7-1.5-2.6 1.2-.9a7 7 0 0 1 0-1.8l-1.2-.9 1.5-2.6 1.4.7a7.5 7.5 0 0 1 1.5-.9l.2-1.6h3l.2 1.6a7.5 7.5 0 0 1 1.5.9l1.4-.7 1.5 2.6-1.2.9a7 7 0 0 1 0 1.8Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" /></svg>
        <span class="nav-label">設定</span>
        <span class="nav-arrow" aria-hidden="true">›</span>
      </button>
    </nav>

    <div class="sidebar-rule"></div>
    <div class="sidebar-source">
      <span class="source-mini-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none"><path d="M3.5 7.5h6l2 2h9v8.8a1.2 1.2 0 0 1-1.2 1.2H4.7a1.2 1.2 0 0 1-1.2-1.2V7.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" /><path d="M3.5 9.5h17" stroke="currentColor" stroke-width="1.5" /></svg>
      </span>
      <div class="source-copy">
        <span>本機曲庫</span>
        <small>{capabilityLabel(capabilities?.library)}</small>
      </div>
      <span class="status-dot" class:ready={libraryReady} aria-hidden="true"></span>
    </div>

    <div class="sidebar-bottom">
      <span class="local-badge"><span aria-hidden="true">●</span> LOCAL FIRST</span>
      <span class="sidebar-version">MoeMusicPlayer <span>0.1</span></span>
    </div>
  </aside>

  <main class="workspace">
    <header class="topbar">
      <div class="breadcrumbs"><span>MOEMUSIC</span><span class="breadcrumb-slash">/</span><strong>{activeView === 'library' ? 'LIBRARY' : activeView === 'now-playing' ? 'NOW PLAYING' : activeView === 'playlists' ? 'PLAYLISTS' : 'SOURCES'}</strong></div>
      <div class="topbar-actions">
        <div class="runtime-pill" class:ready={isReady(capabilities?.desktopRuntime)}>
          <span class="status-dot" class:ready={isReady(capabilities?.desktopRuntime)} aria-hidden="true"></span>
          <span>{isReady(capabilities?.desktopRuntime) ? '桌面服務已連線' : '桌面服務未連線'}</span>
        </div>
        <button class="avatar-button" type="button" aria-label="使用者設定" title="使用者設定" disabled>
          <span>M</span>
        </button>
      </div>
    </header>

    <div class="page-scroll">
      <div class="page-content">
        <section class="welcome-banner">
          <div class="welcome-copy">
            <p class="eyebrow"><span class="eyebrow-line"></span> PERSONAL AUDIO LIBRARY</p>
            <h1>{activeView === 'library' ? '把喜歡的聲音，留在身邊。' : activeView === 'now-playing' ? '正在播放' : activeView === 'playlists' ? '整理想聽的曲目。' : '音樂來源'}</h1>
            <p class="welcome-description">以本機音樂為核心，曲庫先分頁查詢；來源與播放服務就緒後才會開放操作。</p>
            <div class="welcome-tags">
              <span><i></i> 本機優先</span>
              <span><i></i> 大型曲庫友善</span>
              <span><i></i> 跨平台介面</span>
            </div>
          </div>
          <div class="banner-art" aria-hidden="true">
            <div class="art-ring ring-one"></div>
            <div class="art-ring ring-two"></div>
            <div class="art-disc"><div class="disc-center"><span></span></div></div>
            <div class="art-spark spark-one"></div>
            <div class="art-spark spark-two"></div>
            <div class="art-caption">LOCAL<br />COLLECTION</div>
          </div>
          <div class="banner-index" aria-hidden="true">01 <span>/ 04</span></div>
        </section>

        {#if syncProgress}
          <section
            class="sync-progress-banner"
            class:sync-progress-finished={!syncProgress.active}
            class:sync-progress-error={Boolean(syncProgress.error)}
            data-testid="sync-progress-banner"
            role="status"
            aria-live="polite"
            aria-label="音樂來源同步狀態"
          >
            <div class="sync-progress-heading">
              <div>
                <span class="section-kicker">LIBRARY SYNC</span>
                <strong>{syncProgress.active ? '背景同步進行中' : '最近一次同步摘要'}</strong>
              </div>
              {#if syncProgress.active}
                <span class="sync-progress-count">{syncProgressDoneCount} / {syncProgress.sourceCount} 個來源完成</span>
              {/if}
            </div>
            {#if syncProgress.summary}
              <p class="sync-progress-summary">{syncProgress.summary}</p>
            {/if}
            {#if syncProgressSources.length > 0}
              <ul class="sync-progress-sources">
                {#each syncProgressSources as source (source.sourceId)}
                  <li data-source-id={source.sourceId}>
                    <span class="sync-progress-source-name">{source.displayName}</span>
                    <span class="sync-progress-source-detail">{syncProgressLine(source)}</span>
                    {#if source.total !== null && source.stage !== 'finished'}
                      <progress
                        aria-label={`${source.displayName} 階段進度`}
                        max={Math.max(1, source.total)}
                        value={Math.min(source.processed, Math.max(1, source.total))}
                      ></progress>
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}

        {#if activeView === 'library'}
          <section class="library-section" aria-labelledby="library-heading">
            <div class="section-heading">
              <div>
                <p class="section-kicker">COLLECTION</p>
                <h2 id="library-heading">我的曲庫</h2>
              </div>
              <div class="section-heading-actions">
                <span class="page-count">{libraryTrackCount?.toLocaleString() ?? '—'} <small>首曲目</small></span>
                <button
                  class="outline-button"
                  type="button"
                  onclick={syncLibrary}
                  disabled={!sourceSyncReady || isSyncing}
                  title={sourceSyncReady ? '重新同步本機來源' : showCapabilityDetail(capabilities?.sourceSync)}
                >
                  <svg class:spin={isSyncing} viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M20 7v5h-5M4 17v-5h5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /><path d="M6.2 9a6.5 6.5 0 0 1 11-2L20 12M4 12l2.2 5a6.5 6.5 0 0 0 11-2" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
                  <span>{isSyncing ? '同步中' : '重新整理'}</span>
                </button>
              </div>
            </div>

            <div class="library-toolbar">
              <label class="search-field">
                <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="10.8" cy="10.8" r="6.3" stroke="currentColor" stroke-width="1.6" /><path d="m15.5 15.5 4.2 4.2" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
                <input
                  type="search"
                  aria-label="搜尋曲庫"
                  placeholder="搜尋歌名、演出者或專輯"
                  value={query}
                  oninput={handleSearchInput}
                  disabled={!libraryReady}
                />
                <kbd>⌕</kbd>
              </label>
              <button class="filter-button" type="button" disabled title="篩選功能尚未接通">
                <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 6h16M7 12h10m-7 6h4" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg>
                <span>篩選</span>
              </button>
            </div>

            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true">
                  <svg viewBox="0 0 28 28" fill="none"><path d="M5 8h7l2.2 2.3H23v10.2a1.5 1.5 0 0 1-1.5 1.5h-15A1.5 1.5 0 0 1 5 20.5V8Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" /><path d="M5 11h18" stroke="currentColor" stroke-width="1.5" /></svg>
                </div>
                <div class="not-ready-copy">
                  <span class="state-label">NATIVE SERVICE</span>
                  <h3>{runtimeError ? '尚未連上桌面服務' : '曲庫服務尚未就緒'}</h3>
                  <p>{showCapabilityDetail(capabilities?.library)}</p>
                </div>
                <div class="not-ready-meta"><span class="status-dot" aria-hidden="true"></span> 尚未載入曲目</div>
              </div>
            {:else}
              <TrackList
                query={query}
                resetKey={libraryListRevision}
                selectedTrackId={selectedTrackId}
                playbackReady={playbackReady}
                isSendingPlaybackCommand={isSendingPlaybackCommand}
                onPlay={playTrack}
                onTotalCount={(count) => (libraryTrackCount = count)}
              />
            {/if}
          </section>
        {:else if activeView === 'playlists'}
          <section class="playlist-section" aria-labelledby="playlists-heading">
            <div class="section-heading">
              <div>
                <p class="section-kicker">PLAYLISTS</p>
                <h2 id="playlists-heading">我的播放清單</h2>
              </div>
              <div class="section-heading-actions">
                <span class="page-count">{playlists.length.toLocaleString()} <small>份清單</small></span>
                <button
                  class="primary-button playlist-import-button"
                  type="button"
                  onclick={() => void importPlaylist()}
                  disabled={!playlistExchangeReady || isPlaylistOperation}
                  title={showCapabilityDetail(capabilities?.playlistExchange)}
                >
                  {isPlaylistOperation ? '處理中' : '匯入 M3U/M3U8'}
                </button>
              </div>
            </div>

            {#if playlistError}
              <div class="inline-message" role="alert">
                <span class="message-mark">!</span>
                <div><strong>播放清單操作失敗</strong><p>{playlistError}</p></div>
                <button class="text-button" type="button" onclick={() => { if (selectedPlaylistId) void loadPlaylistPage(selectedPlaylistId, playlistPage?.offset ?? 0); else void loadPlaylists(); }}>再試一次</button>
              </div>
            {/if}
            {#if playlistMessage}
              <div class="source-result-message playlist-result-message" role="status">{playlistMessage}</div>
            {/if}

            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true"><span>♪</span></div>
                <div class="not-ready-copy"><span class="state-label">NATIVE SERVICE</span><h3>播放清單服務尚未就緒</h3><p>{showCapabilityDetail(capabilities?.library)}</p></div>
              </div>
            {:else if isLoadingPlaylists && playlists.length === 0}
              <div class="loading-panel"><span class="loader-ring"></span><span>正在載入播放清單…</span></div>
            {:else if playlists.length === 0}
              <div class="empty-panel">
                <div class="empty-wave" aria-hidden="true"><span></span><span></span><span></span><span></span><span></span></div>
                <h3>尚無播放清單</h3>
                <p>{playlistExchangeReady ? '匯入 M3U 或 M3U8 檔案；不在目前曲庫中的項目也會保留。' : showCapabilityDetail(capabilities?.playlistExchange)}</p>
              </div>
            {:else}
              <div class="playlist-browser">
                <div class="playlist-list" aria-label="播放清單">
                  {#each playlists as playlist (playlist.id)}
                    <div class="playlist-list-item" class:selected={selectedPlaylistId === playlist.id}>
                      <button
                        class="playlist-select"
                        type="button"
                        aria-pressed={selectedPlaylistId === playlist.id}
                        onclick={() => void selectPlaylist(playlist.id)}
                      >
                        <span class="playlist-select-icon" aria-hidden="true">♫</span>
                        <span class="playlist-select-copy"><strong>{playlist.name.trim() || '未命名播放清單'}</strong><small>{playlist.entryCount.toLocaleString()} 個項目</small></span>
                      </button>
                    </div>
                  {/each}
                </div>

                {#if selectedPlaylist}
                  <section class="playlist-detail" aria-labelledby="selected-playlist-heading">
                    <div class="playlist-detail-heading">
                      <div><p class="section-kicker">SELECTED PLAYLIST</p><h3 id="selected-playlist-heading">{selectedPlaylist.name.trim() || '未命名播放清單'}</h3></div>
                      <div class="playlist-export-actions">
                        <label class="playlist-export-format">
                          <span>格式</span>
                          <select bind:value={playlistExportFormat} disabled={!playlistExchangeReady || isPlaylistOperation}>
                            <option value="m3u8">M3U8</option>
                            <option value="m3u">M3U</option>
                          </select>
                        </label>
                        <label class="playlist-relative-toggle">
                          <input type="checkbox" bind:checked={playlistExportRelativePaths} disabled={!playlistExchangeReady || isPlaylistOperation} />
                          <span>相對路徑</span>
                        </label>
                        <button
                          class="outline-button"
                          type="button"
                          onclick={() => void exportPlaylist(selectedPlaylist.id)}
                          disabled={!playlistExchangeReady || isPlaylistOperation}
                          title={showCapabilityDetail(capabilities?.playlistExchange)}
                        >
                          {isPlaylistOperation ? '處理中' : `匯出 ${playlistExportFormat.toUpperCase()}`}
                        </button>
                      </div>
                    </div>

                    {#if isLoadingPlaylistPage && !playlistPage}
                      <div class="loading-panel"><span class="loader-ring"></span><span>正在載入項目…</span></div>
                    {:else if playlistPage && playlistPage.items.length === 0}
                      <div class="empty-panel playlist-empty"><h3>這份清單沒有項目</h3><p>可以重新匯入其他播放清單。</p></div>
                    {:else if playlistPage}
                      <div class="playlist-entry-table" role="table" aria-label="播放清單項目">
                        <div class="playlist-entry-head" role="row"><span>#</span><span>曲目</span><span class="playlist-entry-album">專輯／演出者</span><span class="playlist-entry-duration">長度</span><span></span></div>
                        <div class="playlist-entry-body" aria-live="polite">
                          {#each playlistPage.items as entry (entry.position)}
                            <div class="playlist-entry-row" role="row">
                              <span class="playlist-entry-index">{entry.position + 1}</span>
                              <div class="playlist-entry-title"><strong>{entry.title?.trim() || '未命名項目'}</strong><small>{entry.artist?.trim() || (entry.hasEnabledMapping ? '未知演出者' : '目前未對應到曲庫')}</small></div>
                              <span class="playlist-entry-album">{entry.album?.trim() || '—'}</span>
                              <span class="playlist-entry-duration">{formatDuration(entry.durationMs)}</span>
                              <button class="row-play" type="button" aria-label={`播放 ${entry.title?.trim() || '播放清單項目'}`} title={entry.hasEnabledMapping ? '播放曲目' : '這個項目尚未對應到可播放的曲庫曲目'} disabled={!entry.trackId || !entry.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand} onclick={() => void playPlaylistEntry(entry)}>
                                <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.3 5.8 7 4.2-7 4.2V5.8Z" fill="currentColor" /></svg>
                              </button>
                            </div>
                          {/each}
                        </div>
                      </div>
                      <div class="pagination-bar">
                        <span>{playlistPageRangeLabel}</span>
                        <div class="pagination-actions">
                          <button type="button" class="page-button" onclick={previousPlaylistPage} disabled={!hasPreviousPlaylistPage || isLoadingPlaylistPage} aria-label="播放清單上一頁"><svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m12.5 4.5-5 5.5 5 5.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" /></svg></button>
                          <button type="button" class="page-button" onclick={nextPlaylistPage} disabled={!hasNextPlaylistPage || isLoadingPlaylistPage} aria-label="播放清單下一頁"><svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.5 4.5 5 5.5-5 5.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" /></svg></button>
                        </div>
                      </div>
                    {/if}
                  </section>
                {/if}
              </div>
            {/if}
          </section>
        {:else if activeView === 'now-playing'}
          <section class="now-playing-view" aria-labelledby="now-playing-heading">
            <div class="now-playing-card">
              <div class="cover-stage" class:has-artwork={activeArtwork.status === 'ready' && activeArtwork.objectUrl !== null}>
                {#if activeArtwork.status === 'ready' && activeArtwork.objectUrl}
                  <img class="cover-stage-image" src={activeArtwork.objectUrl} alt={`${currentTrackTitle(playback?.currentTrack)} 封面`} onerror={() => artworkController.imageFailed(activeArtwork.objectUrl!)} />
                {:else}
                  <div class="cover-orbit cover-orbit-a"></div>
                  <div class="cover-orbit cover-orbit-b"></div>
                  <div class="cover-wave"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
                  <span class="cover-stage-label">MOE / LOCAL</span>
                  {#if activeArtwork.status === 'too-large'}
                    <span class="cover-fallback-message">原圖超過 32 MiB 或 64 百萬像素上限</span>
                  {:else if activeArtwork.status === 'error'}
                    <span class="cover-fallback-message">封面格式不支援或無法讀取</span>
                  {:else if activeArtwork.status === 'missing' && playback?.currentTrack}
                    <span class="cover-fallback-message">沒有可用封面</span>
                  {/if}
                {/if}
              </div>
              <div class="now-playing-copy">
                <p class="section-kicker">NOW PLAYING</p>
                <h2 id="now-playing-heading">{currentTrackTitle(playback?.currentTrack)}</h2>
                <p class="now-playing-artist">{currentTrackArtist(playback?.currentTrack)}</p>
                <div class="play-state-chip" class:ready={playbackReady}>
                  <span class="status-dot" aria-hidden="true"></span>
                  {playbackReady ? playbackStateLabel(playback?.state) : '播放引擎尚未就緒'}
                </div>
                {#if playbackError || playback?.lastError}<p class="error-note" role="status">{playbackError ?? playback?.lastError}</p>{/if}
                <button class="outline-button now-playing-return" type="button" onclick={() => (activeView = 'library')}>返回曲庫</button>
              </div>
            </div>
            <div class="playback-note">
              <span class="note-icon" aria-hidden="true">i</span>
              <p>{playbackReady ? '播放狀態由原生音訊服務提供。' : showCapabilityDetail(capabilities?.playback)}</p>
            </div>
          </section>
        {:else}
          <section class="settings-page" aria-labelledby="settings-heading">
            <div class="section-heading settings-heading">
              <div><p class="section-kicker">SETTINGS</p><h2 id="settings-heading">設定</h2></div>
            </div>
            <div class="settings-tabs" role="tablist" aria-label="設定分類">
              <button
                id="appearance-tab"
                class="settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'appearance'}
                aria-controls="appearance-panel"
                onclick={() => (settingsSection = 'appearance')}
              >外觀</button>
              <button
                id="sources-tab"
                class="settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'sources'}
                aria-controls="sources-panel"
                onclick={() => (settingsSection = 'sources')}
              >音樂來源</button>
            </div>

            {#if settingsSection === 'appearance'}
              <div id="appearance-panel" class="settings-panel" role="tabpanel" aria-labelledby="appearance-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>顏色</h3>
                    <p>自訂背景與主色。文字會依背景自動選擇黑色或白色，保持清楚對比。</p>
                  </div>
                  <button class="outline-button" type="button" onclick={resetThemePreferences}>恢復預設</button>
                </div>
                <div class="appearance-color-grid">
                  <label class="theme-color-control">
                    <input
                      type="color"
                      aria-label="背景色"
                      value={themePreferences.backgroundHex}
                      oninput={(event) => updateThemeColor('backgroundHex', event.currentTarget.value)}
                      onchange={saveThemePreferencesNow}
                    />
                    <span class="theme-color-copy">
                      <strong>背景色</strong>
                      <code>{themePreferences.backgroundHex}</code>
                    </span>
                  </label>
                  <label class="theme-color-control">
                    <input
                      type="color"
                      aria-label="主色"
                      value={themePreferences.accentHex}
                      oninput={(event) => updateThemeColor('accentHex', event.currentTarget.value)}
                      onchange={saveThemePreferencesNow}
                    />
                    <span class="theme-color-copy">
                      <strong>主色</strong>
                      <code>{themePreferences.accentHex}</code>
                    </span>
                  </label>
                </div>
                <div class="theme-preview" role="img" aria-label="顏色即時預覽">
                  <div class="theme-preview-copy">
                    <strong>外觀預覽</strong>
                    <small>文字會自動調整對比</small>
                  </div>
                  <span class="theme-preview-chip">主色按鈕</span>
                </div>
                <p class="theme-save-status" class:error={themeSaveState === 'error'} role="status">{themeSaveMessage}</p>
              </div>
            {:else}
              <div id="sources-panel" class="settings-source-panel" role="tabpanel" aria-labelledby="sources-tab" tabindex="0">
                <section class="settings-view" aria-labelledby="source-settings-heading">
                  <div class="section-heading settings-heading">
                    <div><p class="section-kicker">SOURCES</p><h2 id="source-settings-heading">管理音樂來源</h2></div>
                    <button class="outline-button" type="button" onclick={() => void loadSources()} disabled={!sourceSyncReady || isLoadingSources}>
                      {isLoadingSources ? '載入中' : '重新載入'}
                    </button>
                  </div>
            <div class="source-status-card">
              <div class="source-status-icon" aria-hidden="true">
                <svg viewBox="0 0 28 28" fill="none"><path d="M4.5 7.5h7l2.2 2.3h9.8v10.1a1.6 1.6 0 0 1-1.6 1.6H6.1a1.6 1.6 0 0 1-1.6-1.6V7.5Z" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" /><path d="M4.5 11h19" stroke="currentColor" stroke-width="1.5" /></svg>
              </div>
              <div class="source-status-copy"><h3>本機音樂來源</h3><p>啟動時先顯示已保存曲目，再於背景掃描來源；只有完整掃描才會確認移除項目。</p></div>
              <span class="status-pill" class:not-ready={!sourceSyncReady}>{capabilityLabel(capabilities?.sourceSync)}</span>
            </div>

            {#if capabilities?.platform === 'windows'}
              <div class="source-action-card source-folder-picker">
                <div class="source-action-heading"><strong>Windows 音樂資料夾</strong><span>本機</span></div>
                <p>使用 Windows 原生資料夾選擇器；取消時不會加入來源或開始同步。</p>
                <button class="primary-button" type="button" onclick={() => void pickWindowsFolder()} disabled={!sourceSyncReady || isUpdatingSource}>
                  {isUpdatingSource ? '處理中' : '選擇資料夾並同步'}
                </button>
              </div>
            {:else if capabilities?.platform === 'android'}
              <div class="source-action-card android-source-actions">
                <div class="source-action-heading"><strong>共享音樂</strong><span>MediaStore</span></div>
                <p>先授權讀取音樂，再選擇裝置提供的媒體儲存空間。</p>
                <div class="source-action-row source-action-buttons">
                  <button class="primary-button" type="button" onclick={() => void requestMediaStorePermission()} disabled={!sourceSyncReady || isUpdatingSource}>
                    {mediaPermissionGranted ? '重新檢查授權' : '授權並讀取共享音樂'}
                  </button>
                  {#if mediaPermissionGranted}
                    <button class="outline-button" type="button" onclick={() => void loadMediaStoreVolumes()} disabled={isLoadingVolumes || isUpdatingSource}>
                      {isLoadingVolumes ? '載入中' : '重新載入儲存空間'}
                    </button>
                  {/if}
                </div>
                {#if mediaStoreVolumes.length > 0}
                  <div class="media-volume-list" aria-label="MediaStore 儲存空間">
                    {#each mediaStoreVolumes as volume (volume.volumeName)}
                      <button class="volume-choice" type="button" onclick={() => void addMediaStoreVolume(volume)} disabled={isUpdatingSource}>
                        <span>{volume.displayName}</span><small>加入並同步</small>
                      </button>
                    {/each}
                  </div>
                {:else if mediaPermissionGranted && !isLoadingVolumes}
                  <p class="source-muted-note">目前沒有可用的共享媒體儲存空間。</p>
                {/if}
              </div>
              <div class="source-action-card android-source-actions">
                <div class="source-action-heading"><strong>選擇文件資料夾</strong><span>SAF</span></div>
                <p>使用 Android 系統文件選擇器授權資料夾；URI 與授權留在原生端管理。</p>
                <button class="outline-button" type="button" onclick={() => void pickSafSource()} disabled={!sourceSyncReady || isUpdatingSource}>
                  {isUpdatingSource ? '處理中' : '選擇資料夾並同步'}
                </button>
              </div>
            {:else}
              <div class="source-action-card"><p>{showCapabilityDetail(capabilities?.sourceSync)}</p></div>
            {/if}

            {#if sourceError}
              <div class="source-error-message" role="status">{sourceError}</div>
            {/if}
            {#if sourceSyncSummary}
              <div class="source-result-message" role="status">{sourceSyncSummary}</div>
            {/if}

            <div class="configured-sources" aria-live="polite">
              <div class="configured-sources-heading"><strong>已加入的來源</strong><span>{sources.length}</span></div>
              {#if isLoadingSources && sources.length === 0}
                <p class="source-muted-note">正在讀取來源…</p>
              {:else if sources.length === 0}
                <p class="source-muted-note">尚未加入來源。加入後會自動開始同步。</p>
              {:else}
                {#each sources as source (source.id)}
                  <div class="configured-source">
                    <div class="configured-source-copy"><strong>{source.displayName}</strong><small>{source.kind === 'androidMediaStore' ? 'Android MediaStore' : source.kind === 'androidSaf' ? 'Android 文件資料夾' : 'Windows 資料夾'}</small></div>
                    <div class="configured-source-state"><span>{sourceStateLabel(source.syncState)}</span>{#if source.errorCount > 0}<small>{source.errorCount} 個項目需要留意</small>{/if}</div>
                  </div>
                {/each}
              {/if}
            </div>
                  <div class="settings-footnote"><span>保留既有曲庫與人工資料</span><span>來源暫時離線時不會當成刪除</span></div>
                </section>
              </div>
            {/if}
          </section>
        {/if}
      </div>
    </div>
  </main>

  <footer class="player-dock" aria-label="播放控制">
    <div class="dock-track">
      <div class="dock-art">
        {#if activeArtwork.status === 'ready' && activeArtwork.objectUrl}
          <img class="dock-art-image" src={activeArtwork.objectUrl} alt="" onerror={() => artworkController.imageFailed(activeArtwork.objectUrl!)} />
        {:else}
          <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M9 17V5l9-2v12M9 17a2.8 2.8 0 1 1-2.8-2.8A2.8 2.8 0 0 1 9 17Zm9-2a2.8 2.8 0 1 1-2.8-2.8A2.8 2.8 0 0 1 18 15Z" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" /></svg>
        {/if}
      </div>
      <div class="dock-track-copy">
        <strong>{currentTrackTitle(playback?.currentTrack)}</strong>
        <span>{currentTrackArtist(playback?.currentTrack)}</span>
      </div>
      <button class="dock-favorite" type="button" aria-label="收藏曲目" title="收藏功能尚未接通" disabled>
        <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="M10 16.4s-6.4-3.8-6.4-8.1a3.4 3.4 0 0 1 6.4-1.5 3.4 3.4 0 0 1 6.4 1.5c0 4.3-6.4 8.1-6.4 8.1Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" /></svg>
      </button>
    </div>

    <div class="dock-center">
      <div class="dock-controls">
        <button class="control-button secondary-control" type="button" aria-label="隨機播放" title={showCapabilityDetail(capabilities?.playbackModes)} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.shuffle} onclick={toggleShuffle}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M16 4h3v3M19 4l-6.5 7.2M5 6h2.2c1 0 1.9.5 2.5 1.2l5.6 7.6c.5.7 1.4 1.2 2.4 1.2H19m-3-3 3 3-3 3M5 16h2.2c.8 0 1.6-.4 2.1-1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="control-button" type="button" aria-label="上一首" title={playback?.canPrevious ? '播放佇列上一首' : showCapabilityDetail(capabilities?.playbackNavigation)} disabled={!playbackNavigationReady || !playback?.canPrevious || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_previous')}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M6 5v12m11-11-8 5 8 5V6Z" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="play-button" type="button" aria-label={playback?.isPlaying ? '暫停' : '播放'} title={showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} onclick={togglePlayback}>
          {#if playback?.isPlaying}
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M8 6v12M16 6v12" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" /></svg>
          {:else}
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m8.5 5.8 10 6.2-10 6.2V5.8Z" fill="currentColor" /></svg>
          {/if}
        </button>
        <button class="control-button" type="button" aria-label="下一首" title={playback?.canNext ? '播放佇列下一首' : showCapabilityDetail(capabilities?.playbackNavigation)} disabled={!playbackNavigationReady || !playback?.canNext || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_next')}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M16 5v12M5 6l8 5-8 5V6Z" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="control-button secondary-control" type="button" aria-label="循環播放" title={showCapabilityDetail(capabilities?.playbackModes)} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.repeatMode !== 'off' && playback?.repeatMode !== undefined} onclick={setRepeatMode}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M17 8h2.5l-3-3-3 3H16v6a3 3 0 0 1-3 3h-1M5 14H2.5l3 3 3-3H6V8a3 3 0 0 1 3-3h1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /><circle cx="16" cy="15" r="3" fill="var(--dock-bg)" /><path d="M16 13.4v1.7l1.1.7" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
      </div>
      <div class="progress-row">
        <span>{formatDuration(playbackSeekPositionMs)}</span>
        <input
          class="progress-slider"
          type="range"
          min="0"
          max={Math.max(1, playback?.durationMs ?? 0)}
          value={playbackSeekPositionMs}
          aria-label="播放進度"
          disabled={!playbackReady || !isPlaybackSeekableDuration(playback?.durationMs ?? null) || isSendingPlaybackCommand}
          onpointerdown={beginPlaybackSeek}
          oninput={updatePlaybackSeek}
          onchange={() => void commitPlaybackSeek()}
          onblur={() => void commitPlaybackSeek()}
        />
        <span>{formatDuration(playback?.durationMs)}</span>
      </div>
      {#if playbackError || playback?.lastError}<span class="dock-error" role="status">{playbackError ?? playback?.lastError}</span>{/if}
    </div>

    <div class="dock-volume">
      <span class="volume-state">{playbackReady ? '音量' : '播放未就緒'}</span>
      <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M4 9v4h3.3l4.2 3.4V5.6L7.3 9H4Z" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round" /><path d="M15 8a4.2 4.2 0 0 1 0 6m2.2-8a7.2 7.2 0 0 1 0 10" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" /></svg>
      <input class="volume-slider" type="range" min="0" max="1" step="0.01" value={playback?.volume ?? 0} aria-label="音量" disabled={!playbackReady || isSendingPlaybackCommand} oninput={setPlaybackVolume} />
      <span class="volume-value">{playback ? formatVolume(playback.volume) : '—'}</span>
    </div>
  </footer>
</div>
