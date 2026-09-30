<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import {
    IconAlertCircle,
    IconArrowLeft,
    IconArrowDown,
    IconArrowUp,
    IconArrowsShuffle,
    IconCheck,
    IconChevronRight,
    IconFilter,
    IconFolder,
    IconHeart,
    IconLibrary,
    IconList,
    IconMusic,
    IconPlayerPause,
    IconPlayerPlay,
    IconPlayerTrackNext,
    IconPlayerTrackPrev,
    IconRefresh,
    IconRepeat,
    IconRepeatOff,
    IconRepeatOnce,
    IconSearch,
    IconSettings,
    IconVolume2,
  } from '@tabler/icons-svelte-runes';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import {
    getErrorText,
    getNowPlayingAppearancePreferences,
    getTrackArtworkBytes,
    invokeCommand,
    isReady,
    setNowPlayingAppearancePreferences,
    type LibrarySyncFinishedEvent,
    type LibrarySyncProgressEvent,
    type LibrarySource,
    type MediaStoreVolumeOption,
    type FeatureCapability,
    type LyricsPreferences,
    type NowPlayingLayout,
    type NowPlayingAppearancePreferences,
    type PlaylistEntrySummary,
    type PlaylistSummary,
    type PlaybackSnapshot,
    type PlaybackQueueSource,
    type RuntimeCapabilities,
    type ThemePreferences,
    type TrackListColumnPreference,
    type TrackSummary,
  } from './lib/ipc';
  import {
    applyTheme,
    DEFAULT_THEME_PREFERENCES,
    isHexColor,
    normalizeThemePreferences,
  } from './lib/theme';
  import { formatVolume } from './lib/format';
  import hiResBadgeUrl from './assets/hi-res-badge.png';
  import {
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    TRACK_COLUMN_DEFINITIONS,
    formatTrackColumnValue,
    isHiResTrack,
    moveTrackColumn,
    normalizeTrackColumnPreferences,
    setTrackColumnVisibility,
  } from './lib/track-columns.js';
  import { effectivePlaybackDurationMs } from './lib/playback-duration';
  import TrackList from './lib/TrackList.svelte';
  import PlaylistEntryList from './lib/PlaylistEntryList.svelte';
  import PlaylistTree from './lib/PlaylistTree.svelte';
  import PlaybackQueueList from './lib/PlaybackQueueList.svelte';
  import NowPlayingArrangement from './lib/NowPlayingArrangement.svelte';
  import NowPlayingLayoutSwitch from './lib/NowPlayingLayoutSwitch.svelte';
  import PlaybackProgress from './lib/PlaybackProgress.svelte';
  import LyricsView from './lib/LyricsView.svelte';
  import {
    createActiveTrackArtworkController,
    type ActiveArtworkState,
  } from './lib/active-track-artwork';
  import { createVolumeCommandQueue } from './lib/volume-command-queue';
  import {
    createNowPlayingAppearanceWriter,
    DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES,
    normalizeNowPlayingAppearancePreferences,
  } from './lib/now-playing-appearance.js';
  import {
    DEFAULT_LYRICS_PREFERENCES,
    normalizeLyricsPreferences,
  } from './lib/lyrics-preferences.js';

  type View = 'library' | 'playlists' | 'queue' | 'settings';
  type SettingsSection = 'appearance' | 'track-columns' | 'now-playing' | 'lyrics' | 'sources';
  type SyncProgressViewState = {
    runId: string;
    sourceCount: number;
    sources: Record<string, LibrarySyncProgressEvent>;
    active: boolean;
    summary: string | null;
    error: string | null;
  };


  let activeView = $state<View>('library');
  let nowPlayingReturnView = $state<View>('library');
  let isNowPlayingMounted = $state(false);
  let isNowPlayingOpen = $state(false);
  let nowPlayingBackButton = $state<HTMLButtonElement | undefined>(undefined);
  let dockArtworkButton = $state<HTMLButtonElement | undefined>(undefined);
  let settingsSection = $state<SettingsSection>('appearance');
  let trackColumnPreferences = $state<TrackListColumnPreference[]>(
    normalizeTrackColumnPreferences(DEFAULT_TRACK_COLUMN_PREFERENCES),
  );
  let trackColumnSettingsState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let trackColumnSettingsError = $state<string | null>(null);
  let nowPlayingLayout = $state<NowPlayingLayout>('a');
  let nowPlayingLayoutState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let nowPlayingLayoutError = $state<string | null>(null);
  let nowPlayingAppearancePreferences = $state<NowPlayingAppearancePreferences>(
    normalizeNowPlayingAppearancePreferences(DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES),
  );
  let nowPlayingAppearanceState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let nowPlayingAppearanceError = $state<string | null>(null);
  let lyricsPreferences = $state<LyricsPreferences>(normalizeLyricsPreferences(DEFAULT_LYRICS_PREFERENCES));
  let lyricsPreferencesState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let lyricsPreferencesError = $state<string | null>(null);
  let themePreferences = $state<ThemePreferences>({ ...DEFAULT_THEME_PREFERENCES });
  let themeSaveState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let themeSaveError = $state<string | null>(null);
  let settingsRecoveryWarning = $state<string | null>(null);
  let settingsSourceRegistryAuthoritative = $state(true);
  let capabilities = $state<RuntimeCapabilities | null>(null);
  let runtimeError = $state<string | null>(null);
  let libraryTrackCount = $state<number | null>(null);
  let libraryListRevision = $state(0);
  let playbackError = $state<string | null>(null);
  let playback = $state<PlaybackSnapshot | null>(null);
  let playbackQueueResetKey = $state(0);
  let playbackQueueCursorChangeKey = $state(0);
  let volumeDraft = $state<number | null>(null);
  let activeArtwork = $state<ActiveArtworkState>({ trackId: null, status: 'empty', objectUrl: null });
  let sources = $state<LibrarySource[]>([]);
  let sourceError = $state<string | null>(null);
  let sourceSyncSummary = $state<string | null>(null);
  let syncProgress = $state<SyncProgressViewState | null>(null);
  let mediaStoreVolumes = $state<MediaStoreVolumeOption[]>([]);
  let mediaPermissionGranted = $state<boolean | null>(null);
  let playlists = $state<PlaylistSummary[]>([]);
  let selectedPlaylistId = $state<string | null>(null);
  let playlistListRevision = $state(0);
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
  let isPlaylistOperation = $state(false);
  let isLoadingPlaybackSnapshot = false;
  let isSendingPlaybackCommand = $state(false);
  let playbackSnapshotRequestVersion = 0;
  let playbackSnapshotFence = 0;
  let isVolumePointerActive = false;
  let volumeCommandGeneration = 0;
  let volumeSettledGeneration = 0;
  let confirmedVolume = $state<number | null>(null);
  let themeSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let themeSaveQueue: Promise<void> = Promise.resolve();
  let themeRevision = 0;
  let trackColumnSettingsRevision = 0;
  let nowPlayingLayoutRevision = 0;
  let nowPlayingAppearanceRevision = 0;
  let lyricsPreferencesRevision = 0;
  let trackColumnSettingsQueue: Promise<void> = Promise.resolve();
  let nowPlayingLayoutQueue: Promise<void> = Promise.resolve();
  let lyricsPreferencesQueue: Promise<void> = Promise.resolve();
  let lyricsPreferencesSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let playbackPollTimer: ReturnType<typeof setInterval> | undefined;
  let playlistListRequestVersion = 0;
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

  const volumeCommandQueue = createVolumeCommandQueue({
    send: sendPlaybackVolumeCommand,
    onError(error) {
      playbackError = getErrorText(error);
    },
    onIdle() {
      if (!isVolumePointerActive) volumeDraft = null;
    },
  });

  const nowPlayingAppearanceWriter = createNowPlayingAppearanceWriter({
    write: setNowPlayingAppearancePreferences,
    onState(state) {
      nowPlayingAppearanceState = state;
    },
    onSaved(saved) {
      nowPlayingAppearancePreferences = saved;
      nowPlayingAppearanceState = 'saved';
      nowPlayingAppearanceError = null;
    },
    onError(error) {
      nowPlayingAppearanceState = 'error';
      nowPlayingAppearanceError = `無法保存正在播放外觀：${getErrorText(error)}`;
    },
  });

  const libraryReady = $derived(isReady(capabilities?.library));
  const sourceSyncReady = $derived(isReady(capabilities?.sourceSync));
  const playbackReady = $derived(isReady(capabilities?.playback));
  const playbackNavigationReady = $derived(isReady(capabilities?.playbackNavigation));
  const playbackModesReady = $derived(isReady(capabilities?.playbackModes));
  const playbackDurationMs = $derived(effectivePlaybackDurationMs(
    playback?.durationMs,
    playback?.currentTrack?.durationMs,
  ));
  const playlistExchangeReady = $derived(isReady(capabilities?.playlistExchange));
  const selectedPlaylist = $derived(
    playlists.find((playlist) => playlist.id === selectedPlaylistId) ?? null,
  );
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
      trackColumnSettingsState = 'preview';
      nowPlayingLayoutState = 'preview';
      nowPlayingAppearanceState = 'preview';
      lyricsPreferencesState = 'preview';
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
      await Promise.all([
        loadCapabilities(),
        loadThemePreferences(),
        loadSettingsRecoveryWarning(),
        loadTrackColumnSettings(),
        loadNowPlayingLayout(),
        loadNowPlayingAppearancePreferences(),
        loadLyricsPreferences(),
      ]);
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
    volumeCommandQueue.dispose();
    nowPlayingAppearanceWriter.invalidate();
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    if (lyricsPreferencesSaveTimer !== undefined) {
      clearTimeout(lyricsPreferencesSaveTimer);
      lyricsPreferencesSaveTimer = undefined;
      if (isTauri()) queueLyricsPreferencesSave(lyricsPreferencesRevision);
    }
    themeRevision += 1;
    if (playbackPollTimer !== undefined) clearInterval(playbackPollTimer);
    unlistenSyncProgress?.();
    unlistenSyncFinished?.();
    playlistListRequestVersion += 1;
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

  async function loadTrackColumnSettings(): Promise<void> {
    const revision = trackColumnSettingsRevision;
    try {
      const stored = await invokeCommand('settings_get_track_list_columns', {});
      if (revision !== trackColumnSettingsRevision) return;
      trackColumnPreferences = normalizeTrackColumnPreferences(stored.columns);
      trackColumnSettingsState = 'saved';
      trackColumnSettingsError = null;
    } catch (error) {
      if (revision !== trackColumnSettingsRevision) return;
      trackColumnSettingsState = 'error';
      trackColumnSettingsError = `無法讀取曲目欄位設定：${getErrorText(error)}`;
    }
  }

  function updateTrackColumnSettings(next: TrackListColumnPreference[]): void {
    trackColumnPreferences = normalizeTrackColumnPreferences(next);
    trackColumnSettingsError = null;
    const revision = ++trackColumnSettingsRevision;
    if (!isTauri()) {
      trackColumnSettingsState = 'preview';
      trackColumnSettingsError = '瀏覽器預覽不會保存曲目欄位設定。';
      return;
    }

    trackColumnSettingsState = 'saving';
    trackColumnSettingsQueue = trackColumnSettingsQueue.catch(() => undefined).then(async () => {
      if (revision !== trackColumnSettingsRevision) return;
      try {
        const saved = await invokeCommand('settings_set_track_list_columns', {
          preferences: { columns: trackColumnPreferences.map((column) => ({ ...column })) },
        });
        if (revision !== trackColumnSettingsRevision) return;
        trackColumnPreferences = normalizeTrackColumnPreferences(saved.columns);
        trackColumnSettingsState = 'saved';
        trackColumnSettingsError = null;
      } catch (error) {
        if (revision !== trackColumnSettingsRevision) return;
        trackColumnSettingsState = 'error';
        trackColumnSettingsError = `無法保存曲目欄位設定：${getErrorText(error)}`;
      }
    });
  }

  function setTrackColumnVisible(id: TrackListColumnPreference['id'], visible: boolean): void {
    updateTrackColumnSettings(setTrackColumnVisibility(trackColumnPreferences, id, visible));
  }

  function moveConfiguredTrackColumn(id: TrackListColumnPreference['id'], direction: 'up' | 'down'): void {
    updateTrackColumnSettings(moveTrackColumn(trackColumnPreferences, id, direction));
  }

  async function loadNowPlayingLayout(): Promise<void> {
    const revision = nowPlayingLayoutRevision;
    try {
      const stored = await invokeCommand('settings_get_now_playing_layout', {});
      if (revision !== nowPlayingLayoutRevision) return;
      nowPlayingLayout = stored === 'b' ? 'b' : 'a';
      nowPlayingLayoutState = 'saved';
      nowPlayingLayoutError = null;
    } catch (error) {
      if (revision !== nowPlayingLayoutRevision) return;
      nowPlayingLayoutState = 'error';
      nowPlayingLayoutError = `無法讀取正在播放版面設定：${getErrorText(error)}`;
    }
  }

  function setNowPlayingLayout(layout: NowPlayingLayout): void {
    nowPlayingLayout = layout;
    nowPlayingLayoutError = null;
    const revision = ++nowPlayingLayoutRevision;
    if (!isTauri()) {
      nowPlayingLayoutState = 'preview';
      nowPlayingLayoutError = '瀏覽器預覽不會保存正在播放版面設定。';
      return;
    }

    nowPlayingLayoutState = 'saving';
    nowPlayingLayoutQueue = nowPlayingLayoutQueue.catch(() => undefined).then(async () => {
      if (revision !== nowPlayingLayoutRevision) return;
      try {
        const saved = await invokeCommand('settings_set_now_playing_layout', { layout });
        if (revision !== nowPlayingLayoutRevision) return;
        nowPlayingLayout = saved;
        nowPlayingLayoutState = 'saved';
        nowPlayingLayoutError = null;
      } catch (error) {
        if (revision !== nowPlayingLayoutRevision) return;
        nowPlayingLayoutState = 'error';
        nowPlayingLayoutError = `無法保存正在播放版面設定：${getErrorText(error)}`;
      }
    });
  }

  async function loadNowPlayingAppearancePreferences(): Promise<void> {
    const revision = nowPlayingAppearanceRevision;
    try {
      const stored = await getNowPlayingAppearancePreferences();
      if (revision !== nowPlayingAppearanceRevision) return;
      nowPlayingAppearancePreferences = normalizeNowPlayingAppearancePreferences(stored);
      nowPlayingAppearanceState = 'saved';
      nowPlayingAppearanceError = null;
    } catch (error) {
      if (revision !== nowPlayingAppearanceRevision) return;
      nowPlayingAppearanceState = 'error';
      nowPlayingAppearanceError = `無法讀取正在播放外觀：${getErrorText(error)}`;
    }
  }

  function updateNowPlayingAppearancePreferences(
    patch: Partial<NowPlayingAppearancePreferences>,
    immediate = false,
  ): void {
    nowPlayingAppearancePreferences = normalizeNowPlayingAppearancePreferences({
      ...nowPlayingAppearancePreferences,
      ...patch,
    });
    nowPlayingAppearanceError = null;
    nowPlayingAppearanceRevision += 1;
    if (!isTauri()) {
      nowPlayingAppearanceState = 'preview';
      nowPlayingAppearanceError = '瀏覽器預覽不會保存正在播放外觀設定。';
      return;
    }
    nowPlayingAppearanceWriter.schedule(nowPlayingAppearancePreferences, immediate);
  }

  async function loadLyricsPreferences(): Promise<void> {
    const revision = lyricsPreferencesRevision;
    try {
      const stored = await invokeCommand('settings_get_lyrics_preferences', {});
      if (revision !== lyricsPreferencesRevision) return;
      lyricsPreferences = normalizeLyricsPreferences(stored);
      lyricsPreferencesState = 'saved';
      lyricsPreferencesError = null;
    } catch (error) {
      if (revision !== lyricsPreferencesRevision) return;
      lyricsPreferencesState = 'error';
      lyricsPreferencesError = `無法讀取歌詞設定：${getErrorText(error)}`;
    }
  }

  function updateLyricsPreferences(patch: Partial<LyricsPreferences>, immediate = false): void {
    lyricsPreferences = normalizeLyricsPreferences({ ...lyricsPreferences, ...patch });
    lyricsPreferencesError = null;
    const revision = ++lyricsPreferencesRevision;
    if (!isTauri()) {
      lyricsPreferencesState = 'preview';
      lyricsPreferencesError = '瀏覽器預覽不會保存歌詞設定。';
      return;
    }

    lyricsPreferencesState = 'saving';
    if (lyricsPreferencesSaveTimer !== undefined) clearTimeout(lyricsPreferencesSaveTimer);
    if (immediate) {
      lyricsPreferencesSaveTimer = undefined;
      queueLyricsPreferencesSave(revision);
    } else {
      lyricsPreferencesSaveTimer = setTimeout(() => {
        lyricsPreferencesSaveTimer = undefined;
        queueLyricsPreferencesSave(revision);
      }, 220);
    }
  }

  function queueLyricsPreferencesSave(revision: number): void {
    const snapshot = { ...lyricsPreferences };
    lyricsPreferencesQueue = lyricsPreferencesQueue.catch(() => undefined).then(async () => {
      if (revision !== lyricsPreferencesRevision) return;
      try {
        const saved = await invokeCommand('settings_set_lyrics_preferences', { preferences: snapshot });
        if (revision !== lyricsPreferencesRevision) return;
        lyricsPreferences = normalizeLyricsPreferences(saved);
        lyricsPreferencesState = 'saved';
        lyricsPreferencesError = null;
      } catch (error) {
        if (revision !== lyricsPreferencesRevision) return;
        lyricsPreferencesState = 'error';
        lyricsPreferencesError = `無法保存歌詞設定：${getErrorText(error)}`;
      }
    });
  }

  async function loadSettingsRecoveryWarning(): Promise<void> {
    try {
      const [warning, authoritative] = await Promise.all([
        invokeCommand('settings_get_recovery_warning', {}),
        invokeCommand('settings_source_registry_authoritative', {})
      ]);
      settingsRecoveryWarning = warning;
      settingsSourceRegistryAuthoritative = authoritative;
    } catch (error) {
      settingsRecoveryWarning = `無法確認設定檔狀態：${getErrorText(error)}`;
      settingsSourceRegistryAuthoritative = false;
    }
  }

  async function confirmSourceRegistry(): Promise<void> {
    if (isUpdatingSource || settingsSourceRegistryAuthoritative) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      settingsSourceRegistryAuthoritative = await invokeCommand('settings_confirm_source_registry', {});
      if (settingsSourceRegistryAuthoritative) await syncLibrary();
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
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
    const requestVersion = ++playlistListRequestVersion;
    isLoadingPlaylists = true;
    playlistError = null;
    try {
      const nextPlaylists = await invokeCommand('playlist_list', {});
      if (requestVersion !== playlistListRequestVersion) return;
      playlists = nextPlaylists;
      const requestedId = preferredId ?? selectedPlaylistId;
      const nextId = playlists.some((playlist) => playlist.id === requestedId)
        ? requestedId
        : playlists[0]?.id ?? null;
      selectedPlaylistId = nextId;
      playlistListRevision += 1;
    } catch (error) {
      if (requestVersion === playlistListRequestVersion) playlistError = getErrorText(error);
    } finally {
      if (requestVersion === playlistListRequestVersion) isLoadingPlaylists = false;
    }
  }

  async function selectPlaylist(playlistId: string): Promise<void> {
    selectedPlaylistId = playlistId;
    activeView = 'playlists';
    playlistListRevision += 1;
    playlistMessage = null;
  }

  function refreshPlaylistEntries(): void {
    playlistListRevision += 1;
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
      await loadSources();
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

  async function playPlaylistEntry(entry: PlaylistEntrySummary, playlistId: string): Promise<void> {
    if (!entry.trackId || !entry.hasEnabledMapping || !playbackReady || isSendingPlaybackCommand) return;
    const queueSource: PlaybackQueueSource = {
      kind: 'playlist',
      playlistId,
      entryPosition: entry.position,
    };
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: entry.trackId!, queueSource }),
      { refreshQueueList: true },
    );
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

  async function setSourceEnabled(source: LibrarySource, enabled: boolean): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      sources = await invokeCommand('library_set_source_enabled', { sourceId: source.id, enabled });
      if (enabled) await syncLibrary();
    } catch (error) {
      sourceError = getErrorText(error);
      await loadSources();
    } finally {
      isUpdatingSource = false;
    }
  }

  async function removeSource(source: LibrarySource): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      sources = await invokeCommand('library_remove_source', { sourceId: source.id });
      sourceSyncSummary = source.kind === 'playlist_file'
        ? '已移除播放清單檔案同步來源；原有播放清單內容仍會保留。'
        : '已移除音樂來源；曲庫資料仍會保留，之後不再同步此位置。';
    } catch (error) {
      sourceError = getErrorText(error);
      await loadSources();
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

  function sourceKindLabel(kind: string): string {
    switch (kind) {
      case 'playlist_file': return '播放清單檔案';
      case 'androidMediaStore':
      case 'android_media_store': return 'Android MediaStore';
      case 'androidSaf':
      case 'android_saf': return 'Android 文件資料夾';
      default: return 'Windows 音樂資料夾';
    }
  }

  async function loadPlaybackSnapshot(clearError = true): Promise<void> {
    if (!playbackReady || isLoadingPlaybackSnapshot) return;
    const requestVersion = ++playbackSnapshotRequestVersion;
    const volumeGenerationAtRequest = volumeSettledGeneration;
    isLoadingPlaybackSnapshot = true;
    if (clearError) playbackError = null;
    try {
      applyPlaybackSnapshot(
        await invokeCommand('playback_get_snapshot', {}),
        requestVersion,
        volumeGenerationAtRequest,
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
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: track.id, queueSource }),
      { refreshQueueList: true },
    );
  }

  async function openNowPlaying(): Promise<void> {
    if (!playback?.currentTrack || isNowPlayingOpen) return;
    nowPlayingReturnView = activeView;
    isNowPlayingMounted = true;
    isNowPlayingOpen = true;
    await tick();
    nowPlayingBackButton?.focus();
  }

  async function closeNowPlaying(): Promise<void> {
    if (!isNowPlayingOpen) return;
    activeView = nowPlayingReturnView;
    isNowPlayingOpen = false;
    await tick();
    dockArtworkButton?.focus();
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
    await sendPlaybackCommand(() => invokeCommand(command, {}), {
      forceQueueCursorProbe: command === 'playback_next' || command === 'playback_previous',
    });
  }

  async function commitPlaybackSeek(positionMs: number): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    await sendPlaybackCommand(() => invokeCommand('playback_seek', { positionMs }));
  }

  function applyPlaybackSnapshot(
    next: PlaybackSnapshot,
    snapshotVersion?: number,
    volumeGenerationAtRequest?: number,
    forceQueueCursorProbe = false,
  ): void {
    if (snapshotVersion !== undefined && snapshotVersion <= playbackSnapshotFence) return;
    const volumeIsStale = volumeGenerationAtRequest !== undefined
      && volumeGenerationAtRequest < volumeSettledGeneration;
    const volumeToPreserve = playback?.volume ?? confirmedVolume;
    const resolved = volumeIsStale && volumeToPreserve !== null
      ? { ...next, volume: volumeToPreserve }
      : next;
    const previous = playback;
    const previousTrackId = previous?.currentTrack?.id ?? null;
    const nextTrackId = resolved.currentTrack?.id ?? null;
    const trackChanged = nextTrackId !== null && previousTrackId !== null && nextTrackId !== previousTrackId;
    const positionReset = nextTrackId !== null
      && previousTrackId === nextTrackId
      && previous !== null
      && previous.positionMs > resolved.positionMs + 50
      && resolved.positionMs < 1500;
    playback = resolved;
    confirmedVolume = resolved.volume;
    if (trackChanged || positionReset || forceQueueCursorProbe) playbackQueueCursorChangeKey += 1;
  }

  function setPlaybackVolume(event: Event): void {
    if (!playbackReady) return;
    const volume = Number((event.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(volume)) return;
    volumeDraft = volume;
    volumeCommandQueue.enqueue(volume);
  }

  function startVolumeInteraction(): void {
    isVolumePointerActive = true;
  }

  function finishVolumeInteraction(): void {
    if (!isVolumePointerActive) return;
    isVolumePointerActive = false;
    volumeCommandQueue.flush();
  }

  function finishVolumeChange(): void {
    volumeCommandQueue.flush();
  }

  async function sendPlaybackVolumeCommand(volume: number): Promise<void> {
    if (!playbackReady) return;
    const generation = ++volumeCommandGeneration;
    playbackError = null;
    try {
      const next = await invokeCommand('playback_set_volume', { volume });
      if (generation === volumeCommandGeneration) {
        if (playback) playback = { ...playback, volume: next.volume };
        confirmedVolume = next.volume;
      }
    } catch (error) {
      if (generation === volumeCommandGeneration) playbackError = getErrorText(error);
      throw error;
    } finally {
      volumeSettledGeneration = Math.max(volumeSettledGeneration, generation);
    }
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
    await sendPlaybackCommand(
      () => invokeCommand('playback_set_shuffle', { enabled: !playback?.shuffle }),
      { refreshQueueList: true },
    );
  }

  interface PlaybackCommandOptions {
    refreshQueueList?: boolean;
    forceQueueCursorProbe?: boolean;
  }

  async function sendPlaybackCommand(
    request: () => Promise<PlaybackSnapshot>,
    options: PlaybackCommandOptions = {},
  ): Promise<void> {
    playbackSnapshotFence = ++playbackSnapshotRequestVersion;
    isSendingPlaybackCommand = true;
    playbackError = null;
    try {
      applyPlaybackSnapshot(await request(), undefined, undefined, options.forceQueueCursorProbe);
      if (options.refreshQueueList) playbackQueueResetKey += 1;
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

  function currentTrackAlbum(track: TrackSummary | null | undefined): string {
    return track?.album?.trim() || '—';
  }

</script>

<svelte:window onpointerup={finishVolumeInteraction} onpointercancel={finishVolumeInteraction} />

<div
  class="app-shell"
  class:has-now-playing-backdrop={isNowPlayingOpen}
  data-active-view={activeView}
  style={`--np-background-blur: ${nowPlayingAppearancePreferences.backgroundBlurPx}px; --np-surface-alpha: ${(100 - nowPlayingAppearancePreferences.surfaceTransparencyPercent) / 100};`}
>
  {#if isNowPlayingOpen}
    <div class="now-playing-backdrop" data-testid="now-playing-backdrop" aria-hidden="true">
      {#if activeArtwork.status === 'ready' && activeArtwork.objectUrl}
        <img src={activeArtwork.objectUrl} alt="" onerror={() => artworkController.imageFailed(activeArtwork.objectUrl!)} />
      {/if}
    </div>
  {/if}
  <aside class="sidebar" aria-label="主要導覽" inert={isNowPlayingOpen}>
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
        <IconLibrary size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">曲庫</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'queue'}
        aria-current={activeView === 'queue' ? 'page' : undefined}
        onclick={() => (activeView = 'queue')}
      >
        <IconList size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">播放佇列</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'settings'}
        aria-current={activeView === 'settings' ? 'page' : undefined}
        onclick={() => (activeView = 'settings')}
      >
        <IconSettings size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">設定</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
    </nav>

    <div class="sidebar-playlist-tree-host">
      <PlaylistTree
        {playlists}
        {selectedPlaylistId}
        active={activeView === 'playlists'}
        onOpen={() => (activeView = 'playlists')}
        onSelect={selectPlaylist}
      />
    </div>

    <div class="sidebar-rule"></div>
    <div class="sidebar-source">
      <span class="source-mini-icon" aria-hidden="true">
        <IconFolder size={18} stroke={1.6} aria-hidden="true" />
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

  <main class="workspace" inert={isNowPlayingOpen}>
    <header class="topbar">
      <div class="breadcrumbs"><span>MOEMUSIC</span><span class="breadcrumb-slash">/</span><strong>{activeView === 'library' ? 'LIBRARY' : activeView === 'playlists' ? 'PLAYLISTS' : activeView === 'queue' ? 'QUEUE' : 'SOURCES'}</strong></div>
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
      <div class="page-content" class:wide-list-page={activeView === 'playlists' || activeView === 'queue'}>
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
                  <IconRefresh size={15} stroke={1.6} class={isSyncing ? 'spin' : ''} aria-hidden="true" />
                  <span>{isSyncing ? '同步中' : '重新整理'}</span>
                </button>
              </div>
            </div>

            <div class="library-toolbar">
              <label class="search-field">
                <IconSearch size={17} stroke={1.6} aria-hidden="true" />
                <input
                  type="search"
                  aria-label="搜尋曲庫"
                  placeholder="搜尋歌名、演出者或專輯"
                  value={query}
                  oninput={handleSearchInput}
                  disabled={!libraryReady}
                />
                <kbd aria-hidden="true"><IconSearch size={11} stroke={1.8} aria-hidden="true" /></kbd>
              </label>
              <button class="filter-button" type="button" disabled title="篩選功能尚未接通">
                <IconFilter size={15} stroke={1.6} aria-hidden="true" />
                <span>篩選</span>
              </button>
            </div>

            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true">
                  <IconFolder size={24} stroke={1.6} aria-hidden="true" />
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
                columns={trackColumnPreferences}
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
                <span class="message-mark"><IconAlertCircle size={16} stroke={1.8} aria-hidden="true" /></span>
                <div><strong>播放清單操作失敗</strong><p>{playlistError}</p></div>
                <button class="text-button" type="button" onclick={() => { if (selectedPlaylistId) refreshPlaylistEntries(); else void loadPlaylists(); }}>再試一次</button>
              </div>
            {/if}
            {#if playlistMessage}
              <div class="source-result-message playlist-result-message" role="status">{playlistMessage}</div>
            {/if}

            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true"><IconMusic size={24} stroke={1.6} aria-hidden="true" /></div>
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
                {#if selectedPlaylist}
                  <section class="playlist-detail" aria-labelledby="selected-playlist-heading">
                    <div class="playlist-detail-heading">
                      <div class="playlist-detail-title">
                        <span class="playlist-detail-artwork" aria-hidden="true"><IconMusic size={27} stroke={1.5} aria-hidden="true" /></span>
                        <div>
                          <p class="section-kicker">PLAYLIST</p>
                          <h3 id="selected-playlist-heading">{selectedPlaylist.name.trim() || '未命名播放清單'}</h3>
                          <p class="playlist-detail-count">{selectedPlaylist.entryCount.toLocaleString()} 個項目</p>
                        </div>
                      </div>
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

                    {#key selectedPlaylist.id}
                      <PlaylistEntryList
                        playlistId={selectedPlaylist.id}
                        resetKey={playlistListRevision}
                        {playbackReady}
                        {isSendingPlaybackCommand}
                        columns={trackColumnPreferences}
                        onPlay={playPlaylistEntry}
                      />
                    {/key}
                  </section>
                {/if}
              </div>
            {/if}
          </section>
        {:else if activeView === 'queue'}
          <section class="queue-page" aria-labelledby="queue-heading">
            <div class="section-heading">
              <div>
                <p class="section-kicker">PLAYBACK QUEUE</p>
                <h2 id="queue-heading">播放佇列</h2>
                <p class="queue-current-track">
                  {#if playback?.currentTrack}
                    目前播放：{currentTrackTitle(playback.currentTrack)}
                  {:else}
                    尚未選擇曲目
                  {/if}
                </p>
              </div>
            </div>
            {#if !playbackReady}
              <div class="not-ready-panel" role="status">
                <div class="not-ready-icon" aria-hidden="true"><IconList size={22} stroke={1.6} /></div>
                <div class="not-ready-copy"><span class="state-label">PLAYBACK</span><h3>播放佇列服務尚未就緒</h3><p>{showCapabilityDetail(capabilities?.playback)}</p></div>
              </div>
            {:else}
              <PlaybackQueueList
                resetKey={playbackQueueResetKey}
                cursorChangeKey={playbackQueueCursorChangeKey}
                columns={trackColumnPreferences}
              />
            {/if}
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
                id="track-columns-tab"
                class="settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'track-columns'}
                aria-controls="track-columns-panel"
                onclick={() => (settingsSection = 'track-columns')}
              >曲目欄位</button>
              <button
                id="now-playing-tab"
                class="settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'now-playing'}
                aria-controls="now-playing-layout-panel"
                onclick={() => (settingsSection = 'now-playing')}
              >正在播放</button>
              <button
                id="lyrics-tab"
                class="settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'lyrics'}
                aria-controls="lyrics-panel"
                onclick={() => (settingsSection = 'lyrics')}
              >歌詞</button>
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

            {#if settingsRecoveryWarning}
              <div class="source-error-message" role="status">
                設定檔修復通知：{settingsRecoveryWarning}
                {#if !settingsSourceRegistryAuthoritative}
                  <p>為保留原有曲庫，來源同步已暫停。請重新登記全部音樂資料夾與播放清單檔案，再確認恢復同步。</p>
                  <button class="text-button" type="button" disabled={isUpdatingSource} onclick={() => void confirmSourceRegistry()}>我已確認來源清單，恢復同步</button>
                {/if}
              </div>
            {/if}

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
            {:else if settingsSection === 'track-columns'}
              <div id="track-columns-panel" class="settings-panel" role="tabpanel" aria-labelledby="track-columns-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>曲庫與播放清單欄位</h3>
                    <p>兩種列表共用欄位順序與顯示設定；序號與播放操作固定在兩側。</p>
                  </div>
                </div>
                <ol class="track-column-settings" aria-label="曲目資訊欄位設定">
                  {#each trackColumnPreferences as preference, index (preference.id)}
                    {@const definition = TRACK_COLUMN_DEFINITIONS.find((column) => column.id === preference.id)!}
                    <li class="track-column-setting-row">
                      <label class="track-column-setting-label">
                        <input
                          type="checkbox"
                          checked={preference.visible}
                          aria-label={`顯示${definition.label}欄`}
                          onchange={(event) => setTrackColumnVisible(preference.id, event.currentTarget.checked)}
                        />
                        <span>{definition.label}</span>
                      </label>
                      <div class="track-column-order-actions">
                        <span aria-label={`第 ${index + 1} 欄`}>{index + 1}</span>
                        <button type="button" aria-label={`${definition.label}欄上移`} title="上移欄位" disabled={index === 0} onclick={() => moveConfiguredTrackColumn(preference.id, 'up')}><IconArrowUp size={16} stroke={1.7} aria-hidden="true" /></button>
                        <button type="button" aria-label={`${definition.label}欄下移`} title="下移欄位" disabled={index === trackColumnPreferences.length - 1} onclick={() => moveConfiguredTrackColumn(preference.id, 'down')}><IconArrowDown size={16} stroke={1.7} aria-hidden="true" /></button>
                      </div>
                    </li>
                  {/each}
                </ol>
                <p class="settings-preference-status" class:error={trackColumnSettingsState === 'error'} role="status">
                  {trackColumnSettingsError ?? (trackColumnSettingsState === 'loading' ? '正在讀取欄位設定…' : trackColumnSettingsState === 'saving' ? '正在保存欄位設定…' : trackColumnSettingsState === 'preview' ? '瀏覽器預覽不會保存欄位設定。' : '欄位設定已保存。')}
                </p>
              </div>
            {:else if settingsSection === 'now-playing'}
              <div id="now-playing-layout-panel" class="settings-panel" role="tabpanel" aria-labelledby="now-playing-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>正在播放排列</h3>
                    <p>只調整封面與歌詞區域的排列，不會重新載入播放或歌詞狀態。窄視窗會依選項順序堆疊。</p>
                  </div>
                </div>
                <NowPlayingLayoutSwitch layout={nowPlayingLayout} onChange={setNowPlayingLayout} />
                <p class="settings-preference-status" class:error={nowPlayingLayoutState === 'error'} role="status">
                  {nowPlayingLayoutError ?? (nowPlayingLayoutState === 'loading' ? '正在讀取正在播放排列…' : nowPlayingLayoutState === 'saving' ? '正在保存排列…' : nowPlayingLayoutState === 'preview' ? '瀏覽器預覽不會保存排列。' : `排列 ${nowPlayingLayout.toUpperCase()} 已保存。`)}
                </p>
                <div class="now-playing-appearance-settings">
                  <div class="settings-panel-header">
                    <div>
                      <h3>封面背景</h3>
                      <p>只影響「正在播放」頁。調整時即時預覽；快速拖動會合併保存。</p>
                    </div>
                  </div>
                  <div
                    class="now-playing-appearance-preview"
                    style={`--preview-blur: ${nowPlayingAppearancePreferences.backgroundBlurPx}px; --preview-surface-alpha: ${(100 - nowPlayingAppearancePreferences.surfaceTransparencyPercent) / 100};`}
                    role="img"
                    aria-label={`外觀預覽：模糊 ${nowPlayingAppearancePreferences.backgroundBlurPx} 像素，元件底色透明度 ${nowPlayingAppearancePreferences.surfaceTransparencyPercent}%`}
                  >
                    <span class="now-playing-appearance-preview-surface">歌詞面板</span>
                    <span class="now-playing-appearance-preview-dock">底部播放控制</span>
                  </div>
                  <div class="lyrics-preference-grid">
                    <label class="lyrics-preference-range">
                      <span><strong>封面背景模糊</strong><output>{nowPlayingAppearancePreferences.backgroundBlurPx}px</output></span>
                      <input
                        type="range"
                        min="0"
                        max="40"
                        step="1"
                        value={nowPlayingAppearancePreferences.backgroundBlurPx}
                        aria-label="封面背景模糊程度"
                        oninput={(event) => updateNowPlayingAppearancePreferences({ backgroundBlurPx: Number(event.currentTarget.value) })}
                        onchange={(event) => updateNowPlayingAppearancePreferences({ backgroundBlurPx: Number(event.currentTarget.value) }, true)}
                      />
                    </label>
                    <label class="lyrics-preference-range">
                      <span><strong>元件底色透明度</strong><output>{nowPlayingAppearancePreferences.surfaceTransparencyPercent}%</output></span>
                      <input
                        type="range"
                        min="0"
                        max="100"
                        step="1"
                        value={nowPlayingAppearancePreferences.surfaceTransparencyPercent}
                        aria-label="元件底色透明度"
                        oninput={(event) => updateNowPlayingAppearancePreferences({ surfaceTransparencyPercent: Number(event.currentTarget.value) })}
                        onchange={(event) => updateNowPlayingAppearancePreferences({ surfaceTransparencyPercent: Number(event.currentTarget.value) }, true)}
                      />
                    </label>
                  </div>
                  <p class="settings-preference-status" class:error={nowPlayingAppearanceState === 'error'} role="status">
                    {nowPlayingAppearanceError ?? (nowPlayingAppearanceState === 'loading' ? '正在讀取正在播放外觀…' : nowPlayingAppearanceState === 'saving' ? '正在保存正在播放外觀…' : nowPlayingAppearanceState === 'preview' ? '瀏覽器預覽不會保存正在播放外觀。' : '正在播放外觀已保存。')}
                  </p>
                </div>
              </div>
            {:else if settingsSection === 'lyrics'}
              <div id="lyrics-panel" class="settings-panel" role="tabpanel" aria-labelledby="lyrics-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>歌詞顯示</h3>
                    <p>設定會套用到所有歌曲；播放頁上方的「譯」「羅」按鈕也會更新同一組偏好。</p>
                  </div>
                  <button class="outline-button" type="button" onclick={() => updateLyricsPreferences(DEFAULT_LYRICS_PREFERENCES, true)}>恢復預設</button>
                </div>
                <div class="lyrics-preference-grid">
                  <label class="lyrics-preference-toggle">
                    <input
                      type="checkbox"
                      checked={lyricsPreferences.showTranslation}
                      onchange={(event) => updateLyricsPreferences({ showTranslation: event.currentTarget.checked }, true)}
                    />
                    <span><strong>顯示譯文</strong><small>在每行原文下方顯示翻譯</small></span>
                  </label>
                  <label class="lyrics-preference-toggle">
                    <input
                      type="checkbox"
                      checked={lyricsPreferences.showRomanization}
                      onchange={(event) => updateLyricsPreferences({ showRomanization: event.currentTarget.checked }, true)}
                    />
                    <span><strong>顯示羅馬拼音</strong><small>在每行原文下方顯示拼音</small></span>
                  </label>
                  <label class="lyrics-preference-range">
                    <span><strong>非目前歌詞透明度</strong><output>{lyricsPreferences.inactiveOpacityPercent}%</output></span>
                    <input
                      type="range"
                      min="10"
                      max="100"
                      step="1"
                      value={lyricsPreferences.inactiveOpacityPercent}
                      aria-label="非目前歌詞透明度"
                      oninput={(event) => updateLyricsPreferences({ inactiveOpacityPercent: Number(event.currentTarget.value) })}
                      onchange={(event) => updateLyricsPreferences({ inactiveOpacityPercent: Number(event.currentTarget.value) }, true)}
                    />
                  </label>
                  <label class="lyrics-preference-range">
                    <span><strong>原文大小</strong><output>{lyricsPreferences.primaryFontSizePx}px</output></span>
                    <input
                      type="range"
                      min="12"
                      max="36"
                      step="1"
                      value={lyricsPreferences.primaryFontSizePx}
                      aria-label="原文字級"
                      oninput={(event) => updateLyricsPreferences({ primaryFontSizePx: Number(event.currentTarget.value) })}
                      onchange={(event) => updateLyricsPreferences({ primaryFontSizePx: Number(event.currentTarget.value) }, true)}
                    />
                  </label>
                  <label class="lyrics-preference-range">
                    <span><strong>譯文與羅馬拼音大小</strong><output>{lyricsPreferences.auxiliaryFontSizePx}px</output></span>
                    <input
                      type="range"
                      min="9"
                      max="24"
                      step="1"
                      value={lyricsPreferences.auxiliaryFontSizePx}
                      aria-label="譯文與羅馬拼音字級"
                      oninput={(event) => updateLyricsPreferences({ auxiliaryFontSizePx: Number(event.currentTarget.value) })}
                      onchange={(event) => updateLyricsPreferences({ auxiliaryFontSizePx: Number(event.currentTarget.value) }, true)}
                    />
                  </label>
                </div>
                <p class="settings-preference-status" class:error={lyricsPreferencesState === 'error'} role="status">
                  {lyricsPreferencesError ?? (lyricsPreferencesState === 'loading' ? '正在讀取歌詞設定…' : lyricsPreferencesState === 'saving' ? '正在保存歌詞設定…' : lyricsPreferencesState === 'preview' ? '瀏覽器預覽不會保存歌詞設定。' : '歌詞設定已保存。')}
                </p>
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
                <IconFolder size={24} stroke={1.6} aria-hidden="true" />
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
                    <div class="configured-source-copy"><strong>{source.displayName}</strong><small>{sourceKindLabel(source.kind)}</small><small class="configured-source-location" title={source.location}>{source.location}</small></div>
                    <div class="configured-source-state"><span>{source.enabled ? sourceStateLabel(source.syncState) : '已停用'}</span>{#if source.enabled && source.errorCount > 0}<small>{source.errorCount} 個項目需要留意</small>{/if}</div>
                    <div class="configured-source-actions">
                      <label><input type="checkbox" checked={source.enabled} disabled={isUpdatingSource} onchange={(event) => void setSourceEnabled(source, event.currentTarget.checked)} />啟用</label>
                      <button class="text-button" type="button" disabled={isUpdatingSource} onclick={() => void removeSource(source)}>移除</button>
                    </div>
                  </div>
                {/each}
              {/if}
            </div>
                  <div class="settings-footnote"><span><IconCheck size={13} stroke={2} aria-hidden="true" />保留既有曲庫與人工資料</span><span><IconCheck size={13} stroke={2} aria-hidden="true" />來源暫時離線時不會當成刪除</span></div>
                </section>
              </div>
            {/if}
          </section>
        {/if}
      </div>
    </div>
  </main>

  {#if isNowPlayingMounted}
    <section
      class="now-playing-overlay"
      class:is-open={isNowPlayingOpen}
      data-testid="now-playing-overlay"
      aria-label="正在播放"
      aria-hidden={!isNowPlayingOpen}
      inert={!isNowPlayingOpen}
    >
      <header class="topbar now-playing-overlay-topbar">
        <button
          bind:this={nowPlayingBackButton}
          class="outline-button now-playing-overlay-return"
          type="button"
          aria-label="返回播放前頁面"
          title="返回播放前頁面"
          onclick={() => void closeNowPlaying()}
        >
          <IconArrowLeft size={17} stroke={1.8} aria-hidden="true" />
          <span>返回</span>
        </button>
        <div class="breadcrumbs"><span>MOEMUSIC</span><span class="breadcrumb-slash">/</span><strong>NOW PLAYING</strong></div>
      </header>
      <div class="now-playing-overlay-body" data-testid="now-playing-overlay-body">
        <div class="now-playing-overlay-content">
          <section class="now-playing-view" aria-labelledby="now-playing-heading">
            <NowPlayingArrangement layout={nowPlayingLayout}>
              {#snippet artwork()}
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
                  <p class="now-playing-format">
                    <span>{formatTrackColumnValue('audioFormat', playback?.currentTrack ?? {}, () => '—')}</span>
                    {#if isHiResTrack(playback?.currentTrack)}<img src={hiResBadgeUrl} alt="Hi-Res" title="Hi-Res" />{/if}
                  </p>
                  <h2 id="now-playing-heading">{currentTrackTitle(playback?.currentTrack)}</h2>
                  <p class="now-playing-artist">{currentTrackArtist(playback?.currentTrack)}</p>
                  <p class="now-playing-album">{currentTrackAlbum(playback?.currentTrack)}</p>
                  {#if playbackError || playback?.lastError}<p class="error-note" role="status">{playbackError ?? playback?.lastError}</p>{/if}
                </div>
              {/snippet}
              {#snippet lyrics()}
                <LyricsView
                  trackId={playback?.currentTrack?.id ?? null}
                  positionMs={playback?.positionMs ?? 0}
                  isPlaying={playback?.isPlaying ?? false}
                  playbackState={playback?.state ?? 'empty'}
                  {lyricsPreferences}
                  onPreferencesChange={(patch) => updateLyricsPreferences(patch, true)}
                />
              {/snippet}
            </NowPlayingArrangement>
          </section>
        </div>
      </div>
    </section>
  {/if}

  <footer class="player-dock" aria-label="播放控制">
    <div class="dock-track">
      <button
        class="dock-art"
        type="button"
        bind:this={dockArtworkButton}
        aria-label={playback?.currentTrack ? isNowPlayingOpen ? '返回播放前頁面' : `開啟正在播放：${currentTrackTitle(playback.currentTrack)}` : '尚未選擇歌曲'}
        title={playback?.currentTrack ? isNowPlayingOpen ? '返回播放前頁面' : `查看正在播放：${currentTrackTitle(playback.currentTrack)}` : '尚未選擇歌曲'}
        disabled={!playback?.currentTrack}
        onclick={() => void (isNowPlayingOpen ? closeNowPlaying() : openNowPlaying())}
      >
        {#if activeArtwork.status === 'ready' && activeArtwork.objectUrl}
          <img class="dock-art-image" src={activeArtwork.objectUrl} alt="" onerror={() => artworkController.imageFailed(activeArtwork.objectUrl!)} />
        {:else}
          <IconMusic size={22} stroke={1.6} aria-hidden="true" />
        {/if}
      </button>
      <div class="dock-track-copy">
        <strong>{currentTrackTitle(playback?.currentTrack)}</strong>
        <span>{currentTrackArtist(playback?.currentTrack)}</span>
      </div>
      <button class="dock-favorite" type="button" aria-label="收藏曲目" title="收藏功能尚未接通" disabled>
        <IconHeart size={18} stroke={1.6} aria-hidden="true" />
      </button>
    </div>

    <div class="dock-center">
      <div class="dock-controls">
        <button class="control-button secondary-control" type="button" aria-label="隨機播放" title={showCapabilityDetail(capabilities?.playbackModes)} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.shuffle} onclick={toggleShuffle}>
          <IconArrowsShuffle size={20} stroke={1.7} aria-hidden="true" />
        </button>
        <button class="control-button" type="button" aria-label="上一首" title={playback?.canPrevious ? '播放佇列上一首' : showCapabilityDetail(capabilities?.playbackNavigation)} disabled={!playbackNavigationReady || !playback?.canPrevious || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_previous')}>
          <IconPlayerTrackPrev size={20} stroke={1.7} aria-hidden="true" />
        </button>
        <button class="play-button" type="button" aria-label={playback?.isPlaying ? '暫停' : '播放'} title={showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} onclick={togglePlayback}>
          {#if playback?.isPlaying}
            <IconPlayerPause size={21} stroke={1.9} aria-hidden="true" />
          {:else}
            <IconPlayerPlay size={21} stroke={1.9} aria-hidden="true" />
          {/if}
        </button>
        <button class="control-button" type="button" aria-label="下一首" title={playback?.canNext ? '播放佇列下一首' : showCapabilityDetail(capabilities?.playbackNavigation)} disabled={!playbackNavigationReady || !playback?.canNext || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_next')}>
          <IconPlayerTrackNext size={20} stroke={1.7} aria-hidden="true" />
        </button>
        <button class="control-button secondary-control" type="button" aria-label="循環播放" title={showCapabilityDetail(capabilities?.playbackModes)} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.repeatMode !== 'off' && playback?.repeatMode !== undefined} onclick={setRepeatMode}>
          {#if playback?.repeatMode === 'one'}
            <IconRepeatOnce size={20} stroke={1.7} aria-hidden="true" />
          {:else if playback?.repeatMode === 'all'}
            <IconRepeat size={20} stroke={1.7} aria-hidden="true" />
          {:else}
            <IconRepeatOff size={20} stroke={1.7} aria-hidden="true" />
          {/if}
        </button>
      </div>
      <PlaybackProgress
        positionMs={playback?.positionMs ?? 0}
        durationMs={playbackDurationMs}
        trackId={playback?.currentTrack?.id ?? null}
        canControl={playbackReady}
        isSending={isSendingPlaybackCommand}
        onSeek={commitPlaybackSeek}
      />
      {#if playbackError || playback?.lastError}<span class="dock-error" role="status">{playbackError ?? playback?.lastError}</span>{/if}
    </div>

    <div class="dock-volume">
      <span class="volume-state">{playbackReady ? '音量' : '播放未就緒'}</span>
      <IconVolume2 size={20} stroke={1.6} aria-hidden="true" />
      <input
        class="volume-slider"
        type="range"
        min="0"
        max="1"
        step="0.01"
        value={volumeDraft ?? playback?.volume ?? confirmedVolume ?? 0}
        aria-label="音量"
        disabled={!playbackReady}
        onpointerdown={startVolumeInteraction}
        oninput={setPlaybackVolume}
        onchange={finishVolumeChange}
      />
      <span class="volume-value">{playback ? formatVolume(volumeDraft ?? playback.volume) : confirmedVolume === null ? '—' : formatVolume(volumeDraft ?? confirmedVolume)}</span>
    </div>
  </footer>
</div>
