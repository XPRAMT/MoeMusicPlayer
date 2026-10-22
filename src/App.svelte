<script lang="ts">
  import { LANGUAGE_CHOICES, getLanguageChoice, tr } from './lib/i18n.js';
  import { displayUnknown } from './lib/unknown-label.js';
  import { createLyricsController } from './lib/lyrics-controller.js';
  import { defaultLyricsApi } from './lib/lyrics-api';
  import { convertTrackLyrics } from './lib/lyrics-traditional.js';
  import { buildTaskbarLyricsData, type TaskbarLyricsData } from './lib/taskbar-lyrics-data.js';
  import { adoptSavedLanguage, changeLanguage, followLanguageChange } from './lib/language-sync.js';
  import { onDestroy, onMount, setContext, tick, untrack } from 'svelte';
  import { writable } from 'svelte/store';
  import { invoke as invokeTauri } from '@tauri-apps/api/core';
  import TrackDetailsDialog from './lib/TrackDetailsDialog.svelte';
  import SpectrogramView from './lib/SpectrogramView.svelte';
  import FolderBrowserDialog from './lib/FolderBrowserDialog.svelte';
  import RemoteSourcesSettings from './lib/RemoteSourcesSettings.svelte';
  import { getPopoverHorizontalShiftPx, getVolumePopoverLeftShiftPx } from './lib/volume-popover-position.js';
  import {
    IconAlertCircle,
    IconArrowLeft,
    IconArrowsRight,
    IconArrowsShuffle,
    IconChevronRight,
    IconDisc,
    IconFolder,
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
  IconAlarm,
  IconInfoCircle,
    IconUsers,
    IconVolume,
    IconVolume2,
    IconVolume4,
    IconVolumeOff,
    IconX,
  } from '@tabler/icons-svelte-runes';
  import { emitTo, listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import {
    getErrorText,
    getNowPlayingAppearancePreferences,
    getTrackArtworkBytes,
    getMainBackgroundBytes,
    nextSlideshowBackgroundBytes,
    invokeCommand,
    isReady,
    setNowPlayingAppearancePreferences,
    type LibrarySyncFinishedEvent,
    type LibrarySyncProgressEvent,
    type LibrarySource,
    type ExcludedFolder,
    type SourceSyncResult,
    type FeatureCapability,
    type LyricsPreferences,
    type NowPlayingLayout,
    type NowPlayingAppearancePreferences,
    type PlaylistEntrySummary,
    type PlaylistSummary,
    type PlaybackQueuePageItem,
    type PlaybackSnapshot,
    type PlaybackQueueSource,
    type RuntimeCapabilities,
    type BackdropOpacityCalculation,
    type ThemePreferences,
    type TrackListColumnPreference,
    type TrackColumnWidths,
    type LibraryListSort,
    type TrackFieldFilter,
    type TrackSummary,
    type TrackPage,
    type TrackPageRequest,
    type ArtistIndexEntry,
    type AlbumIndexEntry,
  } from './lib/ipc';
  import {
    applyTheme,
    DEFAULT_THEME_PREFERENCES,
    isHexColor,
    normalizeThemePreferences,
  } from './lib/theme';
  import { getBackdropTargetPercent, resolveBackdropContrast, systemWallpaperOpacityPercent } from './lib/backdrop-contrast.js';
  import { createDecodedImageObjectUrl } from './lib/decoded-image-object-url.js';
  import { splitArtists } from './lib/artist-names.js';
  import { readDetailNavigationRoute, withDetailNavigationRoute } from './lib/detail-navigation-history.js';
  import { hideWrappedTrailingTrackSeparators } from './lib/track-field-separators.js';
  import { formatVolume } from './lib/format';
  import hiResBadgeUrl from './assets/hi-res-badge.svg';
  import { calculateArtworkFrame } from './lib/artwork-frame.js';
  import FavoritePopover from './lib/FavoritePopover.svelte';
  import {
    DEFAULT_LIBRARY_SORT,
    DEFAULT_TRACK_COLUMN_PREFERENCES,
    DEFAULT_TRACK_ROW_HEIGHT_PX,
    DEFAULT_TRACK_COLUMN_WIDTHS,
    TRACK_COLUMN_DEFINITIONS,
    displayTrackTitle,
    formatAudioFormat,
    formatTrackColumnValue,
    isHiResTrack,
    moveTrackColumn,
    moveTrackColumnTo,
    normalizeLibrarySort,
    normalizeTrackColumnPreferences,
    normalizeTrackRowHeightPx,
    normalizeTrackColumnWidths,
    setTrackColumnVisibility,
    setTrackColumnWidth,
    toggleLibrarySort,
    getFirstSortDirection,
    trackColumnDropIndexFromPointer,
  } from './lib/track-columns.js';
  import { DEFAULT_ALBUM_CARD_MAX_WIDTH_PX, MAX_ALBUM_CARD_MAX_WIDTH_PX, MIN_ALBUM_CARD_MAX_WIDTH_PX, normalizeAlbumCardMaxWidthPx } from './lib/album-index-layout.js';
  import {
    applyLivePlayCountSample,
    createLivePlayCountState,
    livePlayCountValues,
    rememberLivePlayCount,
    toLivePlayCountView,
    withLivePlayedMs,
  } from './lib/live-play-count.js';
  import { effectivePlaybackDurationMs } from './lib/playback-duration';
  import {
    correctPlaybackClock,
    estimatePlaybackPosition,
    msUntilPlaybackPosition,
    type PlaybackClock,
    type TaskbarLyricsPlayback,
  } from './lib/playback-clock';
  import { getPlaybackControlTooltips } from './lib/playback-control-tooltips.js';
  import { appTooltip } from './lib/app-tooltip.js';
  import { dockMarquee } from './lib/dock-marquee.js';
  import SourceScopeSwitch from './lib/SourceScopeSwitch.svelte';
  import { formatSyncResultLine } from './lib/sync-result-line.js';
  import { normalizeSourceScope, shouldShowSourceScopeSwitch } from './lib/source-scope.js';
  import SourceSections from './lib/SourceSections.svelte';
  import { TRACK_TEXT_OVERFLOW_CONTEXT, type TrackTextOverflowMode } from './lib/track-text-overflow-context';
  import { QUEUE_REORDER_LONG_PRESS_MS, QUEUE_REORDER_MOVE_CANCEL_PX } from './lib/queue-reorder.js';
  import { exceedsLongPressSlop, longPressDragGuard } from './lib/long-press-reorder.js';

  import {
    SHORTCUT_ACTIONS,
    addShortcutBinding,
    defaultShortcutSettings,
    normalizeShortcutSettings,
    removeShortcutBinding,
    shortcutActionFor,
    shortcutBindingLabel,
    shortcutTargetIsEditable,
    shortcutTargetIsScrollable,
  } from './lib/shortcuts.js';
  import type { ShortcutAction, ShortcutBinding, ShortcutSettings } from './lib/ipc';
  import TrackList from './lib/TrackList.svelte';
  import ArtistIndex from './lib/ArtistIndex.svelte';
  import AlbumIndex from './lib/AlbumIndex.svelte';
  import AlbumDetail from './lib/AlbumDetail.svelte';
  import { getSearchVariants } from './lib/search-variants.js';
  import PlaylistEntryList from './lib/PlaylistEntryList.svelte';
  import PlaylistTree from './lib/PlaylistTree.svelte';
  import { SLEEP_TIMER_DRAG_STEP_PX, SLEEP_TIMER_MAX_HOURS, SLEEP_TIMER_MAX_MINUTE_FIELD, SLEEP_TIMER_MAX_MINUTES, SLEEP_TIMER_WHEEL_STEP_PX, accumulateSleepTimerSteps, adjustSleepTimerField, splitSleepTimer, splitSleepTimerClock, beginSleepTimerEdit, getSleepTimerEditMinutes, stepSleepTimerEdit, SLEEP_TIMER_EDIT_IDLE_MS, formatSleepTimer, getSleepTimerDisplayMinutes, getSleepTimerRemainingMs } from './lib/sleep-timer.js';
  import type { SleepTimerSnapshot } from './lib/ipc';
  import type { AlbumTrackOrder } from './lib/ipc';
  import type { AndroidAllFilesAccessStatus, AndroidWallpaperBackdropStatus } from './lib/ipc';
  import { formatSlideshowFolderLabel } from './lib/background-source-label.js';
  import { swipeDismiss } from './lib/swipe-dismiss.js';
  import { coverZoom, coverZoomFromFractions, coverZoomToFractions, COVER_ZOOM_MIN } from './lib/cover-zoom.js';
  import {
    DSD_GAIN_MAX_DB,
    DSD_GAIN_MIN_DB,
    DSD_GAIN_STEP_DB,
    DSD_OUTPUT_RATES_HZ,
    formatDsdGain,
    formatDsdOutputRate,
    normalizeDsdSettings,
  } from './lib/dsd-settings.js';
  import type { DsdPlaybackSettings, LanguageChoice, TaskbarLyricsSettings } from './lib/ipc';
  import {
    DEFAULT_TASKBAR_LYRICS_SETTINGS,
    TASKBAR_LYRICS_FONT_SIZE_PX,
    TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT,
    normalizeTaskbarLyricsSettings,
  } from './lib/taskbar-lyrics-settings.js';
  import { NAV_PANEL_MEDIA_QUERY, NAV_PANEL_NAVIGATION_SELECTOR, reduceNavPanelOpen } from './lib/nav-panel.js';
  import PlaybackQueueList from './lib/PlaybackQueueList.svelte';
  import SyncErrorDetails from './lib/SyncErrorDetails.svelte';
  import NowPlayingArrangement from './lib/NowPlayingArrangement.svelte';
  import NowPlayingBackdrop from './lib/NowPlayingBackdrop.svelte';
  import PlaybackProgress from './lib/PlaybackProgress.svelte';
  import LyricsView from './lib/LyricsView.svelte';
  import NowPlayingQuickSettingsControls from './lib/NowPlayingQuickSettingsControls.svelte';
  import OutputDevicesSettings from './lib/OutputDevicesSettings.svelte';
  import OpacityCalculation from './lib/OpacityCalculation.svelte';
  import WindowTitlebar from './lib/WindowTitlebar.svelte';
  import AboutPage from './lib/AboutPage.svelte';
  import UpdateCenter from './lib/UpdateCenter.svelte';
  import { EMPTY_UPDATE_STATE } from './lib/update-controller';
  import type { UpdateState } from './lib/ipc';
  import { detectArtworkMimeType, nowPlayingArtworkObjectUrl } from './lib/active-track-artwork';
  import { APP_WINDOW_TITLE } from './lib/app-version';
  import brandIcon from '../icon/SilverWolfIcon.png';
  import {
    createActiveTrackArtworkController,
    type ActiveArtworkState,
  } from './lib/active-track-artwork';
  import { createVolumeCommandQueue } from './lib/volume-command-queue';
  import { getVolumeIconLevel } from './lib/volume-icon.js';
  import {
    createNowPlayingAppearanceWriter,
    DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES,
    normalizeNowPlayingAppearancePreferences,
  } from './lib/now-playing-appearance.js';
  import {
    DEFAULT_LYRICS_PREFERENCES,
    normalizeLyricsPreferences,
  } from './lib/lyrics-preferences.js';

  const trackTextOverflowStore = writable<TrackTextOverflowMode>('end');
  setContext(TRACK_TEXT_OVERFLOW_CONTEXT, trackTextOverflowStore);

  type AndroidWindowInsets = {
    safeLeftPx: number;
    safeTopPx: number;
    safeRightPx: number;
    safeBottomPx: number;
    imeBottomPx: number;
  };

  type View = 'library' | 'artists' | 'artist-detail' | 'albums' | 'album-detail' | 'playlists' | 'queue' | 'settings';
  type SettingsSection = 'appearance' | 'track-columns' | 'now-playing' | 'lyrics' | 'playback' | 'shortcuts' | 'sources' | 'about';
  type SyncProgressViewState = {
    runId: string;
    sourceCount: number;
    sources: Record<string, LibrarySyncProgressEvent>;
    active: boolean;
    summary: string | null;
    error: string | null;
  };


  let activeView = $state<View>('library');
  type ArtistSectionView = 'artists' | 'artist-detail';
  type ArtistEntry = ArtistIndexEntry;
  type AlbumEntry = AlbumIndexEntry;
  let keepArtistIndexForDetail = $state(false);
  let selectedArtist = $state<ArtistEntry | null>(null);
  let rememberedArtistView = $state<ArtistSectionView>('artists');
  let detailHistoryRestoreRevision = 0;
  let keepAlbumIndexForDetail = $state(false);
  let selectedAlbum = $state<AlbumEntry | null>(null);
  let selectedAlbumInitialCoverSources = $state<string[]>([]);
  let rememberedAlbumView = $state<'albums' | 'album-detail'>('albums');
  let nowPlayingReturnView = $state<View>('library');
  let isNowPlayingMounted = $state(false);
  let isNowPlayingOpen = $state(false);
  let isQuickSettingsOpen = $state(false);
  let isNarrowNav = $state(false);
  let isNavPanelOpen = $state(false);
  let navPanelToggle = $state<HTMLButtonElement | null>(null);

  // Sleep timer: the Rust core owns the wall-clock deadline and pauses playback on expiry,
  // so it fires even if this WebView is throttled. The UI only mirrors and adjusts it.
  let sleepTimerEndsAt = $state<number | null>(null);
  let sleepTimerNow = $state(Date.now());
  let isSleepTimerOpen = $state(false);
  // 歌曲資訊面板（ADR 0013）：播放頁頂欄資訊按鈕開啟目前播放曲目。
  let trackDetailsTrackId = $state<string | null>(null);
  // Android 沒有多視窗，頻譜圖改以全螢幕頁面呈現；桌面開獨立 Tauri 視窗。
  let androidSpectrogramTrackId = $state<string | null>(null);

  function openTrackDetails(): void {
    const id = playback?.currentTrack?.id;
    if (id) trackDetailsTrackId = id;
  }

  async function openSpectrogram(trackId: string): Promise<void> {
    if (capabilities?.platform === 'android') {
      androidSpectrogramTrackId = trackId;
      return;
    }
    try {
      await invokeTauri('spectrogram_window_open', { trackId });
    } catch (error) {
      console.error('spectrogram window', error);
    }
  }
  let sleepTimerAnchor = $state<HTMLDivElement | null>(null);
  let sleepTimerPopover = $state<HTMLDivElement | null>(null);
  let sleepTimerPopoverShiftX = $state(0);
  let sleepTimerToggle = $state<HTMLButtonElement | null>(null);
  let sleepTimerDrag: { pointerId: number; lastY: number; carry: number; field: SleepTimerField } | null = null;
  let sleepTimerWheelCarry = 0;
  // Active adjustment gesture (field locked, frozen hh:mm base); null when idle.
  let sleepTimerEdit = $state<ReturnType<typeof beginSleepTimerEdit> | null>(null);
  let sleepTimerEditIdle: number | undefined;
  let sleepTimerRequestSeq = 0;
  const sleepTimerActive = $derived(sleepTimerEndsAt !== null);
  const sleepTimerLiveMinutes = $derived(getSleepTimerDisplayMinutes(getSleepTimerRemainingMs(sleepTimerEndsAt, sleepTimerNow)));
  // While adjusting, show the gesture's own value so the live tick cannot change the other field.
  const sleepTimerMinutes = $derived(sleepTimerEdit ? getSleepTimerEditMinutes(sleepTimerEdit) : sleepTimerLiveMinutes);
  // hh:mm:ss live countdown; while adjusting, seconds read 00 (adjustments land on hh:mm:00).
  const sleepTimerParts = $derived(sleepTimerEdit ? { ...splitSleepTimer(sleepTimerMinutes), seconds: '00' } : splitSleepTimerClock(getSleepTimerRemainingMs(sleepTimerEndsAt, sleepTimerNow)));
  const sleepTimerLabel = $derived(`${sleepTimerParts.hours}:${sleepTimerParts.minutes}:${sleepTimerParts.seconds}`);

  function applySleepTimerSnapshot(snapshot: SleepTimerSnapshot): void {
    sleepTimerEndsAt = snapshot.endsAtMs ?? null;
    sleepTimerNow = Date.now();
  }

  // Android: the backend asks for the in-app folder browser (the system picker refuses a storage root).
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<number>('android-folder-picker-request', (event) => { folderPickerRequestId = event.payload; })
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });

  // Another window changed the interface language: reload to apply it.
  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<string>('app-language-changed', (event) => followLanguageChange(event.payload))
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });

  $effect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void invokeCommand('sleep_timer_get', {}).then((snapshot) => { if (!disposed) applySleepTimerSnapshot(snapshot); }).catch(() => {});
    void listen<SleepTimerSnapshot>('sleep-timer-changed', (event) => applySleepTimerSnapshot(event.payload))
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });

  $effect(() => {
    if (sleepTimerEndsAt === null) return;
    const tick = () => {
      sleepTimerNow = Date.now();
      // Rust pauses playback and emits the reset; this only keeps the label honest meanwhile.
      if (sleepTimerEndsAt !== null && sleepTimerNow >= sleepTimerEndsAt) sleepTimerEndsAt = null;
    };
    const timer = window.setInterval(tick, 250);
    document.addEventListener('visibilitychange', tick);
    return () => { window.clearInterval(timer); document.removeEventListener('visibilitychange', tick); };
  });

  $effect(() => {
    if (!isNowPlayingOpen) isSleepTimerOpen = false;
  });

  /** Remaining time becomes exactly hh:mm:00. Responses from superseded requests are ignored. */
  async function setSleepTimerMinutes(minutes: number): Promise<void> {
    const durationMs = Math.max(0, minutes) * 60_000;
    const seq = ++sleepTimerRequestSeq;
    const now = Date.now();
    sleepTimerEndsAt = durationMs > 0 ? now + durationMs : null;
    sleepTimerNow = now;
    try {
      const snapshot = await invokeCommand('sleep_timer_set', { durationMs });
      if (seq === sleepTimerRequestSeq) applySleepTimerSnapshot(snapshot);
    } catch (error) {
      playbackError = getErrorText(error);
    }
  }

  /** Start (or keep) a gesture on `field`; switching fields starts a new one from the shown value. */
  function ensureSleepTimerEdit(field: SleepTimerField): void {
    if (!sleepTimerEdit || sleepTimerEdit.field !== field) sleepTimerEdit = beginSleepTimerEdit(sleepTimerMinutes, field);
  }

  function endSleepTimerEdit(): void {
    window.clearTimeout(sleepTimerEditIdle);
    sleepTimerEditIdle = undefined;
    sleepTimerEdit = null;
  }

  /** Wheel and keys have no release event: end their gesture after a short idle. */
  function touchSleepTimerEditIdle(): void {
    window.clearTimeout(sleepTimerEditIdle);
    sleepTimerEditIdle = window.setTimeout(() => { if (!sleepTimerDrag) endSleepTimerEdit(); }, SLEEP_TIMER_EDIT_IDLE_MS);
  }

  function stepSleepTimer(field: SleepTimerField, steps: number): void {
    if (steps === 0) return;
    ensureSleepTimerEdit(field);
    const edit = sleepTimerEdit!;
    const previous = getSleepTimerEditMinutes(edit);
    const nextEdit = stepSleepTimerEdit(edit, steps);
    sleepTimerEdit = nextEdit;
    const next = getSleepTimerEditMinutes(nextEdit);
    // While running, the adjusted value becomes the new remaining time (Rust resets the deadline).
    if (next === previous && sleepTimerActive === next > 0) return;
    void setSleepTimerMinutes(next);
  }

  // Same positioning as the volume and favorite popovers: centered on the button, shifted only as
  // far as needed to keep the shared 8px window inset (getPopoverHorizontalShiftPx).
  $effect(() => {
    if (!isSleepTimerOpen) {
      sleepTimerPopoverShiftX = 0;
      return;
    }
    let active = true;
    const updateAlignment = (): void => {
      if (!active || !sleepTimerAnchor || !sleepTimerPopover) return;
      const anchor = sleepTimerAnchor.getBoundingClientRect();
      sleepTimerPopoverShiftX = getPopoverHorizontalShiftPx(anchor.left + anchor.width / 2, sleepTimerPopover.getBoundingClientRect().width, window.innerWidth);
    };
    window.addEventListener('resize', updateAlignment);
    void tick().then(updateAlignment);
    return () => {
      active = false;
      window.removeEventListener('resize', updateAlignment);
    };
  });

  function toggleSleepTimerPopover(): void {
    isSleepTimerOpen = !isSleepTimerOpen;
  }

  function closeSleepTimerPopover(restoreFocus: boolean): void {
    if (!isSleepTimerOpen) return;
    isSleepTimerOpen = false;
    endSleepTimerEdit();
    if (restoreFocus) sleepTimerToggle?.focus();
  }

  type SleepTimerField = 'hours' | 'minutes';

  /** Non-passive wheel listener so the page does not scroll while adjusting. */
  function sleepTimerWheel(node: HTMLElement, field: SleepTimerField): { destroy: () => void } {
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      const delta = event.deltaMode === 0 ? event.deltaY : Math.sign(event.deltaY) * SLEEP_TIMER_WHEEL_STEP_PX;
      const result = accumulateSleepTimerSteps(sleepTimerWheelCarry, delta, SLEEP_TIMER_WHEEL_STEP_PX);
      sleepTimerWheelCarry = result.carry;
      if (!sleepTimerDrag) touchSleepTimerEditIdle();
      stepSleepTimer(field, result.steps);
    };
    node.addEventListener('wheel', onWheel, { passive: false });
    return { destroy: () => node.removeEventListener('wheel', onWheel) };
  }

  function startSleepTimerDrag(event: PointerEvent, field: SleepTimerField): void {
    if (event.button !== 0) return;
    sleepTimerDrag = { pointerId: event.pointerId, lastY: event.clientY, carry: 0, field };
    window.clearTimeout(sleepTimerEditIdle);
    sleepTimerEdit = beginSleepTimerEdit(sleepTimerMinutes, field);
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    event.preventDefault();
  }

  function moveSleepTimerDrag(event: PointerEvent): void {
    if (!sleepTimerDrag || sleepTimerDrag.pointerId !== event.pointerId) return;
    const result = accumulateSleepTimerSteps(sleepTimerDrag.carry, event.clientY - sleepTimerDrag.lastY, SLEEP_TIMER_DRAG_STEP_PX);
    sleepTimerDrag = { ...sleepTimerDrag, lastY: event.clientY, carry: result.carry };
    stepSleepTimer(sleepTimerDrag.field, result.steps);
  }

  function endSleepTimerDrag(event: PointerEvent): void {
    if (!sleepTimerDrag || sleepTimerDrag.pointerId !== event.pointerId) return;
    sleepTimerDrag = null;
    endSleepTimerEdit();
  }

  function handleSleepTimerKeydown(event: KeyboardEvent, field: SleepTimerField): void {
    const steps = event.key === 'ArrowUp' ? 1 : event.key === 'ArrowDown' ? -1 : 0;
    if (steps === 0) return;
    event.preventDefault();
    touchSleepTimerEditIdle();
    stepSleepTimer(field, steps);
  }

  function handleWindowPointerDown(event: PointerEvent): void {
    if (isSleepTimerOpen && sleepTimerAnchor && event.target instanceof Node && !sleepTimerAnchor.contains(event.target)) closeSleepTimerPopover(false);
    handleDockPointerDown(event);
  }

  $effect(() => {
    const narrowQuery = window.matchMedia(NAV_PANEL_MEDIA_QUERY);
    const sync = () => {
      isNarrowNav = narrowQuery.matches;
      if (!narrowQuery.matches) isNavPanelOpen = reduceNavPanelOpen(isNavPanelOpen, 'widen', false);
    };
    sync();
    narrowQuery.addEventListener('change', sync);
    return () => narrowQuery.removeEventListener('change', sync);
  });

  $effect(() => {
    if (isNowPlayingOpen) isNavPanelOpen = false;
  });

  function toggleNavPanel(): void {
    isNavPanelOpen = reduceNavPanelOpen(isNavPanelOpen, 'toggle', isNarrowNav);
  }

  function closeNavPanel(event: 'escape' | 'outside' | 'navigate', restoreFocus: boolean): void {
    if (!isNavPanelOpen) return;
    isNavPanelOpen = reduceNavPanelOpen(isNavPanelOpen, event, isNarrowNav);
    if (restoreFocus) navPanelToggle?.focus();
  }

  function handleSidebarClick(event: MouseEvent): void {
    if (!isNavPanelOpen) return;
    if ((event.target as Element | null)?.closest(NAV_PANEL_NAVIGATION_SELECTOR)) closeNavPanel('navigate', false);
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (isSleepTimerOpen && event.key === 'Escape') {
      event.preventDefault();
      closeSleepTimerPopover(true);
      return;
    }
    if (isNavPanelOpen && event.key === 'Escape') {
      event.preventDefault();
      closeNavPanel('escape', true);
      return;
    }
    handleQuickSettingsKeydown(event);
  }
  let quickSettingsTrigger = $state<HTMLButtonElement | undefined>(undefined);
  let quickSettingsCloseButton = $state<HTMLButtonElement | undefined>(undefined);
  let quickSettingsDialog = $state<HTMLElement | undefined>(undefined);
  let quickSettingsScroll = $state<HTMLElement | undefined>(undefined);
  let lyricsTopbarStatus = $state<{
    source: string;
    sync: string;
    canAdjustTiming: boolean;
    timingPanelOpen: boolean;
    openManualSelection: () => void;
    toggleTimingOffset: () => void;
  } | null>(null);
  let nowPlayingBackButton = $state<HTMLButtonElement | undefined>(undefined);
  let dockArtworkButton = $state<HTMLButtonElement | undefined>(undefined);
  let librarySearchInput = $state<HTMLInputElement | undefined>(undefined);
  let settingsSection = $state<SettingsSection>('appearance');
  let updateState = $state<UpdateState>({ ...EMPTY_UPDATE_STATE });
  let updateCenter: UpdateCenter | undefined;
  let shortcutSettings = $state<ShortcutSettings>(defaultShortcutSettings());
  let capturingShortcut = $state<ShortcutAction | null>(null);
  let trackColumnPreferences = $state<TrackListColumnPreference[]>(
    normalizeTrackColumnPreferences(DEFAULT_TRACK_COLUMN_PREFERENCES),
  );
  let trackColumnWidths = $state<TrackColumnWidths>({ ...DEFAULT_TRACK_COLUMN_WIDTHS });
  let trackRowHeightPx = $state(DEFAULT_TRACK_ROW_HEIGHT_PX);
  let albumCardMaxWidthPx = $state(DEFAULT_ALBUM_CARD_MAX_WIDTH_PX);
  let groupingEnabled = $state(true);
  let trackTextOverflowMode = $state<TrackTextOverflowMode>('end');
  let librarySort = $state<LibraryListSort>({ ...DEFAULT_LIBRARY_SORT });
  let queueSort = $state<LibraryListSort | null>(null);
  let queueSortBusy = false;
  let trackColumnOrderDrag = $state<{ pointerId: number; fromIndex: number; toIndex: number } | null>(null);
  let trackColumnOrderPending: { pointerId: number; fromIndex: number; startX: number; startY: number } | null = null;
  let trackColumnOrderTimer: ReturnType<typeof setTimeout> | null = null;
  let trackColumnSettingsState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let trackColumnSettingsError = $state<string | null>(null);
  let nowPlayingLayout = $state<NowPlayingLayout>('a');
  let nowPlayingLayoutState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let nowPlayingLayoutError = $state<string | null>(null);
  let dseeHx = $state(false);
  /** WASAPI exclusive output is on (the output settings page reports it). */
  let outputExclusive = $state(false);
  let dseeHxState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let dsdSettings = $state<DsdPlaybackSettings>({ gainDb: 3, outputRateHz: 176_400, dop: false });
  let dsdSettingsState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let dsdSettingsError = $state<string | null>(null);
  let dsdSettingsRevision = 0;
  let dsdSettingsQueue: Promise<void> = Promise.resolve();
  // Each language is named in itself, whatever the interface language is.
  const LANGUAGE_NAMES: Record<string, string> = { 'zh-TW': '繁體中文', 'zh-CN': '简体中文', ja: '日本語', en: 'English' };
  let languageChoice = $state<LanguageChoice>(getLanguageChoice());
  let languageError = $state<string | null>(null);
  let taskbarLyricsSettings = $state<TaskbarLyricsSettings>({ ...DEFAULT_TASKBAR_LYRICS_SETTINGS });
  let taskbarLyricsError = $state<string | null>(null);
  let taskbarLyricsRevision = 0;
  let taskbarLyricsQueue: Promise<void> = Promise.resolve();
  let dseeHxError = $state<string | null>(null);
  let nowPlayingAppearancePreferences = $state<NowPlayingAppearancePreferences>(
    normalizeNowPlayingAppearancePreferences(DEFAULT_NOW_PLAYING_APPEARANCE_PREFERENCES),
  );
  let nowPlayingAppearanceState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let nowPlayingAppearanceError = $state<string | null>(null);
  let lyricsPreferences = $state<LyricsPreferences>(normalizeLyricsPreferences(DEFAULT_LYRICS_PREFERENCES));
  let nowPlayingLyricsRemoved = $state(false);
  let lyricsPreferencesState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let lyricsPreferencesError = $state<string | null>(null);
  let themePreferences = $state<ThemePreferences>({ ...DEFAULT_THEME_PREFERENCES });
  let mainBackgroundUrl = $state<string | null>(null);
  let mainBackgroundObjectUrl: string | null = null;
  let mainBackgroundBytesHash = '';
  let mainBackgroundReadRevision = 0;
  let slideshowNextInSeconds = $state<number | null>(null);
  let slideshowTimingScope: string | null = null;
  let slideshowStartedAt: number | null = null;
  let systemWallpaperNextInSeconds = $state<number | null>(null);
  let mainBackgroundError = $state<string | null>(null);
  let mainBackgroundBusy = $state(false);
  let mainBackgroundEffectiveOpacityPercent = $state(0);
  let nowPlayingBackgroundEffectiveOpacityPercent = $state(0);
  let mainBackgroundOpacityCalculation = $state<BackdropOpacityCalculation | null>(null);
  let nowPlayingBackgroundOpacityCalculation = $state<BackdropOpacityCalculation | null>(null);
  /** Average page+image mix for main-window text contrast; null means use page color only. */
  let mainContrastBackgroundHex = $state<string | null>(null);
  /** Average page+cover mix for Now Playing text contrast; null means use page color only. */
  let nowPlayingContrastBackgroundHex = $state<string | null>(null);
  let mainImageAverageHex: string | null = null;
  let mainImageAverageAlpha: number | null = null;
  let mainImageAverageUrl: string | null = null;
  let mainImageAverageTextColor: 'white' | 'black' | null = null;
  let nowPlayingImageAverageHex: string | null = null;
  let nowPlayingImageAverageAlpha: number | null = null;
  let nowPlayingImageAverageUrl: string | null = null;
  let nowPlayingImageAverageTextColor: 'white' | 'black' | null = null;
  let mainContrastRevision = 0;
  let nowPlayingContrastRevision = 0;
  let themeSaveState = $state<'loading' | 'saved' | 'saving' | 'error' | 'preview'>('loading');
  let themeSaveError = $state<string | null>(null);
  let settingsRecoveryWarning = $state<string | null>(null);
  let settingsSourceRegistryAuthoritative = $state(true);
  let capabilities = $state<RuntimeCapabilities | null>(null);
  // Android converts DSD to 176.4 kHz only (AudioTrack tops out at 192 kHz).
  let dsdEffectiveRateHz = $derived(capabilities?.platform === 'android' ? 176_400 : dsdSettings.outputRateHz);
  const showCustomTitlebar = $derived(capabilities?.platform === 'windows');
  let runtimeError = $state<string | null>(null);
  let libraryTrackCount = $state<number | null>(null);
  let artistTrackCount = $state<number | null>(null);
  let artistIndexCount = $state<number | null>(null);
  let albumIndexCount = $state<number | null>(null);
  let albumTrackCount = $state<number | null>(null);
  let playlistTrackCount = $state<number | null>(null);
  let queueTrackCount = $state<number | null>(null);
  let libraryListRevision = $state(0);
  /** The top-bar switch: only local songs, everything, or only remote songs (kept in settings.json). */
  let librarySourceScope = $state<'local' | 'all' | 'remote'>('all');
  async function loadLibrarySourceScope(): Promise<void> {
    try {
      librarySourceScope = normalizeSourceScope(await invokeCommand('settings_get_library_source_scope', {}));
    } catch (error) {
      console.warn('source scope', error);
    }
  }
  async function changeLibrarySourceScope(next: 'local' | 'all' | 'remote'): Promise<void> {
    const previous = librarySourceScope;
    librarySourceScope = next;
    try {
      librarySourceScope = normalizeSourceScope(await invokeCommand('settings_set_library_source_scope', { scope: next }));
      // Every list, count and grouping is read again under the new scope.
      libraryListRevision += 1;
      playlistListRevision += 1;
      playbackQueueResetKey += 1;
    } catch (error) {
      librarySourceScope = previous;
      console.warn('source scope', error);
    }
  }
  // Without both kinds of source there is no switch, so nothing may stay filtered by it.
  $effect(() => {
    if (!showSourceScopeSwitch && librarySourceScope !== 'all' && sourcesLoadedOnce) void changeLibrarySourceScope('all');
  });
  let playbackError = $state<string | null>(null);
  let playback = $state<PlaybackSnapshot | null>(null);
  // Playback clock (ADR 0028): the progress bar and the lyrics advance from it between the 500 ms samples.
  let playbackClock = $state<PlaybackClock | null>(null);
  let lastTaskbarLyricsPlayback: TaskbarLyricsPlayback | null = null;
  let playbackQueueResetKey = $state(0);
  let playbackQueueCursorChangeKey = $state(0);
  let volumeDraft = $state<number | null>(null);
  let activeArtwork = $state<ActiveArtworkState>({ trackId: null, status: 'empty', objectUrl: null });
  // Now Playing large cover: keeps the previous cover (and the with-cover layout) until the next one has decoded.
  const nowPlayingArtworkUrl = $derived(nowPlayingArtworkObjectUrl(activeArtwork));
  let coverStageElement = $state<HTMLDivElement | null>(null);
  let coverFrame = $state<{ width: number; height: number } | null>(null);
  // The top bar shows the recording date only when the track has one (no "—" placeholder).
  let nowPlayingReleaseDate = $derived(formatTrackColumnValue('year', playback?.currentTrack ?? {}, () => '—'));
  // The cover zoom of this run (scale, and pan as fractions of the cover size): it stays across tracks and closing the
  // page; a double click resets it, and an enlarged cover also returns to the original size when the track changes
  // (a shrunk cover stays). Across restarts only a shrunk cover is remembered
  // (settings.json `coverZoom.scale`, 0.5 to 1); an enlarged cover and its pan are not saved.
  let coverZoomPersisted = $state.raw<{ scale: number; offsetX: number; offsetY: number }>({ scale: 1, offsetX: 0, offsetY: 0 });
  let coverZoomScale = $derived(coverZoomPersisted.scale);
  // The base cover frame is measured at zoom 1 first; until it exists the layout ignores the saved zoom.
  let coverBaseReady = $state(false);
  let coverLayoutScale = $derived(coverBaseReady ? coverZoomScale : 1);
  let coverBaseAvailable: { width: number; height: number } | null = null;
  // 1 at original size or larger, fading to 0 as the cover shrinks to the minimum: the artwork pane's fr share.
  let coverShrinkShare = $derived(Math.min(1, Math.max(0, (coverLayoutScale - COVER_ZOOM_MIN) / (1 - COVER_ZOOM_MIN))));
  let sources = $state<LibrarySource[]>([]);
  let sourcesLoadedOnce = $state(false);
  const showSourceScopeSwitch = $derived(shouldShowSourceScopeSwitch(sources));
  /** Folders and playlists on this device; remote folders and playlists are listed with the remote servers. */
  const localSources = $derived(sources.filter((source) => !source.remoteAccountId));
  const localFolderSources = $derived(localSources.filter((source) => source.kind !== 'playlist_file'));
  const localPlaylistSources = $derived(localSources.filter((source) => source.kind === 'playlist_file'));
  const remoteSources = $derived(sources.filter((source) => Boolean(source.remoteAccountId)));
  let excludedFolders = $state<ExcludedFolder[]>([]);
  let sourceError = $state<string | null>(null);
  let sourceSyncSummary = $state<string | null>(null);
  let sourceSyncResults = $state<SourceSyncResult[]>([]);
  let syncProgress = $state<SyncProgressViewState | null>(null);
  let playlists = $state<PlaylistSummary[]>([]);
  let selectedPlaylistId = $state<string | null>(null);
  let playlistListRevision = $state(0);
  let playlistError = $state<string | null>(null);
  let query = $state('');
  let libraryFieldFilter = $state<TrackFieldFilter | null>(null);
  let selectedTrackId = $state<string | null>(null);
  let isSyncing = $state(false);
  let isLoadingSources = $state(false);
  let isUpdatingSource = $state(false);
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
  /** The speaker icon of the bottom bar follows the volume shown (also while the slider is dragged). */
  let volumeIconLevel = $derived(getVolumeIconLevel(volumeDraft ?? playback?.volume ?? confirmedVolume ?? 0));
  let volumeExpanded = $state(false);
  let volumePanelElement = $state<HTMLElement | null>(null);
  let volumePopoverElement = $state<HTMLElement | null>(null);
  let volumePopoverLeftShiftPx = $state(0);
  let themeSaveTimer: ReturnType<typeof setTimeout> | undefined;
  let themeSaveQueue: Promise<void> = Promise.resolve();
  let themeRevision = 0;
  let trackColumnSettingsRevision = 0;
  let nowPlayingLayoutRevision = 0;
  let dseeHxRevision = 0;
  let dseeHxQueue: Promise<void> = Promise.resolve();
  let nowPlayingAppearanceRevision = 0;
  let lyricsPreferencesRevision = 0;
  let trackColumnSettingsQueue: Promise<void> = Promise.resolve();
  let nowPlayingLayoutQueue: Promise<void> = Promise.resolve();
  let lyricsPreferencesQueue: Promise<void> = Promise.resolve();
  let lyricsPreferencesSaveTimer: ReturnType<typeof setTimeout> | undefined;
  const PLAYBACK_POLL_MS = 500;
  let playbackPollTimer: ReturnType<typeof setInterval> | undefined;
  let livePlayCountTimer: ReturnType<typeof setInterval> | undefined;
  let livePlayCountTracker = createLivePlayCountState();
  const livePlayCountCache = new Map<string, number>();
  let livePlayCount = $state<{ trackId: string; playedMs: number }[]>([]);
  let playlistListRequestVersion = 0;
  let capabilityRefreshInFlight = false;
  let nextCapabilityRefreshAt = 0;
  let unlistenSyncFinished: UnlistenFn | undefined;
  let unlistenSyncProgress: UnlistenFn | undefined;

  const artworkController = createActiveTrackArtworkController({
    fetchBytes: getTrackArtworkBytes,
    createObjectUrl(bytes, mimeType) {
      return createDecodedImageObjectUrl(bytes, mimeType);
    },
    revokeObjectUrl(objectUrl) {
      URL.revokeObjectURL(objectUrl);
    },
    onChange(state) {
      activeArtwork = state;
    },
    afterChange: () => tick(),
  });

  // The lyrics of the current track are loaded here, once, for everything that shows them: the Now Playing page
  // (LyricsView shows this state) and the taskbar lyrics window (it is sent the same lyrics). Two loaders used to
  // search separately and disagreed after a manual choice, a removal or a preference change.
  const lyricsController = createLyricsController({
    api: defaultLyricsApi,
    onChange(next) {
      lyricsState = next;
    },
  });
  let lyricsState = $state(lyricsController.getState());
  let taskbarLyricsData = $state<TaskbarLyricsData | null>(null);

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
      nowPlayingAppearanceError = tr("無法保存正在播放外觀：{0}", [getErrorText(error)]);
    },
  });

  const libraryReady = $derived(isReady(capabilities?.library));
  const sourceSyncReady = $derived(isReady(capabilities?.sourceSync));
  const playbackReady = $derived(isReady(capabilities?.playback));
  const playbackNavigationReady = $derived(isReady(capabilities?.playbackNavigation));
  const playbackModesReady = $derived(isReady(capabilities?.playbackModes));
  const playbackControlTooltips = $derived(getPlaybackControlTooltips(playback));
  const playbackDurationMs = $derived(effectivePlaybackDurationMs(
    playback?.durationMs,
    playback?.currentTrack?.durationMs,
  ));
  const playlistExchangeReady = $derived(isReady(capabilities?.playlistExchange));
  const selectedPlaylist = $derived(
    playlists.find((playlist) => playlist.id === selectedPlaylistId) ?? null,
  );
  const pageTitle = $derived(
    activeView === 'library'
      ? tr('我的曲庫')
      : activeView === 'artists'
        ? tr('演出者')
        : activeView === 'artist-detail'
          ? tr("演出者．{0}", [displayUnknown(selectedArtist?.displayName ?? '')])
      : activeView === 'albums'
        ? tr('專輯')
        : activeView === 'album-detail'
          ? tr("專輯．{0}", [selectedAlbum?.unmarked ? tr('其它') : selectedAlbum?.displayName ?? ''])
      : activeView === 'playlists'
        ? selectedPlaylist
          ? tr("我的播放清單．{0}", [selectedPlaylist.name.trim() || tr('未命名播放清單')])
          : tr('我的播放清單')
        : activeView === 'queue'
          ? tr('播放佇列')
          : tr('設定'),
  );
  const isListPage = $derived(
    activeView === 'library' || activeView === 'artists' || activeView === 'artist-detail' || activeView === 'albums' || activeView === 'album-detail' || activeView === 'playlists' || activeView === 'queue',
  );
  const pageTrackCountLabel = $derived.by(() => {
    if (activeView === 'library') {
      return tr("{0}首曲目", [libraryTrackCount === null ? '' : `${libraryTrackCount.toLocaleString()} `]);
    }
    if (activeView === 'artists') {
      return tr("{0}位演出者", [artistIndexCount === null ? '' : `${artistIndexCount.toLocaleString()} `]);
    }
    if (activeView === 'albums') {
      return tr("{0}張專輯", [albumIndexCount === null ? '' : `${albumIndexCount.toLocaleString()} `]);
    }
    if (activeView === 'playlists') {
      const count = playlistTrackCount ?? selectedPlaylist?.entryCount ?? null;
      if (count === null) return null;
      return tr("{0} 首曲目", [count.toLocaleString()]);
    }
    if (activeView === 'queue') {
      if (queueTrackCount === null) return null;
      return tr("{0} 首曲目", [queueTrackCount.toLocaleString()]);
    }
    if (activeView === 'artist-detail') {
      return tr("{0}首曲目", [artistTrackCount === null ? '' : `${artistTrackCount.toLocaleString()} `]);
    }
    if (activeView === 'album-detail') {
      const count = albumTrackCount ?? selectedAlbum?.trackCount ?? null;
      return tr("{0}首曲目", [count === null ? '' : `${count.toLocaleString()} `]);
    }
    return null;
  });
  $effect(() => {
    const view = activeView;
    if (view !== 'artists' && view !== 'artist-detail') keepArtistIndexForDetail = false;
    if (view !== 'albums' && view !== 'album-detail') keepAlbumIndexForDetail = false;
  });
  const syncProgressSources = $derived.by(() =>
    syncProgress
      ? Object.values(syncProgress.sources).sort((left, right) => left.sourceIndex - right.sourceIndex)
      : [],
  );
  const syncProgressDoneCount = $derived(
    syncProgressSources.filter((source) => source.outcome !== null).length,
  );
  const effectiveMainBackgroundUrl = $derived(
    themePreferences.mainBackgroundMode === 'solid'
      ? null
      : themePreferences.mainBackgroundMode === 'albumCover'
        ? activeArtwork.objectUrl ?? (activeArtwork.status === 'loading' ? mainBackgroundUrl : null)
        : mainBackgroundUrl,
  );
  const mainBackgroundMode = $derived(themePreferences.mainBackgroundMode);
  const slideshowFolder = $derived(themePreferences.slideshowFolder);
  const slideshowIntervalMinutes = $derived(themePreferences.slideshowIntervalMinutes);
  const runtimePlatform = $derived(capabilities?.platform);
  // Android cannot read wallpaper pixels (Android 13/14+ WallpaperManager restrictions), so the
  // native window shows the wallpaper behind a transparent WebView instead (ADR 0008).
  const ANDROID_WALLPAPER_BACKDROP_ENABLED = false;
  let androidWallpaperBackdrop = $state<AndroidWallpaperBackdropStatus | null>(null);
  // With All files access the static wallpaper is read as an image (like Windows); the native
  // FLAG_SHOW_WALLPAPER backdrop is only the fallback (no permission or live wallpaper).
  let androidWallpaperHasImage = $state(false);
  let allFilesAccess = $state<AndroidAllFilesAccessStatus | null>(null);
  let allFilesAccessBusy = $state(false);
  /** The panel that explains the permission is shown by itself once per app start; the button in Music Sources stays. */
  let allFilesAccessAutoPrompted = false;
  let allFilesPromptOpen = $state(false);
  /** The backend's pending request for the in-app folder browser (Android), or null. */
  let folderPickerRequestId = $state<number | null>(null);
  function finishFolderPicker(path: string | null): void {
    const requestId = folderPickerRequestId;
    folderPickerRequestId = null;
    if (requestId !== null) void invokeCommand('android_folder_picker_result', { requestId, path }).catch(() => {});
  }
  let allFilesPromptDialog = $state<HTMLDialogElement>();

  // The system wallpaper needs one more permission on some systems (HyperOS wants "read images" besides All files access).
  // The panel that explains it opens when the user has just chosen the system wallpaper and the read was refused for it.
  let wallpaperPermissionPromptOpen = $state(false);
  let wallpaperPermissionPromptDialog = $state<HTMLDialogElement>();
  /** The user chose the system wallpaper and the first read after that has not been looked at yet. */
  let wallpaperPromptWanted = false;
  $effect(() => {
    const dialog = wallpaperPermissionPromptDialog;
    if (!dialog) return;
    if (wallpaperPermissionPromptOpen && !dialog.open) dialog.showModal();
    else if (!wallpaperPermissionPromptOpen && dialog.open) dialog.close();
  });
  function closeWallpaperPermissionPrompt(): void {
    wallpaperPermissionPromptOpen = false;
  }
  async function confirmWallpaperPermissionPrompt(): Promise<void> {
    wallpaperPermissionPromptOpen = false;
    try {
      const result = await invokeCommand('android_request_media_images_permission', {});
      if (result?.granted) {
        await loadMainBackground();
        return;
      }
      // The system's prompt answers later, or the settings page was opened: look again when the permission is granted.
      for (let attempt = 0; attempt < 60; attempt += 1) {
        await new Promise((resolve) => setTimeout(resolve, 1000));
        const status = await invokeCommand('android_media_images_permission_status', {});
        if (status?.granted) {
          await loadMainBackground();
          return;
        }
      }
    } catch (error) {
      mainBackgroundError = tr("無法讀取主介面背景：{0}", [getErrorText(error)]);
    }
  }

  function closeAllFilesPrompt(): void {
    allFilesPromptOpen = false;
  }
  function confirmAllFilesPrompt(): void {
    allFilesPromptOpen = false;
    void requestAllFilesAccess();
  }
  $effect(() => {
    const dialog = allFilesPromptDialog;
    if (!dialog) return;
    if (allFilesPromptOpen && !dialog.open) dialog.showModal();
    else if (!allFilesPromptOpen && dialog.open) dialog.close();
  });

  async function refreshAllFilesAccess(): Promise<void> {
    if (!isTauri() || capabilities?.platform !== 'android') return;
    try {
      const previous = allFilesAccess?.granted;
      allFilesAccess = await invokeCommand('android_all_files_access_status', {});
      if (!allFilesAccessAutoPrompted && allFilesAccess?.supported && !allFilesAccess.granted && document.visibilityState === 'visible') {
        allFilesAccessAutoPrompted = true;
        // Explain first, right away.
        if (!allFilesAccessBusy) allFilesPromptOpen = true;
      }
      if (allFilesAccess?.granted) allFilesPromptOpen = false;
      if (previous !== undefined && previous !== allFilesAccess?.granted && themePreferences.mainBackgroundMode === 'systemWallpaper') {
        await loadMainBackground();
      }
      // Newly granted access: real-path sources (and the legacy-source migration) can sync now.
      if (previous === false && allFilesAccess?.granted === true && sourceSyncReady) {
        await loadSources();
        await syncLibrary();
      }
    } catch {
      allFilesAccess = null;
    }
  }

  async function requestAllFilesAccess(): Promise<void> {
    allFilesAccessBusy = true;
    try {
      await invokeCommand('android_request_all_files_access', {});
      // Android 10 and older answer the runtime prompt in place; Android 11+ re-checks on return.
      await refreshAllFilesAccess();
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      allFilesAccessBusy = false;
    }
  }

  $effect(() => {
    if (runtimePlatform !== 'android') return;
    // untrack: refreshAllFilesAccess reads and reassigns allFilesAccess synchronously; tracking it
    // here re-ran this effect on every result (thousands of IPC calls/s, native crash on Android).
    untrack(() => void refreshAllFilesAccess());
    const onVisible = () => {
      if (document.visibilityState === 'visible') void refreshAllFilesAccess();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => document.removeEventListener('visibilitychange', onVisible);
  });

  const androidWallpaperActive = $derived(
    runtimePlatform === 'android' && mainBackgroundMode === 'systemWallpaper' && !androidWallpaperHasImage && androidWallpaperBackdrop?.active === true,
  );
  const androidWallpaperBlurPx = $derived(themePreferences.mainBackgroundBlurPx);
  // The wallpaper pixels are not readable, so automatic opacity cannot sample them (it would
  // resolve to 0 and hide the wallpaper behind an opaque page colour). Use the worst-case bound.
  const androidWallpaperOpacityPercent = $derived(systemWallpaperOpacityPercent({
    automaticTextColor: themePreferences.colorMode === 'autoBackgroundOpacity' ? themePreferences.backgroundOpacityTextColor : null,
    clarityPercent: themePreferences.mainBackgroundClarityPercent,
    opacityPercent: themePreferences.mainBackgroundOpacityPercent,
  }));

  $effect(() => {
    if (!isTauri() || runtimePlatform !== 'android') return;
    // FLAG_SHOW_WALLPAPER fallback is disabled: on NX809J (Android 16) the translucent WebView
    // window left repaint trails (ghost copies while scrolling) and still showed black. Without
    // All files access the wallpaper mode shows the solid page colour instead.
    const enabled = ANDROID_WALLPAPER_BACKDROP_ENABLED && mainBackgroundMode === 'systemWallpaper' && !androidWallpaperHasImage;
    const blurPx = enabled ? Math.round(androidWallpaperBlurPx * (window.devicePixelRatio || 1)) : 0;
    let cancelled = false;
    void invokeCommand('appearance_set_android_wallpaper_backdrop', { enabled, blurPx })
      .then((status) => {
        if (!cancelled) androidWallpaperBackdrop = status ?? null;
      })
      .catch((error) => {
        if (cancelled) return;
        androidWallpaperBackdrop = null;
        if (enabled) mainBackgroundError = getErrorText(error);
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    document.documentElement.classList.toggle('has-android-wallpaper-backdrop', androidWallpaperActive);
  });

  $effect(() => {
    const snapshot = playback;
    if (!snapshot?.currentTrack || snapshot.state === 'stopped' || snapshot.state === 'empty') {
      artworkController.setTrack(null);
    } else {
      artworkController.setTrack(snapshot.currentTrack.id);
    }
  });

  function applyActiveTheme(): void {
    const pageBackgroundHex = themePreferences.colorMode === 'autoBackgroundOpacity'
      ? (themePreferences.backgroundOpacityTextColor === 'white' ? '#000000' : '#FFFFFF')
      : themePreferences.backgroundHex;
    const contrastBackgroundHex = isNowPlayingOpen && activeArtwork.objectUrl !== null
      ? (nowPlayingContrastBackgroundHex ?? pageBackgroundHex)
      : (mainContrastBackgroundHex ?? pageBackgroundHex);
    applyTheme(document.documentElement, themePreferences, { contrastBackgroundHex, pageBackgroundHex });
  }

  $effect(() => {
    const pageHex = themePreferences.colorMode === 'autoBackgroundOpacity'
      ? (themePreferences.backgroundOpacityTextColor === 'white' ? '#000000' : '#FFFFFF')
      : themePreferences.backgroundHex;
    const imageUrl = effectiveMainBackgroundUrl;
    const opacityPercent = themePreferences.mainBackgroundOpacityPercent;
    const automaticTextColor = themePreferences.colorMode === 'autoBackgroundOpacity'
      ? themePreferences.backgroundOpacityTextColor
      : null;
    const clarityPercent = themePreferences.mainBackgroundClarityPercent;
    const revision = ++mainContrastRevision;
    const hasCurrentImageSample = mainImageAverageUrl === imageUrl
      && mainImageAverageTextColor === automaticTextColor;
    const cachedImageAverageHex = hasCurrentImageSample ? mainImageAverageHex : null;
    const cachedImageAlpha = hasCurrentImageSample ? mainImageAverageAlpha : null;
    void resolveBackdropContrast({
      pageHex,
      imageUrl,
      opacityPercent,
      automaticTextColor,
      clarityPercent,
      cachedImageAverageHex,
      cachedImageAlpha,
    }).then((result) => {
      if (revision !== mainContrastRevision) return;
      if (imageUrl && result.imageAverageHex) {
        mainImageAverageUrl = imageUrl;
        mainImageAverageTextColor = automaticTextColor;
        mainImageAverageHex = result.imageAverageHex;
        mainImageAverageAlpha = result.imageAlpha;
      } else if (!imageUrl) {
        mainImageAverageUrl = null;
        mainImageAverageTextColor = null;
        mainImageAverageHex = null;
        mainImageAverageAlpha = null;
      }
      mainContrastBackgroundHex = result.contrastHex;
      mainBackgroundEffectiveOpacityPercent = result.opacityPercent;
      mainBackgroundOpacityCalculation = result.calculation;
    });
  });

  $effect(() => {
    const mode = mainBackgroundMode;
    const intervalMinutes = slideshowIntervalMinutes;
    const folder = slideshowFolder;
    const platform = runtimePlatform;
    if (!isTauri()) {
      slideshowNextInSeconds = null;
      systemWallpaperNextInSeconds = null;
      slideshowTimingScope = null;
      slideshowStartedAt = null;
      return;
    }
    if (mode === 'slideshow' && folder) {
      systemWallpaperNextInSeconds = null;
      const intervalMs = intervalMinutes * 60_000;
      const timingScope = `${mode}\u0000${folder}`;
      if (slideshowTimingScope !== timingScope || slideshowStartedAt === null) {
        slideshowTimingScope = timingScope;
        slideshowStartedAt = Date.now();
      }
      let disposed = false;
      let switchTimer = 0;
      const nextAt = () => (slideshowStartedAt ?? Date.now()) + intervalMs;
      const updateCountdown = () => {
        slideshowNextInSeconds = Math.max(0, Math.ceil((nextAt() - Date.now()) / 1000));
      };
      const scheduleNext = () => {
        switchTimer = window.setTimeout(async () => {
          if (disposed) return;
          // Start the next interval as the image change begins. If the user
          // adjusts the duration while it loads, the new deadline uses this
          // same elapsed-time origin instead of scheduling a duplicate change.
          slideshowStartedAt = Date.now();
          updateCountdown();
          await loadNextSlideshowBackground();
          if (disposed) return;
          updateCountdown();
          scheduleNext();
        }, Math.max(0, nextAt() - Date.now()));
      };
      updateCountdown();
      const countdownTimer = window.setInterval(updateCountdown, 1000);
      scheduleNext();
      return () => {
        disposed = true;
        window.clearInterval(countdownTimer);
        window.clearTimeout(switchTimer);
      };
    }
    slideshowNextInSeconds = null;
    slideshowTimingScope = null;
    slideshowStartedAt = null;
    if (mode === 'systemWallpaper' && (platform === 'windows' || platform === 'android')) {
      let nextAt = Date.now() + 60_000;
      let disposed = false;
      let checkTimer = 0;
      const updateCountdown = () => {
        systemWallpaperNextInSeconds = Math.max(0, Math.ceil((nextAt - Date.now()) / 1000));
      };
      const scheduleCheck = () => {
        checkTimer = window.setTimeout(async () => {
          if (disposed) return;
          await loadMainBackground();
          if (disposed) return;
          nextAt = Date.now() + 60_000;
          updateCountdown();
          scheduleCheck();
        }, Math.max(0, nextAt - Date.now()));
      };
      updateCountdown();
      const countdownTimer = window.setInterval(updateCountdown, 1000);
      scheduleCheck();
      return () => {
        disposed = true;
        window.clearInterval(countdownTimer);
        window.clearTimeout(checkTimer);
      };
    }
    systemWallpaperNextInSeconds = null;
  });

  $effect(() => {
    const pageHex = themePreferences.colorMode === 'autoBackgroundOpacity'
      ? (themePreferences.backgroundOpacityTextColor === 'white' ? '#000000' : '#FFFFFF')
      : themePreferences.backgroundHex;
    // Keep contrast/opacity tied to the image that is still rendered while a
    // new track's artwork is loading. The artwork controller deliberately
    // retains the previous URL until the replacement has decoded.
    const imageUrl = activeArtwork.objectUrl;
    const opacityPercent = nowPlayingAppearancePreferences.backgroundOpacityPercent;
    const automaticTextColor = themePreferences.colorMode === 'autoBackgroundOpacity'
      ? themePreferences.backgroundOpacityTextColor
      : null;
    const clarityPercent = themePreferences.coverBackgroundClarityPercent;
    const revision = ++nowPlayingContrastRevision;
    const hasCurrentNowPlayingImageSample = nowPlayingImageAverageUrl === imageUrl
      && nowPlayingImageAverageTextColor === automaticTextColor;
    const cachedImageAverageHex = hasCurrentNowPlayingImageSample
      ? nowPlayingImageAverageHex
      : null;
    const cachedImageAlpha = hasCurrentNowPlayingImageSample
      ? nowPlayingImageAverageAlpha
      : null;
    void resolveBackdropContrast({
      pageHex,
      imageUrl,
      opacityPercent,
      automaticTextColor,
      clarityPercent,
      cachedImageAverageHex,
      cachedImageAlpha,
    }).then((result) => {
      if (revision !== nowPlayingContrastRevision) return;
      if (imageUrl && result.imageAverageHex) {
        nowPlayingImageAverageUrl = imageUrl;
        nowPlayingImageAverageTextColor = automaticTextColor;
        nowPlayingImageAverageHex = result.imageAverageHex;
        nowPlayingImageAverageAlpha = result.imageAlpha;
      } else if (!imageUrl) {
        nowPlayingImageAverageUrl = null;
        nowPlayingImageAverageTextColor = null;
        nowPlayingImageAverageHex = null;
        nowPlayingImageAverageAlpha = null;
      }
      nowPlayingContrastBackgroundHex = result.contrastHex;
      nowPlayingBackgroundEffectiveOpacityPercent = result.opacityPercent;
      nowPlayingBackgroundOpacityCalculation = result.calculation;
    });
  });

  $effect(() => {
    void isNowPlayingOpen;
    void mainContrastBackgroundHex;
    void nowPlayingContrastBackgroundHex;
    void themePreferences.backgroundHex;
    void themePreferences.colorMode;
    void themePreferences.backgroundOpacityTextColor;
    void themePreferences.accentHex;
    void themePreferences.autoTextColorThresholdPercent;
    applyActiveTheme();
  });

  function updateCoverFrame(): void {
    const stage = coverStageElement;
    const artworkPane = stage?.parentElement;
    const image = stage?.querySelector('img');
    if (!stage || !artworkPane || !image?.naturalWidth || !image.naturalHeight) {
      coverFrame = null;
      return;
    }

    // While zoomed the artwork pane is resized by the zoom itself; fitting to it would feed back (shrink to nothing /
    // grow without bound). Fit to the pane size measured at 1x instead, so a new track's cover keeps the zoom.
    if (untrack(() => coverZoomScale) === 1 || !coverBaseAvailable) {
      coverBaseAvailable = { width: artworkPane.clientWidth, height: artworkPane.clientHeight };
    }
    const maxDimension = window.innerHeight * 0.92;
    const nextFrame = calculateArtworkFrame({
      sourceWidth: image.naturalWidth,
      sourceHeight: image.naturalHeight,
      availableWidth: coverBaseAvailable.width,
      availableHeight: coverBaseAvailable.height,
      maxWidth: maxDimension,
      maxHeight: maxDimension,
      border: 0,
    });
    if (!nextFrame) {
      coverFrame = null;
      return;
    }
    const nextSize = {
      width: Math.max(1, Math.round(nextFrame.width)),
      height: Math.max(1, Math.round(nextFrame.height)),
    };
    if (!coverFrame || coverFrame.width !== nextSize.width || coverFrame.height !== nextSize.height) {
      coverFrame = nextSize;
    }
    coverBaseReady = true;
  }

  function handleCoverZoom(zoom: { scale: number; x: number; y: number }, size: { width: number; height: number }): void {
    const next = coverZoomToFractions(zoom, size.width, size.height);
    // A cover with no size yet cannot express its pan as a fraction: keep the saved one.
    coverZoomPersisted = size.width > 0 && size.height > 0
      ? next
      : { ...next, offsetX: coverZoomPersisted.offsetX, offsetY: coverZoomPersisted.offsetY };
    scheduleCoverZoomSave();
  }

  // An enlarged cover goes back to the original size when the track changes; a shrunk cover stays shrunk.
  let coverZoomTrackId: string | null | undefined;
  $effect(() => {
    const trackId = playback?.currentTrack?.id ?? null;
    untrack(() => {
      const changed = coverZoomTrackId !== undefined && coverZoomTrackId !== trackId;
      coverZoomTrackId = trackId;
      if (changed && coverZoomPersisted.scale > 1) coverZoomPersisted = { scale: 1, offsetX: 0, offsetY: 0 };
    });
  });

  function restoreCoverZoom(node: HTMLElement): { scale: number; x: number; y: number } {
    return coverZoomFromFractions(coverZoomPersisted, node.offsetWidth, node.offsetHeight);
  }

  let coverZoomSaveTimer: ReturnType<typeof setTimeout> | null = null;
  let coverZoomLastSaved = '';

  function scheduleCoverZoomSave(): void {
    if (!isTauri()) return;
    if (coverZoomSaveTimer) clearTimeout(coverZoomSaveTimer);
    coverZoomSaveTimer = setTimeout(() => void saveCoverZoom(), 300);
  }

  async function saveCoverZoom(): Promise<void> {
    coverZoomSaveTimer = null;
    // Only a shrunk cover is remembered; enlarging is saved as the original size.
    const scale = Math.min(1, coverZoomPersisted.scale);
    const key = String(scale);
    if (key === coverZoomLastSaved) return;
    try {
      await invokeCommand('settings_set_cover_zoom', { scale });
      coverZoomLastSaved = key;
    } catch (error) {
      console.warn(tr('無法保存封面縮放：'), getErrorText(error));
    }
  }

  async function loadCoverZoom(): Promise<void> {
    try {
      const stored = await invokeCommand('settings_get_cover_zoom', {});
      const scale = Number.isFinite(stored.scale) ? Math.min(1, stored.scale) : 1;
      coverZoomPersisted = { scale, offsetX: 0, offsetY: 0 };
      coverZoomLastSaved = String(scale);
    } catch (error) {
      console.warn(tr('無法讀取封面縮放：'), getErrorText(error));
    }
  }

  $effect(() => {
    const stage = coverStageElement;
    const artworkUrl = activeArtwork.objectUrl;
    const artworkStatus = activeArtwork.status;
    if (!stage) {
      coverFrame = null;
      return;
    }

    void artworkUrl;
    void artworkStatus;
    const image = stage.querySelector('img');
    const observer = new ResizeObserver(() => untrack(updateCoverFrame));
    if (stage.parentElement) observer.observe(stage.parentElement);
    image?.addEventListener('load', updateCoverFrame);
    untrack(updateCoverFrame);
    return () => {
      observer.disconnect();
      image?.removeEventListener('load', updateCoverFrame);
    };
  });

  $effect(() => {
    if (coverZoomScale === 1) untrack(updateCoverFrame);
  });

  onMount(() => {
    document.title = APP_WINDOW_TITLE;
    let disposed = false;
    const updateTrackHeaderTextOffset = (): void => {
      const devicePixelRatio = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
      document.documentElement.style.setProperty(
        '--track-header-text-optical-offset',
        `${-3.25 / devicePixelRatio}px`,
      );
    };
    const applyAndroidInsets = (value?: AndroidWindowInsets): void => {
      let insets = value;
      if (!insets) {
        const current = (window as Window & { MoeAndroidInsets?: { currentInsetsJson(): string } }).MoeAndroidInsets?.currentInsetsJson();
        if (!current) return;
        try {
          insets = JSON.parse(current) as AndroidWindowInsets;
        } catch {
          return;
        }
      }
      const root = document.documentElement;
      const devicePixelRatio = window.devicePixelRatio > 0 ? window.devicePixelRatio : 1;
      for (const [property, pixels] of [
        ['--android-safe-left', insets.safeLeftPx],
        ['--android-safe-top', insets.safeTopPx],
        ['--android-safe-right', insets.safeRightPx],
        ['--android-safe-bottom', insets.safeBottomPx],
      ] as const) {
        if (Number.isFinite(pixels) && pixels >= 0) {
          root.style.setProperty(property, `${pixels / devicePixelRatio}px`);
        }
      }
    };
    const handleAndroidInsets = (event: Event): void => {
      applyAndroidInsets((event as CustomEvent<AndroidWindowInsets>).detail);
    };
    window.addEventListener('resize', updateTrackHeaderTextOffset);
    window.addEventListener('popstate', handleDetailHistoryPopState);
    window.addEventListener('moe:android-insets', handleAndroidInsets);
    window.addEventListener('keydown', handleShortcutKeydown, true);
    window.addEventListener('mousedown', handleShortcutMouseDown, true);
    window.addEventListener('wheel', handleShortcutWheel, { capture: true, passive: false });
    gamepadFrame = requestAnimationFrame(pollGamepads);
    updateTrackHeaderTextOffset();
    applyAndroidInsets();
    if (isTauri()) {
      // Backstop sample (ADR 0028): the clock advances the bar between samples; commands, focus, visibility and
      // device changes take a sample at once. It keeps running while minimized so the taskbar lyrics stay fed.
      playbackPollTimer = setInterval(() => {
        if (playbackReady && !isSendingPlaybackCommand) {
          void loadPlaybackSnapshot(false);
        }
      }, PLAYBACK_POLL_MS);
      livePlayCountTimer = setInterval(() => {
        publishLivePlayCount();
      }, 1000);
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
          if (libraryReady) void loadPlaylists(undefined, true);
          if (sourceSyncReady) void loadSources();
        });
      } catch (error) {
        unlistenProgress?.();
        unlistenFinished?.();
        await loadCapabilities();
        if (!disposed) runtimeError = tr("無法訂閱曲庫同步狀態：{0}", [getErrorText(error)]);
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
      ]);
      await Promise.all([
        loadMainBackground(),
        loadSettingsRecoveryWarning(),
        loadTrackColumnSettings(),
        loadNowPlayingLayout(),
        loadDseeHx(),
        loadDsdSettings(),
        loadTaskbarLyricsSettings(),
        loadLanguage(),
        loadCoverZoom(),
        loadShortcutSettings(),
        loadNowPlayingAppearancePreferences(),
        loadLyricsPreferences(),
      ]);
      if (!disposed && sourceSyncReady) void syncLibrary();
    }

    return () => {
      disposed = true;
      window.removeEventListener('resize', updateTrackHeaderTextOffset);
      window.removeEventListener('popstate', handleDetailHistoryPopState);
      window.removeEventListener('moe:android-insets', handleAndroidInsets);
      unlistenSyncProgress?.();
      unlistenSyncFinished?.();
    };
  });

  onDestroy(() => {
    clearTrackColumnOrderTimer();
    trackColumnOrderPending = null;
    trackColumnOrderDrag = null;
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
    if (livePlayCountTimer !== undefined) clearInterval(livePlayCountTimer);
    unlistenSyncProgress?.();
    unlistenSyncFinished?.();
    playlistListRequestVersion += 1;
    window.removeEventListener('keydown', handleShortcutKeydown, true);
    window.removeEventListener('mousedown', handleShortcutMouseDown, true);
    window.removeEventListener('wheel', handleShortcutWheel, true);
    mainBackgroundReadRevision += 1;
    if (mainBackgroundObjectUrl) URL.revokeObjectURL(mainBackgroundObjectUrl);
  });

  async function loadMainBackground(): Promise<void> {
    if (!isTauri()) return;
    const revision = ++mainBackgroundReadRevision;
    try {
      const bytes = await getMainBackgroundBytes();
      if (revision !== mainBackgroundReadRevision) return;
      if (capabilities?.platform === 'android' && themePreferences.mainBackgroundMode === 'systemWallpaper') {
        androidWallpaperHasImage = bytes.byteLength > 0;
        // No readable wallpaper: drop any previous image so the native backdrop shows instead.
        if (bytes.byteLength === 0) {
          clearMainBackgroundImage();
          // Just chosen by the user and refused for want of the read-images permission: explain it, once per choice.
          if (wallpaperPromptWanted) {
            wallpaperPromptWanted = false;
            if ((await invokeCommand('android_system_wallpaper_reason', {})) === 'mediaImages') {
              wallpaperPermissionPromptOpen = true;
            }
          }
        } else {
          wallpaperPromptWanted = false;
        }
      }
      await setMainBackgroundBytes(bytes, revision);
      if (revision !== mainBackgroundReadRevision) return;
      mainBackgroundError = null;
    } catch (error) {
      if (revision !== mainBackgroundReadRevision) return;
      mainBackgroundError = tr("無法讀取主介面背景：{0}", [getErrorText(error)]);
    }
  }

  async function loadNextSlideshowBackground(): Promise<void> {
    const revision = ++mainBackgroundReadRevision;
    try {
      const bytes = await nextSlideshowBackgroundBytes();
      if (revision !== mainBackgroundReadRevision || themePreferences.mainBackgroundMode !== 'slideshow') return;
      if (bytes.byteLength === 0) return;
      await setMainBackgroundBytes(bytes, revision);
      if (revision !== mainBackgroundReadRevision || themePreferences.mainBackgroundMode !== 'slideshow') return;
      mainBackgroundError = null;
    } catch (error) {
      if (revision !== mainBackgroundReadRevision) return;
      mainBackgroundError = getErrorText(error);
    }
  }

  async function setMainBackgroundBytes(bytes: ArrayBuffer, revision: number): Promise<void> {
    const data = new Uint8Array(bytes);
    let hash = 2166136261;
    for (const byte of data) {
      hash ^= byte;
      hash = Math.imul(hash, 16777619);
    }
    const nextHash = `${data.byteLength}:${hash >>> 0}`;
    if (nextHash === mainBackgroundBytesHash) return;

    const mimeType = bytes.byteLength > 0 ? detectArtworkMimeType(bytes) : null;
    if (!mimeType) {
      if (revision !== mainBackgroundReadRevision) return;
      return;
    }
    const nextObjectUrl = await createDecodedImageObjectUrl(bytes, mimeType);
    if (revision !== mainBackgroundReadRevision) {
      URL.revokeObjectURL(nextObjectUrl);
      return;
    }

    const previousObjectUrl = mainBackgroundObjectUrl;
    mainBackgroundObjectUrl = nextObjectUrl;
    mainBackgroundUrl = nextObjectUrl;
    mainBackgroundBytesHash = nextHash;
    await tick();
    if (previousObjectUrl && previousObjectUrl !== nextObjectUrl) {
      URL.revokeObjectURL(previousObjectUrl);
    }
  }

  function clearMainBackgroundImage(): void {
    const previousObjectUrl = mainBackgroundObjectUrl;
    mainBackgroundObjectUrl = null;
    mainBackgroundBytesHash = '';
    mainBackgroundUrl = null;
    if (previousObjectUrl) {
      void tick().then(() => URL.revokeObjectURL(previousObjectUrl));
    }
  }

  function updateMainBackgroundLook(patch: Partial<ThemePreferences>): void {
    const next = normalizeThemePreferences({ ...themePreferences, ...patch });
    themePreferences = next;
    themeSaveError = null;
    themeSaveState = isTauri() ? 'saving' : 'preview';
    const revision = ++themeRevision;
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = setTimeout(() => {
      themeSaveTimer = undefined;
      queueThemeSave(next, revision);
    }, 300);
  }

  function updateCoverBackgroundClarity(value: number, flush = false): void {
    updateMainBackgroundLook({ coverBackgroundClarityPercent: value });
    if (flush) saveThemePreferencesNow();
  }

  async function pickMainBackground(): Promise<void> {
    if (!isTauri() || mainBackgroundBusy) {
      mainBackgroundError = isTauri() ? null : tr('瀏覽器預覽無法選擇背景圖片。');
      return;
    }
    mainBackgroundBusy = true;
    mainBackgroundError = null;
    mainBackgroundReadRevision += 1;
    try {
      await themeSaveQueue;
      const picked = await invokeCommand('appearance_pick_main_background', {});
      if (picked) {
        await loadThemePreferences();
        await loadMainBackground();
      }
    } catch (error) {
      mainBackgroundError = getErrorText(error);
    } finally {
      mainBackgroundBusy = false;
    }
  }

  async function pickSlideshowFolder(): Promise<void> {
    if (!isTauri() || mainBackgroundBusy) {
      mainBackgroundError = isTauri() ? null : tr('瀏覽器預覽無法選擇幻燈片資料夾。');
      return;
    }
    mainBackgroundBusy = true;
    mainBackgroundError = null;
    mainBackgroundReadRevision += 1;
    try {
      await themeSaveQueue;
      const picked = await invokeCommand('appearance_pick_slideshow_folder', {});
      if (picked) {
        await loadThemePreferences();
        await loadMainBackground();
      }
    } catch (error) {
      mainBackgroundError = getErrorText(error);
    } finally {
      mainBackgroundBusy = false;
    }
  }

  async function clearMainBackground(): Promise<void> {
    if (!isTauri() || mainBackgroundBusy) return;
    mainBackgroundBusy = true;
    mainBackgroundError = null;
    mainBackgroundReadRevision += 1;
    try {
      await invokeCommand('appearance_clear_main_background', {});
      clearMainBackgroundImage();
    } catch (error) {
      mainBackgroundError = getErrorText(error);
    } finally {
      mainBackgroundBusy = false;
    }
  }

  async function selectMainBackgroundMode(mode: ThemePreferences['mainBackgroundMode']): Promise<void> {
    if (mode === 'systemWallpaper' && capabilities?.platform !== 'windows' && capabilities?.platform !== 'android') return;
    mainBackgroundReadRevision += 1;
    wallpaperPromptWanted = mode === 'systemWallpaper' && capabilities?.platform === 'android';
    updateMainBackgroundLook({ mainBackgroundMode: mode });
    if (mode === 'solid') clearMainBackgroundImage();
    saveThemePreferencesNow();
    await themeSaveQueue;
    await loadMainBackground();
  }

  async function loadThemePreferences(): Promise<void> {
    const revision = themeRevision;
    try {
      const stored = await invokeCommand('theme_get_preferences', {});
      if (revision !== themeRevision) return;
      themePreferences = normalizeThemePreferences(stored);
      themeSaveState = 'saved';
      themeSaveError = null;
    } catch (error) {
      if (revision !== themeRevision) return;
      themeSaveState = 'error';
      themeSaveError = tr("無法讀取已保存的外觀設定：{0}", [getErrorText(error)]);
    }
  }

  /** Lyrics are loaded while something shows them: the Now Playing page (once opened) or the taskbar lyrics. */
  $effect(() => {
    const wanted = isNowPlayingMounted || (capabilities?.platform === 'windows' && taskbarLyricsSettings.enabled);
    const track = playback?.currentTrack ?? null;
    const id = wanted && track ? track.id : null;
    const title = track?.title ?? null;
    const artist = track?.artist ?? null;
    untrack(() => void lyricsController.setTrack(id, { title, artist }));
  });

  // The lyrics as the page shows them (Simplified-to-Traditional applied), reduced for the taskbar window.
  $effect(() => {
    const lyrics = lyricsState.lyrics;
    const trackId = lyricsState.trackId;
    const convert = lyricsPreferences.simplifiedToTraditional;
    let cancelled = false;
    void (async () => {
      let shown = lyrics;
      if (lyrics && convert) {
        try {
          shown = { ...lyrics, lines: (await convertTrackLyrics(lyrics)).lines };
        } catch {
          shown = lyrics;
        }
      }
      if (!cancelled) taskbarLyricsData = buildTaskbarLyricsData(trackId, shown);
    })();
    return () => { cancelled = true; };
  });

  // Send them to the taskbar window whenever they change, and when the window opens and asks. (`capabilities` is
  // replaced every few seconds, so only its platform, a plain value, is tracked: the same lyrics are not sent again.)
  let isWindowsPlatform = $derived(capabilities?.platform === 'windows');
  let taskbarLyricsEnabled = $derived(taskbarLyricsSettings.enabled);
  $effect(() => {
    const data = taskbarLyricsData;
    if (!isTauri() || !data || !isWindowsPlatform || !taskbarLyricsEnabled) return;
    void emitTo('taskbar-lyrics', 'taskbar-lyrics-data', data).catch(() => {});
  });

  $effect(() => {
    if (!isTauri() || capabilities?.platform !== 'windows') return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('taskbar-lyrics-request', () => {
      if (taskbarLyricsData) void emitTo('taskbar-lyrics', 'taskbar-lyrics-data', taskbarLyricsData).catch(() => {});
    })
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });

  /** Saving tags rewrote the file: rescan the library, then reload the playing track (summary, cover, lyrics). */
  async function reloadAfterTrackDetailsSaved(): Promise<void> {
    await syncLibrary();
    try {
      await invokeCommand('playback_refresh_current_track', {});
    } catch (error) {
      playbackError = getErrorText(error);
    }
    artworkController.reload();
    void lyricsController.reload();
    void loadPlaybackSnapshot(false);
  }

  async function loadTrackColumnSettings(): Promise<void> {
    const revision = trackColumnSettingsRevision;
    try {
      const stored = await invokeCommand('settings_get_track_list_columns', {});
      if (revision !== trackColumnSettingsRevision) return;
      trackColumnPreferences = normalizeTrackColumnPreferences(stored.columns);
      trackColumnWidths = normalizeTrackColumnWidths(stored.widths);
      trackRowHeightPx = normalizeTrackRowHeightPx(stored.rowHeightPx);
      albumCardMaxWidthPx = normalizeAlbumCardMaxWidthPx(stored.albumCardMaxWidthPx);
      groupingEnabled = stored.groupingEnabled !== false;
      trackTextOverflowMode = stored.textOverflowMode === 'middle' ? 'middle' : 'end';
      trackTextOverflowStore.set(trackTextOverflowMode);
      librarySort = normalizeLibrarySort(stored.librarySort);
      trackColumnSettingsState = 'saved';
      trackColumnSettingsError = null;
    } catch (error) {
      if (revision !== trackColumnSettingsRevision) return;
      trackColumnSettingsState = 'error';
      trackColumnSettingsError = tr("無法讀取曲目欄位設定：{0}", [getErrorText(error)]);
    }
  }

  function persistTrackColumnSettings(options?: {
    columns?: TrackListColumnPreference[];
    widths?: TrackColumnWidths;
    librarySort?: LibraryListSort;
    rowHeightPx?: number;
    albumCardMaxWidthPx?: number;
    groupingEnabled?: boolean;
    textOverflowMode?: TrackTextOverflowMode;
  }): void {
    if (options?.columns) {
      trackColumnPreferences = normalizeTrackColumnPreferences(options.columns);
    }
    if (options?.widths) {
      trackColumnWidths = normalizeTrackColumnWidths(options.widths);
    }
    if (options?.rowHeightPx !== undefined) {
      trackRowHeightPx = normalizeTrackRowHeightPx(options.rowHeightPx);
    }
    if (options?.albumCardMaxWidthPx !== undefined) {
      albumCardMaxWidthPx = normalizeAlbumCardMaxWidthPx(options.albumCardMaxWidthPx);
    }
    if (options?.groupingEnabled !== undefined) {
      groupingEnabled = options.groupingEnabled;
    }
    if (options?.textOverflowMode !== undefined) {
      trackTextOverflowMode = options.textOverflowMode;
      trackTextOverflowStore.set(trackTextOverflowMode);
    }
    if (options?.librarySort) {
      librarySort = normalizeLibrarySort(options.librarySort);
    }
    trackColumnSettingsError = null;
    const revision = ++trackColumnSettingsRevision;
    if (!isTauri()) {
      trackColumnSettingsState = 'preview';
      trackColumnSettingsError = tr('瀏覽器預覽不會保存曲目欄位設定。');
      return;
    }

    trackColumnSettingsState = 'saving';
    trackColumnSettingsQueue = trackColumnSettingsQueue.catch(() => undefined).then(async () => {
      if (revision !== trackColumnSettingsRevision) return;
      try {
        const saved = await invokeCommand('settings_set_track_list_columns', {
          preferences: {
            columns: trackColumnPreferences.map((column) => ({ ...column })),
            widths: { ...trackColumnWidths },
            librarySort: { ...librarySort },
            rowHeightPx: trackRowHeightPx,
            albumCardMaxWidthPx,
            groupingEnabled,
            textOverflowMode: trackTextOverflowMode,
          },
        });
        if (revision !== trackColumnSettingsRevision) return;
        trackColumnPreferences = normalizeTrackColumnPreferences(saved.columns);
        trackColumnWidths = normalizeTrackColumnWidths(saved.widths);
        trackRowHeightPx = normalizeTrackRowHeightPx(saved.rowHeightPx);
        albumCardMaxWidthPx = normalizeAlbumCardMaxWidthPx(saved.albumCardMaxWidthPx);
        groupingEnabled = saved.groupingEnabled !== false;
        trackTextOverflowMode = saved.textOverflowMode === 'middle' ? 'middle' : 'end';
        trackTextOverflowStore.set(trackTextOverflowMode);
        librarySort = normalizeLibrarySort(saved.librarySort);
        trackColumnSettingsState = 'saved';
        trackColumnSettingsError = null;
      } catch (error) {
        if (revision !== trackColumnSettingsRevision) return;
        trackColumnSettingsState = 'error';
        trackColumnSettingsError = tr("無法保存曲目欄位設定：{0}", [getErrorText(error)]);
      }
    });
  }

  function updateTrackColumnSettings(next: TrackListColumnPreference[]): void {
    persistTrackColumnSettings({ columns: next });
  }

  function setTrackColumnVisible(id: TrackListColumnPreference['id'], visible: boolean): void {
    updateTrackColumnSettings(setTrackColumnVisibility(trackColumnPreferences, id, visible));
  }

  function moveConfiguredTrackColumn(id: TrackListColumnPreference['id'], direction: 'up' | 'down'): void {
    updateTrackColumnSettings(moveTrackColumn(trackColumnPreferences, id, direction));
  }

  function clearTrackColumnOrderTimer(): void {
    if (trackColumnOrderTimer !== null) {
      clearTimeout(trackColumnOrderTimer);
      trackColumnOrderTimer = null;
    }
  }

  function handleTrackColumnOrderPointerDown(event: PointerEvent, index: number): void {
    if (event.button !== 0) return;
    clearTrackColumnOrderTimer();
    trackColumnOrderPending = {
      pointerId: event.pointerId,
      fromIndex: index,
      startX: event.clientX,
      startY: event.clientY,
    };
    const target = event.currentTarget as HTMLElement;
    trackColumnOrderTimer = setTimeout(() => {
      const pending = trackColumnOrderPending;
      if (!pending || pending.pointerId !== event.pointerId) return;
      trackColumnOrderPending = null;
      trackColumnOrderDrag = { pointerId: pending.pointerId, fromIndex: pending.fromIndex, toIndex: pending.fromIndex };
      try {
        target.setPointerCapture(pending.pointerId);
      } catch {
        // The pointer may have ended while the long-press callback was queued.
      }
    }, QUEUE_REORDER_LONG_PRESS_MS);
  }

  function handleTrackColumnOrderPointerMove(event: PointerEvent): void {
    const pending = trackColumnOrderPending;
    if (pending?.pointerId === event.pointerId && !trackColumnOrderDrag) {
      if (exceedsLongPressSlop(event.clientX - pending.startX, event.clientY - pending.startY, QUEUE_REORDER_MOVE_CANCEL_PX)) {
        clearTrackColumnOrderTimer();
        trackColumnOrderPending = null;
      }
      return;
    }
    const drag = trackColumnOrderDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    event.preventDefault();
    const list = (event.currentTarget as HTMLElement).closest<HTMLOListElement>('.track-column-settings');
    const rows = list ? [...list.querySelectorAll<HTMLElement>('[data-track-column-index]')] : [];
    if (rows.length === 0) return;
    const rowCenters = rows.map((row) => {
      const rect = row.getBoundingClientRect();
      return rect.top + rect.height / 2;
    });
    const destination = trackColumnDropIndexFromPointer(event.clientY, rowCenters, drag.fromIndex);
    if (destination !== drag.toIndex) trackColumnOrderDrag = { ...drag, toIndex: destination };
  }

  function finishTrackColumnOrderPointer(event: PointerEvent): void {
    const pending = trackColumnOrderPending;
    if (pending?.pointerId === event.pointerId) {
      clearTrackColumnOrderTimer();
      trackColumnOrderPending = null;
    }
    const drag = trackColumnOrderDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already have been released.
    }
    trackColumnOrderDrag = null;
    if (drag.fromIndex !== drag.toIndex) {
      const column = trackColumnPreferences[drag.fromIndex];
      if (column) updateTrackColumnSettings(moveTrackColumnTo(trackColumnPreferences, column.id, drag.toIndex));
    }
  }

  function cancelTrackColumnOrderPointer(event: PointerEvent): void {
    if (trackColumnOrderPending?.pointerId === event.pointerId || trackColumnOrderDrag?.pointerId === event.pointerId) {
      clearTrackColumnOrderTimer();
      trackColumnOrderPending = null;
      trackColumnOrderDrag = null;
    }
  }

  function handleTrackColumnOrderKeydown(event: KeyboardEvent, id: TrackListColumnPreference['id']): void {
    if (event.target !== event.currentTarget) return;
    if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
    event.preventDefault();
    event.stopPropagation();
    moveConfiguredTrackColumn(id, event.key === 'ArrowUp' ? 'up' : 'down');
  }

  function handleTrackColumnResize(id: TrackListColumnPreference['id'], widthPx: number): void {
    persistTrackColumnSettings({ widths: setTrackColumnWidth(trackColumnWidths, id, widthPx) });
  }

  function previewTrackRowHeight(value: number): void {
    trackRowHeightPx = normalizeTrackRowHeightPx(value);
  }

  function commitTrackRowHeight(value: number): void {
    persistTrackColumnSettings({ rowHeightPx: value });
  }

  function previewAlbumCardMaxWidth(value: number): void {
    albumCardMaxWidthPx = normalizeAlbumCardMaxWidthPx(value);
  }

  function commitAlbumCardMaxWidth(value: number): void {
    persistTrackColumnSettings({ albumCardMaxWidthPx: value });
  }

  function setGroupingEnabled(value: boolean): void {
    persistTrackColumnSettings({ groupingEnabled: value });
  }

  function handleLibrarySort(id: TrackListColumnPreference['id']): void {
    persistTrackColumnSettings({ librarySort: toggleLibrarySort(librarySort, id) });
  }

  async function handleQueueSort(id: TrackListColumnPreference['id']): Promise<void> {
    if (queueSortBusy) return;
    const previousSort = queueSort;
    const nextSort: LibraryListSort = queueSort?.field === id
      ? toggleLibrarySort(queueSort, id)
      : { field: id, direction: getFirstSortDirection(id) };
    queueSort = nextSort;
    queueSortBusy = true;
    try {
      const snapshot = await invokeCommand('playback_sort_queue', { sort: nextSort });
      applyPlaybackSnapshot(snapshot);
      playbackQueueResetKey += 1;
      playbackError = null;
    } catch (error) {
      queueSort = previousSort;
      playbackError = tr("播放佇列排序失敗：{0}", [getErrorText(error)]);
    } finally {
      queueSortBusy = false;
    }
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
      nowPlayingLayoutError = tr("無法讀取正在播放版面設定：{0}", [getErrorText(error)]);
    }
  }

  function setNowPlayingLayout(layout: NowPlayingLayout): void {
    nowPlayingLayout = layout;
    nowPlayingLayoutError = null;
    const revision = ++nowPlayingLayoutRevision;
    if (!isTauri()) {
      nowPlayingLayoutState = 'preview';
      nowPlayingLayoutError = tr('瀏覽器預覽不會保存正在播放版面設定。');
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
        nowPlayingLayoutError = tr("無法保存正在播放版面設定：{0}", [getErrorText(error)]);
      }
    });
  }

  async function loadDseeHx(): Promise<void> {
    const revision = dseeHxRevision;
    try {
      const stored = await invokeCommand('settings_get_dsee_hx', {});
      if (revision !== dseeHxRevision) return;
      dseeHx = stored === true;
      dseeHxState = 'saved';
      dseeHxError = null;
    } catch (error) {
      if (revision !== dseeHxRevision) return;
      dseeHxState = 'error';
      dseeHxError = tr("無法讀取 DSEE HX 設定：{0}", [getErrorText(error)]);
    }
  }

  async function loadShortcutSettings(): Promise<void> {
    if (!isTauri()) {
      shortcutSettings = defaultShortcutSettings();
      return;
    }
    try {
      shortcutSettings = normalizeShortcutSettings(await invokeCommand('settings_get_shortcuts', {}));
    } catch {
      shortcutSettings = defaultShortcutSettings();
    }
  }

  async function saveShortcutSettings(next: ShortcutSettings): Promise<void> {
    shortcutSettings = next;
    if (!isTauri()) return;
    try {
      shortcutSettings = normalizeShortcutSettings(await invokeCommand('settings_set_shortcuts', { shortcuts: next }));
    } catch {
      /* keep the local bindings if the save fails */
    }
  }

  function beginShortcutCapture(action: ShortcutAction): void {
    capturingShortcut = action;
  }

  function recordShortcutBinding(binding: ShortcutBinding): void {
    if (!capturingShortcut) return;
    const action = capturingShortcut;
    capturingShortcut = null;
    void saveShortcutSettings(addShortcutBinding(shortcutSettings, action, binding));
  }

  function deleteShortcutBinding(action: ShortcutAction, binding: ShortcutBinding): void {
    void saveShortcutSettings(removeShortcutBinding(shortcutSettings, action, binding));
  }

  function runShortcut(action: ShortcutAction): void {
    if (action === 'fullscreen') {
      window.dispatchEvent(new Event('moemusicplayer-toggle-fullscreen'));
      return;
    }
    if (action === 'playPause') {
      void togglePlayback();
      return;
    }
    if (action === 'previous') {
      void controlPlayback('playback_previous');
      return;
    }
    if (action === 'next') {
      void controlPlayback('playback_next');
      return;
    }
    const delta = action === 'seekBack' ? -5_000 : 5_000;
    const duration = playback?.durationMs;
    const next = Math.max(0, currentPlaybackPositionMs() + delta);
    void commitPlaybackSeek(duration == null ? next : Math.min(next, duration));
  }

  function shortcutEventShouldRun(target: EventTarget | null): boolean {
    return !shortcutTargetIsEditable(target) && !(target instanceof Element && target.closest('button, a, [role="slider"]'));
  }

  function handleShortcutKeydown(event: KeyboardEvent): void {
    if (document.querySelector('.update-dialog[open]')) return;
    if (event.repeat || event.ctrlKey || event.altKey || event.metaKey) return;
    if (capturingShortcut) {
      event.preventDefault();
      event.stopPropagation();
      if (event.code === 'Escape') {
        capturingShortcut = null;
        return;
      }
      recordShortcutBinding({ device: 'keyboard', code: event.code });
      return;
    }
    if (!shortcutEventShouldRun(event.target)) return;
    const action = shortcutActionFor(shortcutSettings, 'keyboard', event.code);
    if (!action) return;
    event.preventDefault();
    runShortcut(action);
  }

  function handleShortcutMouseDown(event: MouseEvent): void {
    if (document.querySelector('.update-dialog[open]')) return;
    const code = event.button === 3 ? 'back' : event.button === 4 ? 'forward' : null;
    if (!code) return;
    if (capturingShortcut) {
      event.preventDefault();
      recordShortcutBinding({ device: 'mouse', code });
      return;
    }
    const action = shortcutActionFor(shortcutSettings, 'mouse', code);
    if (!action) return;
    event.preventDefault();
    runShortcut(action);
  }

  const gamepadPressed = new Map<string, boolean>();
  let gamepadFrame = 0;

  function pollGamepads(): void {
    if (document.querySelector('.update-dialog[open]')) {
      gamepadFrame = requestAnimationFrame(pollGamepads);
      return;
    }
    const pads = navigator.getGamepads?.() ?? [];
    let captured = false;
    for (const pad of pads) {
      if (!pad || captured) continue;
      for (let index = 0; index < pad.buttons.length && index <= 15; index += 1) {
        const button = pad.buttons[index];
        const key = `${pad.index}:${index}`;
        const pressed = button.pressed || button.value > 0.55;
        const wasPressed = gamepadPressed.get(key) === true;
        gamepadPressed.set(key, pressed);
        if (!pressed || wasPressed) continue;
        const code = `button${index}`;
        if (capturingShortcut) {
          recordShortcutBinding({ device: 'gamepad', code });
          captured = true;
          break;
        }
        const action = shortcutActionFor(shortcutSettings, 'gamepad', code);
        if (action) runShortcut(action);
      }
      if (captured) continue;
      const stickCodes = [
        ['stickLeftXMinus', 'stickLeftXPlus'],
        ['stickLeftYMinus', 'stickLeftYPlus'],
        ['stickRightXMinus', 'stickRightXPlus'],
        ['stickRightYMinus', 'stickRightYPlus'],
      ] as const;
      for (let axis = 0; axis < stickCodes.length && axis < pad.axes.length; axis += 1) {
        const value = pad.axes[axis] ?? 0;
        for (const [sign, code] of [[-1, stickCodes[axis][0]], [1, stickCodes[axis][1]]] as const) {
          const key = `${pad.index}:${code}`;
          const pressed = sign < 0 ? value < -0.65 : value > 0.65;
          const wasPressed = gamepadPressed.get(key) === true;
          gamepadPressed.set(key, pressed);
          if (!pressed || wasPressed || captured) continue;
          if (capturingShortcut) {
            recordShortcutBinding({ device: 'gamepad', code });
            captured = true;
            break;
          }
          const action = shortcutActionFor(shortcutSettings, 'gamepad', code);
          if (action) runShortcut(action);
        }
      }
    }
    gamepadFrame = requestAnimationFrame(pollGamepads);
  }

  function handleShortcutWheel(event: WheelEvent): void {
    if (document.querySelector('.update-dialog[open]')) return;
    if (event.deltaY === 0) return;
    const code = event.deltaY < 0 ? 'wheelUp' : 'wheelDown';
    if (capturingShortcut) {
      event.preventDefault();
      recordShortcutBinding({ device: 'mouse', code });
      return;
    }
    if (shortcutTargetIsEditable(event.target) || shortcutTargetIsScrollable(event.target)) return;
    const action = shortcutActionFor(shortcutSettings, 'mouse', code);
    if (!action) return;
    event.preventDefault();
    runShortcut(action);
  }

  function setDseeHx(enabled: boolean): void {
    dseeHx = enabled;
    dseeHxError = null;
    const revision = ++dseeHxRevision;
    if (!isTauri()) {
      dseeHxState = 'preview';
      dseeHxError = tr('瀏覽器預覽不會保存 DSEE HX 設定。');
      return;
    }
    dseeHxState = 'saving';
    dseeHxQueue = dseeHxQueue.catch(() => undefined).then(async () => {
      if (revision !== dseeHxRevision) return;
      try {
        const saved = await invokeCommand('settings_set_dsee_hx', { enabled });
        if (revision !== dseeHxRevision) return;
        dseeHx = saved;
        dseeHxState = 'saved';
        dseeHxError = null;
      } catch (error) {
        if (revision !== dseeHxRevision) return;
        dseeHxState = 'error';
        dseeHxError = getErrorText(error);
      }
      void loadPlaybackSnapshot(false);
    });
  }

  async function loadDsdSettings(): Promise<void> {
    const revision = dsdSettingsRevision;
    try {
      const stored = await invokeCommand('settings_get_dsd', {});
      if (revision !== dsdSettingsRevision) return;
      dsdSettings = normalizeDsdSettings(stored);
      dsdSettingsState = 'saved';
      dsdSettingsError = null;
    } catch (error) {
      if (revision !== dsdSettingsRevision) return;
      dsdSettingsState = 'error';
      dsdSettingsError = tr("無法讀取 DSD 設定：{0}", [getErrorText(error)]);
    }
  }

  function setDsdSettings(patch: Partial<DsdPlaybackSettings>): void {
    const next = normalizeDsdSettings({ ...dsdSettings, ...patch });
    dsdSettings = next;
    dsdSettingsError = null;
    const revision = ++dsdSettingsRevision;
    if (!isTauri()) {
      dsdSettingsState = 'preview';
      dsdSettingsError = tr('瀏覽器預覽不會保存 DSD 設定。');
      return;
    }
    dsdSettingsState = 'saving';
    dsdSettingsQueue = dsdSettingsQueue.catch(() => undefined).then(async () => {
      if (revision !== dsdSettingsRevision) return;
      try {
        const saved = await invokeCommand('settings_set_dsd', { gainDb: next.gainDb, outputRateHz: next.outputRateHz, dop: next.dop });
        if (revision !== dsdSettingsRevision) return;
        dsdSettings = normalizeDsdSettings(saved);
        dsdSettingsState = 'saved';
        dsdSettingsError = null;
      } catch (error) {
        if (revision !== dsdSettingsRevision) return;
        dsdSettingsState = 'error';
        dsdSettingsError = getErrorText(error);
      }
      void loadPlaybackSnapshot(false);
    });
  }

  /** The saved language wins: when this page loaded with another one, adopt it (reloads once). */
  async function loadLanguage(): Promise<void> {
    try {
      adoptSavedLanguage(await invokeCommand('settings_get_language', {}));
    } catch {
      // Keep the language this page loaded with.
    }
  }

  async function chooseLanguage(next: LanguageChoice): Promise<void> {
    languageError = null;
    try {
      await changeLanguage(next, (choice) => invokeCommand('settings_set_language', { language: choice as LanguageChoice }));
    } catch (error) {
      languageError = getErrorText(error);
    }
  }

  async function loadTaskbarLyricsSettings(): Promise<void> {
    const revision = taskbarLyricsRevision;
    try {
      const stored = await invokeCommand('settings_get_taskbar_lyrics', {});
      if (revision !== taskbarLyricsRevision) return;
      taskbarLyricsSettings = normalizeTaskbarLyricsSettings(stored);
    } catch (error) {
      if (revision !== taskbarLyricsRevision) return;
      taskbarLyricsError = tr("無法讀取工作列歌詞設定：{0}", [getErrorText(error)]);
    }
  }

  function setTaskbarLyricsSettings(patch: Partial<TaskbarLyricsSettings>): void {
    const next = normalizeTaskbarLyricsSettings({ ...taskbarLyricsSettings, ...patch });
    taskbarLyricsSettings = next;
    taskbarLyricsError = null;
    const revision = ++taskbarLyricsRevision;
    if (!isTauri()) return;
    taskbarLyricsQueue = taskbarLyricsQueue.catch(() => undefined).then(async () => {
      // A burst of changes (dragging in the colour picker) only saves the latest one.
      if (revision !== taskbarLyricsRevision) return;
      try {
        const saved = await invokeCommand('settings_set_taskbar_lyrics', { settings: next });
        if (revision === taskbarLyricsRevision) taskbarLyricsSettings = normalizeTaskbarLyricsSettings(saved);
      } catch (error) {
        if (revision === taskbarLyricsRevision) taskbarLyricsError = getErrorText(error);
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
      nowPlayingAppearanceError = tr("無法讀取正在播放外觀：{0}", [getErrorText(error)]);
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
      nowPlayingAppearanceError = tr('瀏覽器預覽不會保存正在播放外觀設定。');
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
      lyricsPreferencesError = tr("無法讀取歌詞設定：{0}", [getErrorText(error)]);
    }
  }

  function updateLyricsPreferences(patch: Partial<LyricsPreferences>, immediate = false): void {
    lyricsPreferences = normalizeLyricsPreferences({ ...lyricsPreferences, ...patch });
    lyricsPreferencesError = null;
    const revision = ++lyricsPreferencesRevision;
    if (!isTauri()) {
      lyricsPreferencesState = 'preview';
      lyricsPreferencesError = tr('瀏覽器預覽不會保存歌詞設定。');
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
        lyricsPreferencesError = tr("無法保存歌詞設定：{0}", [getErrorText(error)]);
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
      settingsRecoveryWarning = tr("無法確認設定檔狀態：{0}", [getErrorText(error)]);
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

  function updateQuickSettingsLook(patch: Partial<ThemePreferences>): void {
    const next = normalizeThemePreferences({ ...themePreferences, ...patch });
    themePreferences = next;
    themeSaveError = null;
    themeSaveState = isTauri() ? 'saving' : 'preview';
    const revision = ++themeRevision;
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = setTimeout(() => {
      themeSaveTimer = undefined;
      queueThemeSave(next, revision);
    }, 300);
  }

  function updateThemeColor(key: keyof ThemePreferences, value: string): void {
    if (!isHexColor(value)) return;
    const next = normalizeThemePreferences({ ...themePreferences, [key]: value });
    themePreferences = next;
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
      themeSaveError = tr('瀏覽器預覽不會保存外觀設定。');
      return;
    }

    themeSaveQueue = themeSaveQueue.then(async () => {
      if (revision !== themeRevision) return;
      themeSaveState = 'saving';
      try {
        const saved = await invokeCommand('theme_set_preferences', { preferences });
        if (revision !== themeRevision) return;
        themePreferences = normalizeThemePreferences(saved);
        themeSaveState = 'saved';
        themeSaveError = null;
      } catch (error) {
        if (revision !== themeRevision) return;
        themeSaveState = 'error';
        themeSaveError = tr("無法保存外觀設定：{0}", [getErrorText(error)]);
      }
    });
  }

  function resetThemePreferences(): void {
    themePreferences = { ...DEFAULT_THEME_PREFERENCES };
    themeSaveError = null;
    themeSaveState = isTauri() ? 'saving' : 'preview';
    const revision = ++themeRevision;
    if (themeSaveTimer !== undefined) clearTimeout(themeSaveTimer);
    themeSaveTimer = undefined;
    queueThemeSave({ ...themePreferences }, revision);
  }

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
    // The result of each source is its own line; there is no summary sentence above them.
    syncProgress = {
      ...current,
      sourceCount: event.sourceCount,
      sources,
      active: !isComplete,
      summary: null,
      error: null,
    };
    isSyncing = !isComplete;
  }

  function handleLibrarySyncFinished(event: LibrarySyncFinishedEvent): void {
    sourceSyncResults = event.sources;
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
      ? tr("同步失敗：{0}", [event.error])
      : event.sources.length === 0
        ? tr('尚未加入可同步的音樂來源。')
        : null;
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

  function syncProgressLine(progress: LibrarySyncProgressEvent): string {
    if (progress.stage === 'finished') {
      const outcome = progress.outcome === 'complete'
        ? tr('同步完成')
        : progress.outcome === 'cancelled'
          ? tr('已取消')
          : progress.outcome === 'unavailable'
            ? tr('來源暫不可用')
            : progress.outcome === 'permissionRevoked'
              ? tr('需要重新授權')
              : progress.outcome === 'failed'
                ? tr('同步失敗')
                : tr('掃描未完成');
      // "同步完成；新增 n 個項目，移除 n 個項目" (the errors only when there are some). The counts arrive with the
      // finished event; until then only the outcome shows.
      const result = sourceSyncResults.find((source) => source.sourceId === progress.sourceId);
      return formatSyncResultLine(outcome, result?.addedOrUpdated ?? null, result?.removedMappings ?? null, progress.errorCount);
    }
    if (progress.stage === 'enumerating') {
      const count = progress.processed.toLocaleString();
      return progress.total === null
        ? tr("掃描中，已檢查 {0} 個檔案系統項目；總數尚未確定", [count])
        : tr("掃描中，已檢查 {0} / {1} 個檔案系統項目", [count, progress.total.toLocaleString()]);
    }
    const stage = progress.stage === 'metadata' ? tr('讀取曲目資訊') : tr('寫入曲庫');
    const total = progress.total === null ? '—' : progress.total.toLocaleString();
    return tr("{0}，{1} / {2} 首曲目", [stage, progress.processed.toLocaleString(), total]);
  }

  async function loadCapabilities(): Promise<void> {
    runtimeError = null;
    try {
      capabilities = await invokeCommand('get_runtime_capabilities', {});
      if (isReady(capabilities.library)) void loadPlaylists();
      if (isReady(capabilities.library)) void loadLibrarySourceScope();
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
    libraryFieldFilter = null;
    if (activeView !== 'library') {
      activeView = 'library';
    }
  }

  async function syncLibrary(): Promise<void> {
    if (!sourceSyncReady) return;
    isSyncing = true;
    sourceError = null;
    sourceSyncResults = [];
    try {
      const result = await invokeCommand('library_sync', {});
      sourceSyncResults = result.sources;
      sourceSyncSummary = result.sources.length === 0 ? tr('尚未加入可同步的音樂來源。') : null;
      await loadSources();
      // Playlist-file orphan cleanup may delete sidebar projections during sync.
      if (libraryReady) await loadPlaylists(undefined, true);
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
      sourcesLoadedOnce = true;
      if (capabilities?.platform === 'windows' || capabilities?.platform === 'android') {
        excludedFolders = await invokeCommand('library_list_excluded_folders', {});
      } else {
        excludedFolders = [];
      }
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isLoadingSources = false;
    }
  }

  async function loadPlaylists(preferredId?: string, refreshSelectedEntries = false): Promise<void> {
    if (!libraryReady) return;
    const requestVersion = ++playlistListRequestVersion;
    isLoadingPlaylists = true;
    playlistError = null;
    try {
      const nextPlaylists = await invokeCommand('playlist_list', {});
      if (requestVersion !== playlistListRequestVersion) return;
      const previousPlaylistId = selectedPlaylistId;
      const previousEntryCount = playlists.find((playlist) => playlist.id === previousPlaylistId)?.entryCount ?? null;
      playlists = nextPlaylists;
      const requestedId = preferredId ?? selectedPlaylistId;
      const nextId = playlists.some((playlist) => playlist.id === requestedId)
        ? requestedId
        : playlists[0]?.id ?? null;
      selectedPlaylistId = nextId;
      const nextEntryCount = playlists.find((playlist) => playlist.id === nextId)?.entryCount ?? null;
      if (refreshSelectedEntries || previousPlaylistId !== nextId || previousEntryCount !== nextEntryCount) {
        playlistTrackCount = null;
        playlistListRevision += 1;
      }
    } catch (error) {
      if (requestVersion === playlistListRequestVersion) playlistError = getErrorText(error);
    } finally {
      if (requestVersion === playlistListRequestVersion) isLoadingPlaylists = false;
    }
  }

  async function selectPlaylist(playlistId: string): Promise<void> {
    const playlistChanged = selectedPlaylistId !== playlistId;
    selectedPlaylistId = playlistId;
    openPlaylistPage();
    if (playlistChanged) {
      playlistTrackCount = null;
      playlistListRevision += 1;
    }
  }

  function refreshPlaylistEntries(): void {
    playlistListRevision += 1;
  }

  function openPlaylistPage(): void {
    activeView = 'playlists';
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

  async function playQueueEntry(entry: PlaybackQueuePageItem): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = entry.trackId;
    const queueSource: PlaybackQueueSource = {
      kind: 'queue',
      traversalPosition: entry.traversalPosition,
    };
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: entry.trackId, queueSource }),
      { forceQueueCursorProbe: true },
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

  async function pickWindowsPlaylistSource(): Promise<void> {
    if (!playlistExchangeReady || !sourceSyncReady || isUpdatingSource || isPlaylistOperation) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      const result = await invokeCommand('playlist_import_m3u', {});
      if (!result) return;
      await loadPlaylists(result.playlist.id);
      await loadSources();
      sourceSyncSummary = tr("已加入播放清單「{0}」。", [result.playlist.name]);
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
      sourcesLoadedOnce = true;
      sourceSyncResults = sourceSyncResults.filter((result) => result.sourceId !== source.id);
      libraryTrackCount = null;
      libraryListRevision += 1;
      await loadPlaylists();
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
      sourcesLoadedOnce = true;
      sourceSyncResults = sourceSyncResults.filter((result) => result.sourceId !== source.id);
      libraryTrackCount = null;
      libraryListRevision += 1;
      if (source.kind === 'playlist_file') {
        await loadPlaylists();
        sourceSyncSummary = tr('已移除播放清單來源；側欄清單已刪除，曲庫中的曲目仍會保留。');
      } else {
        sourceSyncSummary = tr('已移除音樂來源；曲庫資料仍會保留，之後不再同步此位置。');
      }
    } catch (error) {
      sourceError = getErrorText(error);
      await loadSources();
    } finally {
      isUpdatingSource = false;
    }
  }

  async function pickExcludedFolder(): Promise<void> {
    if (!sourceSyncReady || isUpdatingSource || (capabilities?.platform !== 'windows' && capabilities?.platform !== 'android')) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      const excluded = await invokeCommand('library_pick_excluded_folder', {});
      if (excluded) {
        await loadSources();
        await syncLibrary();
      }
    } catch (error) {
      sourceError = getErrorText(error);
    } finally {
      isUpdatingSource = false;
    }
  }

  async function removeExcludedFolder(excluded: ExcludedFolder): Promise<void> {
    if (isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      excludedFolders = await invokeCommand('library_remove_excluded_folder', {
        excludedId: excluded.id,
      });
      sourceSyncSummary = tr('已移除排除資料夾；下次同步會重新掃入此位置的曲目。');
      await syncLibrary();
    } catch (error) {
      sourceError = getErrorText(error);
      await loadSources();
    } finally {
      isUpdatingSource = false;
    }
  }

  async function pickAndroidFolderSource(): Promise<void> {
    if (!sourceSyncReady || isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      const source = await invokeCommand('android_pick_folder_source', {});
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

  function sourceKindLabel(kind: string): string {
    switch (kind) {
      case 'playlist_file': return tr('播放清單檔案');
      case 'remote': return tr('遠端資料夾（SMB／WebDAV）');
      case 'androidFilesystem':
      case 'android_filesystem': return tr('Android 音樂資料夾');
      case 'androidMediaStore':
      case 'android_media_store': return tr('Android MediaStore（舊版，待遷移）');
      case 'androidSaf':
      case 'android_saf': return tr('Android 文件資料夾（舊版，待遷移）');
      default: return tr('Windows 音樂資料夾');
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
    const searchQuery = query.trim();
    let queryAlternates: string[] = [];
    try {
      queryAlternates = (await getSearchVariants(searchQuery)).slice(1);
    } catch {
      // Search conversion must never prevent playback of an already selected track.
    }
    if (!playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = track.id;
    const queueSource: PlaybackQueueSource = {
      kind: 'library',
      query: searchQuery || null,
      queryAlternates,
      fieldFilter: libraryFieldFilter ?? undefined,
      sort: librarySort,
    };
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: track.id, queueSource }),
      { refreshQueueList: true },
    );
  }

  async function resolveArtistEntryByKey(key: string): Promise<ArtistEntry | null> {
    return invokeCommand('library_resolve_artist_index_entry', {
      identityKey: key,
    });
  }

  async function resolveArtistEntryByName(name: string): Promise<ArtistEntry | null> {
    const normalizedName = name.trim();
    if (!normalizedName) return null;
    return invokeCommand('library_resolve_artist_index_entry_by_name', { name: normalizedName });
  }

  async function resolveAlbumEntryByKey(key: string): Promise<AlbumEntry | null> {
    return invokeCommand('library_resolve_album_index_entry', {
      identityKey: key,
    });
  }

  async function resolveAlbumEntryByName(name: string): Promise<AlbumEntry | null> {
    const normalizedName = name.trim();
    if (!normalizedName) return null;
    return invokeCommand('library_resolve_album_index_entry_by_name', { name: normalizedName });
  }

  function currentDetailNavigationRoute(): { view: View; artistKey?: string; albumKey?: string; preserveIndex?: boolean } {
    if (activeView === 'artist-detail' && selectedArtist) {
      return { view: activeView, artistKey: selectedArtist.key, preserveIndex: keepArtistIndexForDetail };
    }
    if (activeView === 'album-detail' && selectedAlbum) {
      return { view: activeView, albumKey: selectedAlbum.key, preserveIndex: keepAlbumIndexForDetail };
    }
    return { view: activeView };
  }

  function pushDetailNavigationRoute(route: { view: View; artistKey?: string; albumKey?: string; preserveIndex?: boolean }): void {
    // Save the route currently visible underneath this detail in the current
    // entry, then add the detail as a same-document browser history entry.
    window.history.replaceState(
      withDetailNavigationRoute(window.history.state, currentDetailNavigationRoute()),
      '',
    );
    window.history.pushState(withDetailNavigationRoute(window.history.state, route), '');
  }

  async function restoreDetailNavigationRoute(
    route: { view: View; artistKey?: string; albumKey?: string; preserveIndex?: boolean },
  ): Promise<void> {
    const revision = ++detailHistoryRestoreRevision;
    if (route.view === 'artist-detail' && route.artistKey) {
      try {
        const artist = await resolveArtistEntryByKey(route.artistKey);
        if (revision !== detailHistoryRestoreRevision) return;
        if (artist) {
          selectedArtist = artist;
          keepArtistIndexForDetail = route.preserveIndex ?? false;
          rememberedArtistView = 'artist-detail';
          activeView = 'artist-detail';
          return;
        }
      } catch (error) {
        console.error(tr('無法還原瀏覽歷史中的演出者明細'), error);
      }
      if (revision !== detailHistoryRestoreRevision) return;
      rememberedArtistView = 'artists';
      activeView = 'artists';
      return;
    }
    if (route.view === 'album-detail' && route.albumKey) {
      try {
        const album = await resolveAlbumEntryByKey(route.albumKey);
        if (revision !== detailHistoryRestoreRevision) return;
        if (album) {
          if (selectedAlbum?.key !== album.key) selectedAlbumInitialCoverSources = [];
          selectedAlbum = album;
          keepAlbumIndexForDetail = route.preserveIndex ?? false;
          rememberedAlbumView = 'album-detail';
          activeView = 'album-detail';
          return;
        }
      } catch (error) {
        console.error(tr('無法還原瀏覽歷史中的專輯明細'), error);
      }
      if (revision !== detailHistoryRestoreRevision) return;
      rememberedAlbumView = 'albums';
      activeView = 'albums';
      return;
    }

    if (route.view === 'artists') rememberedArtistView = 'artists';
    if (route.view === 'albums') rememberedAlbumView = 'albums';
    activeView = route.view;
  }

  function handleDetailHistoryPopState(event: PopStateEvent): void {
    const route = readDetailNavigationRoute(event.state);
    if (route) void restoreDetailNavigationRoute(route);
  }

  function openArtistEntry(artist: ArtistEntry, preserveIndex = false): void {
    pushDetailNavigationRoute({ view: 'artist-detail', artistKey: artist.key, preserveIndex });
    if (selectedArtist?.key !== artist.key) artistTrackCount = null;
    selectedArtist = artist;
    keepArtistIndexForDetail = preserveIndex;
    rememberedArtistView = 'artist-detail';
    activeView = 'artist-detail';
  }

  function navigateToArtistSection(): void {
    if (activeView === 'artist-detail') {
      rememberedArtistView = 'artists';
      activeView = 'artists';
      const route = readDetailNavigationRoute(window.history.state);
      if (route?.view === 'artist-detail' && route.artistKey === selectedArtist?.key) {
        window.history.replaceState(withDetailNavigationRoute(window.history.state, { view: 'artists' }), '');
      }
      return;
    }
    activeView = selectedArtist ? rememberedArtistView : 'artists';
  }

  function navigateToArtistIndex(): void {
    const route = readDetailNavigationRoute(window.history.state);
    if (route?.view === 'artist-detail' && route.artistKey === selectedArtist?.key) {
      window.history.back();
      return;
    }
    rememberedArtistView = 'artists';
    activeView = 'artists';
  }

  function openAlbumEntry(album: AlbumEntry, preserveIndex = false, initialCoverSources: string[] = []): void {
    pushDetailNavigationRoute({ view: 'album-detail', albumKey: album.key, preserveIndex });
    if (selectedAlbum?.key !== album.key) {
      albumTrackCount = null;
      selectedAlbumInitialCoverSources = [...initialCoverSources];
    } else if (initialCoverSources.length > 0) {
      selectedAlbumInitialCoverSources = [...initialCoverSources];
    }
    selectedAlbum = album;
    keepAlbumIndexForDetail = preserveIndex;
    rememberedAlbumView = 'album-detail';
    activeView = 'album-detail';
  }

  function navigateToAlbumSection(): void {
    if (activeView === 'album-detail') {
      rememberedAlbumView = 'albums';
      activeView = 'albums';
      const route = readDetailNavigationRoute(window.history.state);
      if (route?.view === 'album-detail' && route.albumKey === selectedAlbum?.key) {
        window.history.replaceState(withDetailNavigationRoute(window.history.state, { view: 'albums' }), '');
      }
      return;
    }
    activeView = selectedAlbum ? rememberedAlbumView : 'albums';
  }

  function navigateToAlbumIndex(): void {
    const route = readDetailNavigationRoute(window.history.state);
    if (route?.view === 'album-detail' && route.albumKey === selectedAlbum?.key) {
      window.history.back();
      return;
    }
    rememberedAlbumView = 'albums';
    activeView = 'albums';
  }

  async function openAlbumDetailForName(name: string): Promise<void> {
    const trimmedName = name.trim();
    if (!trimmedName) return;
    try {
      const album = await resolveAlbumEntryByName(trimmedName);
      if (!album) {
        if (isNowPlayingOpen) await closeNowPlaying();
        rememberedAlbumView = 'albums';
        activeView = 'albums';
        return;
      }
      const preserveIndex = activeView === 'albums' || (activeView === 'album-detail' && keepAlbumIndexForDetail);
      if (isNowPlayingOpen) await closeNowPlaying();
      openAlbumEntry(album, preserveIndex);
    } catch (error) {
      console.error(tr("無法開啟專輯「{0}」的明細", [trimmedName]), error);
    }
  }

  async function fetchAlbumPage(request: TrackPageRequest): Promise<TrackPage> {
    const album = selectedAlbum;
    if (!album) return { items: [], offset: request.offset, limit: request.limit, totalCount: 0 };
    const page = await invokeCommand('library_get_album_page', {
      albumNames: album.aliases,
      sort: request.sort,
      offset: request.offset,
      limit: request.limit,
    });
    albumTrackCount = page.totalCount;
    return page;
  }

  async function openArtistDetailForName(name: string): Promise<void> {
    const trimmedName = name.trim();
    if (!trimmedName) return;
    try {
      const artist = await resolveArtistEntryByName(trimmedName);
      if (!artist) {
        if (isNowPlayingOpen) await closeNowPlaying();
        rememberedArtistView = 'artists';
        activeView = 'artists';
        return;
      }
      if (isNowPlayingOpen) await closeNowPlaying();
      openArtistEntry(artist);
    } catch (error) {
      console.error(tr("無法開啟演出者「{0}」的明細", [trimmedName]), error);
    }
  }

  async function fetchArtistPage(request: TrackPageRequest): Promise<TrackPage> {
    const artist = selectedArtist;
    if (!artist) {
      return { items: [], offset: request.offset, limit: request.limit, totalCount: 0 };
    }
    const page = await invokeCommand('library_get_artist_page', {
      query: request.query,
      queryAlternates: request.queryAlternates ?? [],
      sort: request.sort ?? librarySort,
      offset: request.offset,
      limit: request.limit,
      artistNames: artist.artistNames,
    });
    artistTrackCount = page.totalCount;
    return page;
  }

  async function playArtistTrack(track: TrackSummary): Promise<void> {
    if (!selectedArtist || !playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = track.id;
    const queueSource: PlaybackQueueSource = {
      kind: 'library',
      query: null,
      queryAlternates: [],
      fieldFilter: { field: 'artist', value: selectedArtist.artistNames.join('|') },
      sort: librarySort,
    };
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: track.id, queueSource }),
      { refreshQueueList: true },
    );
  }

  async function playAlbumTrack(track: TrackSummary, order: AlbumTrackOrder): Promise<void> {
    if (!selectedAlbum || !playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = track.id;
    // The queue is built by the album page's own query and ordering (not the library list sort).
    const queueSource: PlaybackQueueSource = {
      kind: 'album',
      albumNames: [...selectedAlbum.aliases],
      sort: order.sort,
      indexSort: order.indexSort,
    };
    await sendPlaybackCommand(
      () => invokeCommand('playback_play', { trackId: track.id, queueSource }),
      { refreshQueueList: true },
    );
  }

  // Android system Back (MainActivity -> window.__moeHandleBack). Closes the top-most overlay,
  // then the playback page, then a detail page. Returns false at the root so native moves the
  // task to the background (playback continues).
  // Page to return to when leaving Settings via Back/Esc/mouse back (lists keep their own scroll cache).
  let viewBeforeSettings: View | null = null;
  let lastNonSettingsView: View = 'library';
  $effect(() => {
    if (activeView === 'settings') viewBeforeSettings = lastNonSettingsView;
    else lastNonSettingsView = activeView;
  });

  function handleAndroidBack(): boolean {
    if (folderPickerRequestId !== null) { finishFolderPicker(null); return true; }
    if (allFilesPromptOpen) { allFilesPromptOpen = false; return true; }
    if (androidSpectrogramTrackId) { androidSpectrogramTrackId = null; return true; }
    if (trackDetailsTrackId) { window.dispatchEvent(new CustomEvent('moe:track-details-request-close')); return true; }
    if (isSleepTimerOpen) { closeSleepTimerPopover(false); return true; }
    if (volumeExpanded) { volumeExpanded = false; volumePopoverLeftShiftPx = 0; return true; }
    if (isQuickSettingsOpen) { void closeQuickSettings(); return true; }
    if (isNavPanelOpen) { closeNavPanel('escape', false); return true; }
    if (isNowPlayingOpen) { void closeNowPlaying(); return true; }
    if (activeView === 'settings') { activeView = viewBeforeSettings ?? 'library'; return true; }
    if (activeView === 'album-detail') { navigateToAlbumIndex(); return true; }
    if (activeView === 'artist-detail') { navigateToArtistIndex(); return true; }
    return false;
  }
  if (typeof window !== 'undefined') (window as unknown as { __moeHandleBack?: () => boolean }).__moeHandleBack = handleAndroidBack;

  // Desktop: Esc and the mouse Back button share the same back stack (root = no-op).
  // Capture phase so the older per-overlay Esc handlers don't double-handle.
  $effect(() => {
    const isEditable = (el: Element | null): el is HTMLElement =>
      el instanceof HTMLElement && (el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName));
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.isComposing || event.repeat) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const active = document.activeElement;
      if (isEditable(active)) { active.blur(); return; }
      handleAndroidBack();
    };
    const blockMouseBack = (event: MouseEvent) => { if (event.button === 3) event.preventDefault(); };
    const onMouseBack = (event: MouseEvent) => {
      if (event.button !== 3) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      handleAndroidBack();
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('mousedown', blockMouseBack, true);
    window.addEventListener('mouseup', onMouseBack, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('mousedown', blockMouseBack, true);
      window.removeEventListener('mouseup', onMouseBack, true);
    };
  });

  async function openNowPlaying(): Promise<void> {
    if (!playback?.currentTrack || isNowPlayingOpen) return;
    nowPlayingReturnView = activeView;
    isNowPlayingMounted = true;
    isNowPlayingOpen = true;
    await tick();
    nowPlayingBackButton?.focus();
  }

  function toggleNowPlayingFromDock(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof Element)) return;
    if (target.closest('button:not(.dock-art), a, input, textarea, select, .progress-slider, .dock-controls, .volume-popover')) return;
    if (isNowPlayingOpen) void closeNowPlaying();
    else void openNowPlaying();
  }

  async function closeNowPlaying(): Promise<void> {
    if (!isNowPlayingOpen) return;
    isQuickSettingsOpen = false;
    activeView = nowPlayingReturnView;
    isNowPlayingOpen = false;
    await tick();
    dockArtworkButton?.focus();
  }

  function trackFieldValue(track: TrackSummary | null | undefined, field: TrackFieldFilter['field']): string | null {
    const value = track?.[field];
    return typeof value === 'string' && value.trim().length > 0 ? value : null;
  }

  function trackFieldLabel(track: TrackSummary | null | undefined, field: TrackFieldFilter['field']): string {
    if (field === 'title') return displayTrackTitle(track, '—');
    return trackFieldValue(track, field)?.trim() ?? '—';
  }

  const nowPlayingArtists = $derived(splitArtists(trackFieldValue(playback?.currentTrack, 'artist')));

  async function searchNowPlayingTitle(): Promise<void> {
    const title = displayTrackTitle(playback?.currentTrack, '').trim();
    if (!title) return;
    nowPlayingReturnView = 'library';
    query = title;
    libraryFieldFilter = null;
    libraryListRevision += 1;
    await closeNowPlaying();
    await tick();
    librarySearchInput?.focus();
  }

  async function openTrackField(field: TrackFieldFilter['field'], value?: string): Promise<void> {
    const resolved = (value ?? trackFieldValue(playback?.currentTrack, field))?.trim();
    if (!resolved) return;
    await closeNowPlaying();
    nowPlayingReturnView = 'library';
    activeView = 'library';
    query = '';
    libraryFieldFilter = { field, value: resolved };
    libraryListRevision += 1;
    await tick();
    librarySearchInput?.focus();
  }

  function selectSettingsSection(section: SettingsSection): void {
    settingsSection = section;
    if (quickSettingsScroll) quickSettingsScroll.scrollTop = 0;
  }

  async function openQuickSettings(): Promise<void> {
    if (isQuickSettingsOpen) return;
    isQuickSettingsOpen = true;
    await tick();
    if (quickSettingsScroll) quickSettingsScroll.scrollTop = 0;
    quickSettingsCloseButton?.focus();
  }

  async function closeQuickSettings(): Promise<void> {
    if (!isQuickSettingsOpen) return;
    isQuickSettingsOpen = false;
    await tick();
    quickSettingsTrigger?.focus();
  }

  function handleQuickSettingsKeydown(event: KeyboardEvent): void {
    if (document.querySelector('.update-dialog[open]')) return;
    if (!isQuickSettingsOpen || !quickSettingsDialog) return;
    if (event.key === 'Escape') {
      event.preventDefault();
      void closeQuickSettings();
      return;
    }
    if (event.key !== 'Tab') return;
    const focusable = Array.from(quickSettingsDialog.querySelectorAll<HTMLElement>(
      'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
    ));
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
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
    if (command === 'playback_previous' && currentPlaybackPositionMs() > 3000) {
      await sendPlaybackCommand(() => invokeCommand('playback_seek', { positionMs: 0 }));
      return;
    }
    await sendPlaybackCommand(() => invokeCommand(command, {}), {
      forceQueueCursorProbe: command === 'playback_next' || command === 'playback_previous',
    });
  }

  async function commitPlaybackSeek(positionMs: number): Promise<boolean> {
    if (!playbackReady || isSendingPlaybackCommand) return false;
    await sendPlaybackCommand(() => invokeCommand('playback_seek', { positionMs }));
    return playbackError === null;
  }

  function publishLivePlayCount(): void {
    rememberLivePlayCount(livePlayCountCache, toLivePlayCountView(livePlayCountTracker));
    livePlayCount = livePlayCountValues(livePlayCountCache);
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
    const previousLiveCount = toLivePlayCountView(livePlayCountTracker);
    if (previousLiveCount && previousLiveCount.trackId !== nextTrackId) {
      rememberLivePlayCount(livePlayCountCache, previousLiveCount);
    }
    const trackChanged = nextTrackId !== null && previousTrackId !== null && nextTrackId !== previousTrackId;
    const positionReset = nextTrackId !== null
      && previousTrackId === nextTrackId
      && previous !== null
      && previous.positionMs > resolved.positionMs + 50
      && resolved.positionMs < 1500;
    playback = resolved;
    updatePlaybackClock(resolved);
    if (Object.hasOwn(resolved, 'queueSort')) queueSort = resolved.queueSort ?? null;
    confirmedVolume = resolved.volume;
    if (trackChanged || positionReset || forceQueueCursorProbe) playbackQueueCursorChangeKey += 1;
    livePlayCountTracker = applyLivePlayCountSample(livePlayCountTracker, {
      trackId: resolved.currentTrack?.id ?? null,
      playedMs: resolved.currentTrack?.playedMs,
      positionMs: resolved.positionMs,
      isPlaying: resolved.isPlaying && resolved.state === 'playing',
    });
    if (trackChanged || previousTrackId !== nextTrackId || !resolved.currentTrack) {
      publishLivePlayCount();
    }
  }

  function updatePlaybackClock(snapshot: PlaybackSnapshot): void {
    const sample: TaskbarLyricsPlayback = {
      trackId: snapshot.currentTrack?.id ?? null,
      positionMs: snapshot.positionMs,
      durationMs: effectivePlaybackDurationMs(snapshot.durationMs, snapshot.currentTrack?.durationMs),
      state: snapshot.state,
      playing: snapshot.isPlaying && snapshot.state === 'playing',
    };
    playbackClock = correctPlaybackClock(untrack(() => playbackClock), sample, performance.now());
    lastTaskbarLyricsPlayback = sample;
    untrack(forwardTaskbarLyricsPlayback);
  }

  function forwardTaskbarLyricsPlayback(): void {
    const sample = lastTaskbarLyricsPlayback;
    if (!sample || !isTauri() || capabilities?.platform !== 'windows' || !taskbarLyricsSettings.enabled) return;
    void emitTo('taskbar-lyrics', 'taskbar-lyrics-playback', sample).catch(() => {});
  }

  /** The position now, from the clock (falls back to the last sample). */
  function currentPlaybackPositionMs(): number {
    return playbackClock ? estimatePlaybackPosition(playbackClock, performance.now()) : (playback?.positionMs ?? 0);
  }

  function requestPlaybackSample(): void {
    if (playbackReady && !isSendingPlaybackCommand) void loadPlaybackSnapshot(false);
  }

  // A playing clock that reaches the end of the track: the shell has most likely moved on, so sample at once.
  $effect(() => {
    const clock = playbackClock;
    if (!clock?.playing || clock.durationMs == null || clock.durationMs <= 0) return;
    const wait = msUntilPlaybackPosition(clock, performance.now(), clock.durationMs);
    if (wait === null) return;
    const timer = setTimeout(() => untrack(requestPlaybackSample), wait + 20);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (!isTauri()) return;
    const onVisible = () => { if (document.visibilityState === 'visible') requestPlaybackSample(); };
    const onFocus = () => requestPlaybackSample();
    document.addEventListener('visibilitychange', onVisible);
    window.addEventListener('focus', onFocus);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('audio-devices-changed', () => requestPlaybackSample())
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
      document.removeEventListener('visibilitychange', onVisible);
      window.removeEventListener('focus', onFocus);
    };
  });

  $effect(() => {
    if (!isTauri() || capabilities?.platform !== 'windows') return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('taskbar-lyrics-playback-request', () => untrack(forwardTaskbarLyricsPlayback))
      .then((stop) => { if (disposed) stop(); else unlisten = stop; })
      .catch(() => {});
    return () => { disposed = true; unlisten?.(); };
  });

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

  function toggleVolumeExpanded(event: MouseEvent): void {
    event.stopPropagation();
    volumeExpanded = !volumeExpanded;
    if (volumeExpanded) {
      void tick().then(updateVolumePopoverAlignment);
    } else {
      volumePopoverLeftShiftPx = 0;
    }
  }

  function updateVolumePopoverAlignment(): void {
    const anchor = volumePanelElement;
    const popover = volumePopoverElement;
    if (!volumeExpanded || !anchor || !popover) {
      volumePopoverLeftShiftPx = 0;
      return;
    }
    const anchorRect = anchor.getBoundingClientRect();
    volumePopoverLeftShiftPx = getVolumePopoverLeftShiftPx(
      anchorRect.left + anchorRect.width / 2,
      popover.getBoundingClientRect().width,
      window.innerWidth,
    );
  }

  function handleDockPointerDown(event: PointerEvent): void {
    if (!volumeExpanded || !volumePanelElement) return;
    const target = event.target;
    if (target instanceof Node && volumePanelElement.contains(target)) return;
    volumeExpanded = false;
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
    // The command's own snapshot can precede the change taking effect (a seek settling); sample once more now.
    requestPlaybackSample();
  }

  function capabilityLabel(capability: FeatureCapability | undefined): string {
    if (!capability) return capabilities?.platform === 'android' ? tr('等待 Android 服務') : tr('等待桌面服務');
    if (capability.state === 'ready') return tr('已就緒');
    if (capability.state === 'unavailable') return tr('目前無法使用');
    return tr('尚未就緒');
  }

  function showCapabilityDetail(capability: FeatureCapability | undefined): string {
    return capability?.detail ?? runtimeError ?? (
      capabilities?.platform === 'android'
        ? tr('請確認「所有檔案存取權」及音訊服務狀態。')
        : tr('請由 MoeMusicPlayer 桌面程式開啟。')
    );
  }

  function currentTrackTitle(track: TrackSummary | null | undefined): string {
    if (!track) return tr('尚未選擇曲目');
    return displayTrackTitle(track, tr('未命名曲目'));
  }

  function currentTrackArtist(track: TrackSummary | null | undefined): string {
    return track?.artist?.trim() || tr('選取曲庫中的曲目開始播放');
  }

  function formatSlideshowCountdown(totalSeconds: number): string {
    const minutes = Math.floor(totalSeconds / 60);
    const seconds = totalSeconds % 60;
    return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
  }

</script>

<svelte:window onpointerup={finishVolumeInteraction} onpointercancel={finishVolumeInteraction} onpointerdown={handleWindowPointerDown} onkeydown={handleWindowKeydown} onresize={updateVolumePopoverAlignment} />

{#snippet settingsTabs(scope: string)}
            <div class="settings-tabs" role="tablist" aria-label={tr("設定分類")}>
              <button
                id="{scope}appearance-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'appearance'}
                aria-controls="{scope}appearance-panel"
                onclick={() => selectSettingsSection('appearance')}
              >{tr("主介面")}</button>
              <button
                id="{scope}track-columns-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'track-columns'}
                aria-controls="{scope}track-columns-panel"
                onclick={() => selectSettingsSection('track-columns')}
              >{tr("曲目欄位")}</button>
              <button
                id="{scope}now-playing-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'now-playing'}
                aria-controls="{scope}now-playing-layout-panel"
                onclick={() => selectSettingsSection('now-playing')}
              >{tr("播放頁")}</button>
              <button
                id="{scope}lyrics-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'lyrics'}
                aria-controls="{scope}lyrics-panel"
                onclick={() => selectSettingsSection('lyrics')}
              >{tr("歌詞")}</button>
              {#if capabilities?.platform === 'windows' || capabilities?.platform === 'android'}
                <button
                  id="{scope}playback-tab"
                  class="soft-button settings-tab"
                  type="button"
                  role="tab"
                  aria-selected={settingsSection === 'playback'}
                  aria-controls="{scope}playback-panel"
                  onclick={() => selectSettingsSection('playback')}
                >{tr("輸出")}</button>
              {/if}
              <button
                id="{scope}shortcuts-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'shortcuts'}
                aria-controls="{scope}shortcuts-panel"
                onclick={() => selectSettingsSection('shortcuts')}
              >{tr("快捷鍵")}</button>
              <button
                id="{scope}sources-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'sources'}
                aria-controls="{scope}sources-panel"
                onclick={() => selectSettingsSection('sources')}
              >{tr("音樂來源")}</button>
              <button
                id="{scope}about-tab"
                class="soft-button settings-tab"
                type="button"
                role="tab"
                aria-selected={settingsSection === 'about'}
                aria-controls="{scope}about-panel"
                onclick={() => selectSettingsSection('about')}
              >{tr("關於")}</button>
            </div>
{/snippet}

{#snippet settingsPanels(scope: string, options?: { includeTabs?: boolean })}
            {#if options?.includeTabs !== false}
              {@render settingsTabs(scope)}
            {/if}

            {#if settingsRecoveryWarning}
              <div class="source-error-message" role="status">
                {tr("設定檔修復通知：")}{settingsRecoveryWarning}
                {#if !settingsSourceRegistryAuthoritative}
                  <p>{tr("為保留原有曲庫，來源同步已暫停。請重新登記全部音樂資料夾與播放清單檔案，再確認恢復同步。")}</p>
                  <button class="text-button" type="button" disabled={isUpdatingSource} onclick={() => void confirmSourceRegistry()}>{tr("我已確認來源清單，恢復同步")}</button>
                {/if}
              </div>
            {/if}

            {#if settingsSection === 'appearance'}
              <div id="{scope}appearance-panel" class="settings-panel main-interface-panel" role="tabpanel" aria-labelledby="{scope}appearance-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("顏色")}</h3>
                    <p>{tr("選擇文字顏色或圖片透明度的自動調整方式，並自訂主色。")}</p>
                  </div>
                  <button class="soft-button" type="button" onclick={resetThemePreferences}>{tr("恢復預設")}</button>
                </div>
                <div class="appearance-color-grid">
                  <label class="theme-color-control">
                    <input
                      type="color"
                      aria-label={tr("主色")}
                      value={themePreferences.accentHex}
                      oninput={(event) => updateThemeColor('accentHex', event.currentTarget.value)}
                      onchange={saveThemePreferencesNow}
                    />
                    <span class="theme-color-copy">
                      <strong>{tr("主色")}</strong>
                      <code>{themePreferences.accentHex}</code>
                    </span>
                  </label>
                </div>
                <label class="lyrics-preference-range">
                  <span><strong>{tr("框線透明度")}</strong><output>{themePreferences.borderOpacityPercent}%</output></span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    value={themePreferences.borderOpacityPercent}
                    aria-label={tr("框線透明度")}
                    oninput={(event) => updateMainBackgroundLook({ borderOpacityPercent: Number(event.currentTarget.value) })}
                    onchange={saveThemePreferencesNow}
                  />
                </label>
                <label class="lyrics-preference-range">
                  <span><strong>{tr("分隔線透明度")}</strong><output>{themePreferences.dividerOpacityPercent}%</output></span>
                  <input
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    value={themePreferences.dividerOpacityPercent}
                    aria-label={tr("分隔線透明度")}
                    oninput={(event) => updateMainBackgroundLook({ dividerOpacityPercent: Number(event.currentTarget.value) })}
                    onchange={saveThemePreferencesNow}
                  />
                </label>
                <div class="color-mode-settings">
                  <div class="color-mode-picker" role="group" aria-label={tr("顏色模式")}>
                    <button class="soft-button" class:is-active={themePreferences.colorMode === 'autoTextColor'} type="button" aria-pressed={themePreferences.colorMode === 'autoTextColor'} onclick={() => { updateMainBackgroundLook({ colorMode: 'autoTextColor' }); saveThemePreferencesNow(); }}>{tr("自動文字顏色")}</button>
                    <button class="soft-button" class:is-active={themePreferences.colorMode === 'autoBackgroundOpacity'} type="button" aria-pressed={themePreferences.colorMode === 'autoBackgroundOpacity'} onclick={() => { updateMainBackgroundLook({ colorMode: 'autoBackgroundOpacity' }); saveThemePreferencesNow(); }}>{tr("自動背景透明度")}</button>
                  </div>
                  <div class="appearance-color-grid">
                    {#if themePreferences.colorMode === 'autoTextColor'}
                      <label class="theme-color-control">
                        <input
                          type="color"
                          aria-label={tr("背景色")}
                          value={themePreferences.backgroundHex}
                          oninput={(event) => updateThemeColor('backgroundHex', event.currentTarget.value)}
                          onchange={saveThemePreferencesNow}
                        />
                        <span class="theme-color-copy">
                          <strong>{tr("背景色")}</strong>
                          <code>{themePreferences.backgroundHex}</code>
                        </span>
                      </label>
                    {:else}
                      <div class="cover-corner-setting" role="group" aria-label={tr("文字顏色")}>
                        <span class="cover-corner-setting-label"><strong>{tr("文字顏色")}</strong></span>
                        <div class="text-color-choice-row">
                          <button class="soft-button" class:is-active={themePreferences.backgroundOpacityTextColor === 'white'} type="button" aria-pressed={themePreferences.backgroundOpacityTextColor === 'white'} onclick={() => { updateMainBackgroundLook({ backgroundOpacityTextColor: 'white' }); saveThemePreferencesNow(); }}>{tr("白色字")}</button>
                          <button class="soft-button" class:is-active={themePreferences.backgroundOpacityTextColor === 'black'} type="button" aria-pressed={themePreferences.backgroundOpacityTextColor === 'black'} onclick={() => { updateMainBackgroundLook({ backgroundOpacityTextColor: 'black' }); saveThemePreferencesNow(); }}>{tr("黑色字")}</button>
                        </div>
                        <p class="source-muted-note">{tr("背景色固定為{0}；主介面與播放頁各自依照清晰度設定調整背景圖片透明度。", [themePreferences.backgroundOpacityTextColor === 'white' ? tr('純黑') : tr('純白')])}</p>
                      </div>
                    {/if}
                  </div>
                  {#if themePreferences.colorMode === 'autoTextColor'}
                    <label class="lyrics-preference-range">
                      <span><strong>{tr("自動切換文字顏色閥值")}</strong><output>{themePreferences.autoTextColorThresholdPercent}%</output></span>
                      <input
                        type="range"
                        min="0"
                        max="100"
                        step="1"
                        value={themePreferences.autoTextColorThresholdPercent}
                        aria-label={tr("自動切換文字顏色閥值")}
                        oninput={(event) => updateMainBackgroundLook({ autoTextColorThresholdPercent: Number(event.currentTarget.value) })}
                        onchange={saveThemePreferencesNow}
                      />
                    </label>
                  {/if}
                </div>
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("主介面背景")}</h3>
                    <p>{tr("選擇主介面的背景來源。模糊與透明度不會改變播放頁封面背景。")}</p>
                  </div>
                </div>
                <div class="main-background-mode-picker" role="group" aria-label={tr("主介面背景模式")}>
                  {#each [
                    { mode: 'solid', label: tr('純色') },
                    { mode: 'singleImage', label: tr('單圖') },
                    { mode: 'slideshow', label: tr('幻燈片') },
                    { mode: 'albumCover', label: tr('專輯封面') },
                    { mode: 'systemWallpaper', label: tr('系統桌布') },
                  ] as option (option.mode)}
                    <button
                      class="soft-button"
                      class:is-active={themePreferences.mainBackgroundMode === option.mode}
                      type="button"
                      aria-pressed={themePreferences.mainBackgroundMode === option.mode}
                      onclick={() => void selectMainBackgroundMode(option.mode as ThemePreferences['mainBackgroundMode'])}
                    >{option.label}</button>
                  {/each}
                </div>
                {#if themePreferences.mainBackgroundMode === 'singleImage'}
                  <div class="source-action-row source-action-buttons">
                    <button class="soft-button" type="button" onclick={() => void pickMainBackground()} disabled={mainBackgroundBusy}>
                      {mainBackgroundBusy ? tr('處理中') : tr('選擇圖片')}
                    </button>
                    <button class="soft-button" type="button" onclick={() => void clearMainBackground()} disabled={mainBackgroundBusy || !mainBackgroundUrl}>
                      {tr("移除圖片")}
                    </button>
                  </div>
                  {#if !mainBackgroundUrl}<p class="source-muted-note">{tr("尚未選擇圖片，將顯示純色背景。")}</p>{/if}
                {:else if themePreferences.mainBackgroundMode === 'slideshow'}
                  <div class="source-action-row source-action-buttons">
                    <button class="soft-button" type="button" onclick={() => void pickSlideshowFolder()} disabled={mainBackgroundBusy}>
                      {mainBackgroundBusy ? tr('處理中') : tr('選擇資料夾')}
                    </button>
                  </div>
                  <p class="source-muted-note slideshow-folder-note">
                    <span>{formatSlideshowFolderLabel(themePreferences.slideshowFolder) ?? tr('尚未選擇資料夾，請選擇包含圖片的資料夾。')}</span>
                    {#if themePreferences.slideshowFolder && slideshowNextInSeconds !== null}
                      <span class="slideshow-countdown">{tr("下一張")} {formatSlideshowCountdown(slideshowNextInSeconds)}</span>
                    {/if}
                  </p>
                  <div class="main-background-mode-picker" role="group" aria-label={tr("幻燈片播放順序")}>
                    <button class="soft-button" class:is-active={themePreferences.slideshowOrder === 'sequential'} type="button" aria-pressed={themePreferences.slideshowOrder === 'sequential'} onclick={() => { updateMainBackgroundLook({ slideshowOrder: 'sequential' }); saveThemePreferencesNow(); }}>{tr("順序播放")}</button>
                    <button class="soft-button" class:is-active={themePreferences.slideshowOrder === 'random'} type="button" aria-pressed={themePreferences.slideshowOrder === 'random'} onclick={() => { updateMainBackgroundLook({ slideshowOrder: 'random' }); saveThemePreferencesNow(); }}>{tr("隨機播放")}</button>
                  </div>
                  <label class="lyrics-preference-range">
                    <span><strong>{tr("切換時間")}</strong><output>{themePreferences.slideshowIntervalMinutes} {tr("分鐘")}</output></span>
                    <input type="range" min="1" max="120" step="1" value={themePreferences.slideshowIntervalMinutes} aria-label={tr("幻燈片切換時間")} oninput={(event) => updateMainBackgroundLook({ slideshowIntervalMinutes: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                  </label>
                {:else if themePreferences.mainBackgroundMode === 'albumCover'}
                  <p class="source-muted-note">{tr("背景會跟隨目前播放歌曲的專輯封面；沒有可用封面時暫時顯示純色。")}</p>
                {:else if themePreferences.mainBackgroundMode === 'systemWallpaper'}
                  {#if capabilities?.platform === 'android' && androidWallpaperHasImage}
                    <p class="source-muted-note">{tr("已透過「所有檔案存取權」讀取目前的系統桌布，並每分鐘檢查是否已變更，下次檢查 {0}s。", [systemWallpaperNextInSeconds ?? 60])}</p>
                  {:else if capabilities?.platform === 'android'}
                    <p class="source-muted-note">{allFilesAccess?.supported && !allFilesAccess.granted ? tr('Android 13 以上須開啟「音樂來源」中的「所有檔案存取權」才能讀取系統桌布；未授權時顯示純色背景。') : tr('無法讀取系統桌布（動態桌布沒有可讀取的圖片），目前顯示純色背景。')}</p>
                  {:else}
                    <p class="source-muted-note">{tr("讀取 Windows 目前的系統桌布，並每分鐘檢查是否已變更，下次檢查 {0}s。", [systemWallpaperNextInSeconds ?? 60])}</p>
                  {/if}
                {/if}
                {#if themePreferences.mainBackgroundMode !== 'solid'}
                  <label class="lyrics-preference-range">
                    <span><strong>{tr("背景模糊")}</strong><output>{themePreferences.mainBackgroundBlurPx}px</output></span>
                    <input type="range" min="0" max="50" step="1" value={themePreferences.mainBackgroundBlurPx} aria-label={tr("主介面背景模糊程度")} oninput={(event) => updateMainBackgroundLook({ mainBackgroundBlurPx: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                  </label>
                  {#if themePreferences.colorMode === 'autoBackgroundOpacity'}
                    <label class="lyrics-preference-range">
                      <span><strong>{tr("清晰度")}</strong><output>{themePreferences.mainBackgroundClarityPercent}%</output></span>
                      <input type="range" min="0" max="100" step="1" value={themePreferences.mainBackgroundClarityPercent} aria-label={tr("主介面背景清晰度")} oninput={(event) => updateMainBackgroundLook({ mainBackgroundClarityPercent: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                    </label>
                  {/if}
                  <label class="lyrics-preference-range">
                    <span><strong>{tr("背景透明度")}</strong>{#if themePreferences.colorMode !== 'autoBackgroundOpacity'}<output>{themePreferences.mainBackgroundOpacityPercent}%</output>{/if}</span>
                    {#if themePreferences.colorMode === 'autoBackgroundOpacity'}
                      <OpacityCalculation calculation={mainBackgroundOpacityCalculation} description={tr("依目前背景圖片的平均亮度與文字顏色自動計算。")} emptyMessage={tr("沒有可讀取的背景圖片，使用固定底色。")} />
                    {:else}
                      <input type="range" min="0" max="100" step="1" value={themePreferences.mainBackgroundOpacityPercent} aria-label={tr("主介面背景透明度")} oninput={(event) => updateMainBackgroundLook({ mainBackgroundOpacityPercent: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                    {/if}
                  </label>
                {/if}
                {#if mainBackgroundError}
                  <p class="theme-save-status error" role="alert">{mainBackgroundError}</p>
                {/if}
                <div class="settings-panel-header language-heading">
                  <div>
                    <h3>{tr("語言")}</h3>
                    <p>{tr("切換語言後會重新載入介面。")}</p>
                  </div>
                </div>
                <div class="cover-corner-setting" role="group" aria-label={tr("語言")}>
                  <div class="layout-choice-row lyrics-text-effect-row language-choice-row">
                    {#each LANGUAGE_CHOICES as choice (choice)}
                      <button
                        type="button"
                        aria-pressed={languageChoice === choice}
                        class:chosen={languageChoice === choice}
                        onclick={() => void chooseLanguage(choice as LanguageChoice)}
                      >{choice === 'system' ? tr("跟隨系統") : LANGUAGE_NAMES[choice]}</button>
                    {/each}
                  </div>
                </div>
                {#if languageError}
                  <p class="theme-save-status error" role="alert">{languageError}</p>
                {/if}
              </div>
            {:else if settingsSection === 'track-columns'}
              <div id="{scope}track-columns-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}track-columns-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("曲目欄位與列表高度")}</h3>
                    <p>{tr("曲庫、播放清單與播放佇列共用欄位順序、顯示、高度及分組設定；序號與播放操作固定在兩側。")}</p>
                  </div>
                </div>
                <label class="lyrics-preference-range track-row-height-setting">
                  <span><strong>{tr("列表高度")}</strong><output>{trackRowHeightPx}px</output></span>
                  <input
                    type="range"
                    min="30"
                    max="60"
                    step="1"
                    value={trackRowHeightPx}
                    aria-label={tr("列表高度")}
                    oninput={(event) => previewTrackRowHeight(Number(event.currentTarget.value))}
                    onchange={(event) => commitTrackRowHeight(Number(event.currentTarget.value))}
                  />
                </label>
                <label class="lyrics-preference-range track-row-height-setting">
                  <span><strong>{tr("專輯卡片最大寬度")}</strong><output>{albumCardMaxWidthPx}px</output></span>
                  <input
                    type="range"
                    min={MIN_ALBUM_CARD_MAX_WIDTH_PX}
                    max={MAX_ALBUM_CARD_MAX_WIDTH_PX}
                    step="10"
                    value={albumCardMaxWidthPx}
                    aria-label={tr("專輯卡片最大寬度")}
                    oninput={(event) => previewAlbumCardMaxWidth(Number(event.currentTarget.value))}
                    onchange={(event) => commitAlbumCardMaxWidth(Number(event.currentTarget.value))}
                  />
                </label>
                <label class="lyrics-preference-toggle track-grouping-setting">
                  <input type="checkbox" checked={groupingEnabled} aria-label={tr("啟用分組模式")} onchange={(event) => setGroupingEnabled(event.currentTarget.checked)} />
                  <span><strong>{tr("啟用分組模式")}</strong><small>{tr("啟用後可能增加載入時間")}</small></span>
                </label>
                <div class="track-text-overflow-setting" role="group" aria-label={tr("欄位文字不足時的隱藏方式")}>
                  <button class="soft-button" class:is-active={trackTextOverflowMode === 'middle'} type="button" aria-pressed={trackTextOverflowMode === 'middle'} onclick={() => persistTrackColumnSettings({ textOverflowMode: 'middle' })}>{tr("隱藏中段")}</button>
                  <button class="soft-button" class:is-active={trackTextOverflowMode === 'end'} type="button" aria-pressed={trackTextOverflowMode === 'end'} onclick={() => persistTrackColumnSettings({ textOverflowMode: 'end' })}>{tr("隱藏後段")}</button>
                </div>
                <ol class="track-column-settings" use:longPressDragGuard={() => trackColumnOrderDrag !== null} aria-label={tr("曲目資訊欄位設定")}>
                  {#each trackColumnPreferences as preference, index (preference.id)}
                    {@const definition = TRACK_COLUMN_DEFINITIONS.find((column) => column.id === preference.id)!}
                    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
                    <li
                      class="track-column-setting-row"
                      class:column-order-dragging={trackColumnOrderDrag?.fromIndex === index}
                      class:column-order-drop-before={trackColumnOrderDrag !== null && trackColumnOrderDrag.toIndex === index && trackColumnOrderDrag.fromIndex > index}
                      class:column-order-drop-after={trackColumnOrderDrag !== null && trackColumnOrderDrag.toIndex === index && trackColumnOrderDrag.fromIndex < index}
                      data-track-column-index={index}
                      use:appTooltip={tr('長按 300 毫秒後拖曳排序；鍵盤可用上下方向鍵')}
                      onpointerdown={(event) => handleTrackColumnOrderPointerDown(event, index)}
                      onpointermove={handleTrackColumnOrderPointerMove}
                      onpointerup={finishTrackColumnOrderPointer}
                      onpointercancel={cancelTrackColumnOrderPointer}
                    >
                      <label class="track-column-setting-label">
                        <input
                          type="checkbox"
                          checked={preference.visible}
                          aria-label={tr("顯示{0}欄", [definition.label])}
                          use:appTooltip={tr('上下方向鍵可調整欄位順序')}
                          onchange={(event) => setTrackColumnVisible(preference.id, event.currentTarget.checked)}
                          onkeydown={(event) => handleTrackColumnOrderKeydown(event, preference.id)}
                        />
                        <span>{definition.label}</span>
                      </label>
                      <div class="track-column-order-actions">
                        <span aria-label={tr("第 {0} 欄", [index + 1])}>{index + 1}</span>
                      </div>
                    </li>
                  {/each}
                </ol>
              </div>
            {:else if settingsSection === 'playback'}
              <div id="{scope}playback-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}playback-tab" tabindex="0">
                {#if capabilities?.platform === 'windows'}
                <OutputDevicesSettings outputFormat={playback?.outputFormat ?? null} onExclusiveChange={(value) => (outputExclusive = value)} />
                <div class="resampling-section" class:is-inactive={outputExclusive} inert={outputExclusive}>
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("DSEE HX")}</h3>
                  </div>
                </div>
                <div class="resampling-stack" role="group" aria-label={tr("DSEE HX")}>
                  <label class="resampling-option">
                    <input
                      type="checkbox"
                      checked={dseeHx}
                      disabled={dseeHxState === 'loading' || dseeHxState === 'saving'}
                      onchange={(event) => setDseeHx(event.currentTarget.checked)}
                    />
                    <span class="resampling-option-copy">
                      <strong>{tr("啟用 DSEE HX")}</strong>
                      <small>{tr("串流透過 Sony Music Center 濾鏡處理，需要先安裝 Sony Music Center，播放器載入 'C:\\Program Files (x86)\\Sony\\Music Center\\Sony.Earth\\OmgDseeHxFilter.ax'，本專案不會散佈這個檔案。48 kHz 及以下的雙聲道音樂自動套用DSEE HX效果，高於 48 kHz 或非雙聲道不處理。")}</small>
                    </span>
                  </label>
                </div>
                </div>
                {#if outputExclusive}
                  <p class="dsd-setting-hint">{tr("獨佔模式不使用 DSEE HX；裝置不接受檔案的取樣率時，播放器會自動轉換成裝置接受的最接近取樣率。")}</p>
                {/if}
                {/if}
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("DSD 播放")}</h3>
                    <p>{tr("DSD（.dsf）會先在播放器內轉成 PCM 再輸出。增益補償 DSD 較低的標稱電平；播放中調整增益會平滑漸變，不會突然變大聲。")}{#if capabilities?.platform === 'android'}{tr("Android 的音訊輸出上限為 192 kHz，所以輸出取樣率固定為 176.4 kHz。")}{:else}{tr("改變輸出取樣率時，正在播放的 DSD 曲目會從目前位置重新開始。")}{/if}</p>
                  </div>
                </div>
                <div class="dsd-settings" role="group" aria-label={tr("DSD 播放設定")}>
                  <div class="dsd-setting-row" role="group" aria-label={tr("DSD 增益")}>
                    <span class="dsd-setting-label"><strong>{tr("增益")}</strong><output>{formatDsdGain(dsdSettings.gainDb)}</output></span>
                    <input
                      class="dsd-gain-slider"
                      type="range"
                      min={DSD_GAIN_MIN_DB}
                      max={DSD_GAIN_MAX_DB}
                      step={DSD_GAIN_STEP_DB}
                      value={dsdSettings.gainDb}
                      aria-label={tr("DSD 增益")}
                      aria-valuetext={formatDsdGain(dsdSettings.gainDb)}
                      disabled={dsdSettingsState === 'loading'}
                      oninput={(event) => setDsdSettings({ gainDb: Number(event.currentTarget.value) })}
                    />
                  </div>
                  <div class="dsd-setting-row" role="group" aria-label={tr("DSD 輸出取樣率")}>
                    <span class="dsd-setting-label"><strong>{tr("輸出取樣率")}</strong><output>{formatDsdOutputRate(dsdEffectiveRateHz)}</output></span>
                    <div class="dsd-choice-row">
                      {#each DSD_OUTPUT_RATES_HZ as rate (rate)}
                        <button
                          type="button"
                          class="soft-button"
                          class:is-active={dsdEffectiveRateHz === rate}
                          aria-pressed={dsdEffectiveRateHz === rate}
                          aria-label={tr("DSD 輸出取樣率 {0}", [formatDsdOutputRate(rate)])}
                          disabled={dsdSettingsState === 'loading' || (capabilities?.platform === 'android' && rate !== 176_400)}
                          onclick={() => setDsdSettings({ outputRateHz: rate })}
                        >{formatDsdOutputRate(rate)}</button>
                      {/each}
                    </div>
                  </div>
                  {#if capabilities?.platform !== 'android'}
                    <p class="dsd-setting-hint">{tr("352.8 kHz 保留較多超音波成分，峰值通常比 176.4 kHz 高約 0 到 2 dB；增益較高時請留意削波。")}</p>
                    <label class="resampling-option">
                      <input
                        type="checkbox"
                        checked={dsdSettings.dop}
                        disabled={dsdSettingsState === 'loading'}
                        onchange={(event) => setDsdSettings({ dop: event.currentTarget.checked })}
                      />
                      <span class="resampling-option-copy">
                        <strong>{tr("使用 DoP 輸出 DSD（獨佔模式）")}</strong>
                        <small>{outputExclusive
                          ? tr("DSD 以 DoP 原樣送給 DAC，不轉成 PCM；上面的增益與輸出取樣率、播放器音量都不再適用，音量請在 DAC 或擴大機調整。裝置必須接受 24-bit 的 DoP 取樣率（DSD64 為 176.4 kHz，DSD128 為 352.8 kHz，DSD256 為 705.6 kHz），否則該曲自動改為轉換成 PCM。只支援雙聲道 DSD64、DSD128、DSD256。")
                          : tr("DoP 只在獨佔模式下使用；請先在上方「輸出模式」選擇獨佔。")}</small>
                      </span>
                    </label>
                  {/if}
                  {#if dsdSettingsError}<p class="source-error-message" role="alert">{dsdSettingsError}</p>{/if}
                </div>
              </div>
            {:else if settingsSection === 'now-playing'}
              <div id="{scope}now-playing-layout-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}now-playing-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("封面與歌詞排列")}</h3>
                  </div>
                </div>
                <NowPlayingQuickSettingsControls
                  groups="playback"
                  showGroupHeading={false}
                  layout={nowPlayingLayout}
                  appearance={nowPlayingAppearancePreferences}
                  autoBackgroundOpacity={themePreferences.colorMode === 'autoBackgroundOpacity'}
                  coverClarityPercent={themePreferences.coverBackgroundClarityPercent}
                  onCoverClarityChange={updateCoverBackgroundClarity}
                  opacityCalculation={nowPlayingBackgroundOpacityCalculation}
                  lyrics={lyricsPreferences}
                  onLayoutChange={setNowPlayingLayout}
                  onAppearanceChange={updateNowPlayingAppearancePreferences}
                  onLyricsChange={updateLyricsPreferences}
                  onLyricsReset={() => updateLyricsPreferences(DEFAULT_LYRICS_PREFERENCES, true)}
                />
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("快速設定面板")}</h3>
                    <p>{tr("調整播放頁右側快速設定抽屜的底色不透明度與背景模糊。底色與文字相反。")}</p>
                  </div>
                </div>
                <label class="lyrics-preference-range">
                  <span><strong>{tr("面板透明度")}</strong><output>{themePreferences.quickSettingsOpacityPercent}%</output></span>
                  <input type="range" min="0" max="100" step="1" value={themePreferences.quickSettingsOpacityPercent} aria-label={tr("快速設定面板透明度")} oninput={(event) => updateQuickSettingsLook({ quickSettingsOpacityPercent: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                </label>
                <label class="lyrics-preference-range">
                  <span><strong>{tr("面板模糊")}</strong><output>{themePreferences.quickSettingsBlurPx}px</output></span>
                  <input type="range" min="0" max="50" step="1" value={themePreferences.quickSettingsBlurPx} aria-label={tr("快速設定面板模糊程度")} oninput={(event) => updateQuickSettingsLook({ quickSettingsBlurPx: Number(event.currentTarget.value) })} onchange={saveThemePreferencesNow} />
                </label>
              </div>
            {:else if settingsSection === 'lyrics'}
              <div id="{scope}lyrics-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}lyrics-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("歌詞顯示")}</h3>
                  </div>
                </div>
                <NowPlayingQuickSettingsControls
                  groups="lyrics"
                  showGroupHeading={false}
                  layout={nowPlayingLayout}
                  appearance={nowPlayingAppearancePreferences}
                  autoBackgroundOpacity={themePreferences.colorMode === 'autoBackgroundOpacity'}
                  coverClarityPercent={themePreferences.coverBackgroundClarityPercent}
                  onCoverClarityChange={updateCoverBackgroundClarity}
                  opacityCalculation={nowPlayingBackgroundOpacityCalculation}
                  lyrics={lyricsPreferences}
                  onLayoutChange={setNowPlayingLayout}
                  onAppearanceChange={updateNowPlayingAppearancePreferences}
                  onLyricsChange={updateLyricsPreferences}
                  onLyricsReset={() => updateLyricsPreferences(DEFAULT_LYRICS_PREFERENCES, true)}
                />
                {#if capabilities?.platform === 'windows'}
                  <div class="settings-panel-header taskbar-lyrics-heading">
                    <div>
                      <h3>{tr("工作列歌詞")}</h3>
                      <p>{tr("在工作列沒被工作鈕和系統匣佔用的空白處顯示目前這一句歌詞（只顯示有時間軸的歌詞）。工作列自動隱藏、有全螢幕程式或空白處太窄時不顯示。")}</p>
                    </div>
                  </div>
                  <div class="taskbar-lyrics-settings" role="group" aria-label={tr("工作列歌詞設定")}>
                    <label class="lyrics-preference-toggle">
                      <input type="checkbox" checked={taskbarLyricsSettings.enabled} onchange={(event) => setTaskbarLyricsSettings({ enabled: event.currentTarget.checked })} />
                      <span><strong>{tr("顯示工作列歌詞")}</strong><small>{tr("在工作列空白處繪製目前播放的歌詞")}</small></span>
                    </label>
                    <div class="cover-corner-setting" role="group" aria-labelledby="{scope}taskbar-lyrics-align-label">
                      <span id="{scope}taskbar-lyrics-align-label" class="cover-corner-setting-label"><strong>{tr("對齊")}</strong></span>
                      <div class="layout-choice-row lyrics-text-effect-row">
                        {#each [['left', tr('靠左')], ['center', tr('置中')], ['right', tr('靠右')]] as [value, label] (value)}
                          <button
                            type="button"
                            aria-pressed={taskbarLyricsSettings.align === value}
                            class:chosen={taskbarLyricsSettings.align === value}
                            onclick={() => setTaskbarLyricsSettings({ align: value as TaskbarLyricsSettings['align'] })}
                          >{label}</button>
                        {/each}
                      </div>
                    </div>
                    <label class="lyrics-preference-range">
                      <span><strong>{tr("字體大小")}</strong><output>{taskbarLyricsSettings.fontSizePx} px</output></span>
                      <input
                        type="range"
                        min={TASKBAR_LYRICS_FONT_SIZE_PX.min}
                        max={TASKBAR_LYRICS_FONT_SIZE_PX.max}
                        step="1"
                        value={taskbarLyricsSettings.fontSizePx}
                        aria-label={tr("工作列歌詞字體大小")}
                        oninput={(event) => setTaskbarLyricsSettings({ fontSizePx: Number(event.currentTarget.value) })}
                      />
                    </label>
                    <label class="lyrics-preference-toggle">
                      <input type="checkbox" checked={taskbarLyricsSettings.useCustomColor} onchange={(event) => setTaskbarLyricsSettings({ useCustomColor: event.currentTarget.checked })} />
                      <span><strong>{tr("自訂顏色")}</strong><small>{tr("關閉時依工作列的深淺色自動使用黑色或白色")}</small></span>
                      <input
                        class="taskbar-lyrics-color"
                        type="color"
                        value={taskbarLyricsSettings.customColor}
                        disabled={!taskbarLyricsSettings.useCustomColor}
                        aria-label={tr("工作列歌詞顏色")}
                        oninput={(event) => setTaskbarLyricsSettings({ customColor: event.currentTarget.value })}
                      />
                    </label>
                    <div class="cover-corner-setting" role="group" aria-labelledby="{scope}taskbar-lyrics-effect-label">
                      <span id="{scope}taskbar-lyrics-effect-label" class="cover-corner-setting-label"><strong>{tr("歌詞外觀")}</strong></span>
                      <div class="layout-choice-row lyrics-text-effect-row">
                        {#each [['shadow', tr('陰影'), tr('歌詞陰影')], ['stroke', tr('描邊'), tr('歌詞描邊')], ['none', tr('關閉'), tr('關閉歌詞外觀')]] as [value, label, description] (value)}
                          <button
                            type="button"
                            aria-label={description}
                            aria-pressed={taskbarLyricsSettings.textEffect === value}
                            class:chosen={taskbarLyricsSettings.textEffect === value}
                            onclick={() => setTaskbarLyricsSettings({ textEffect: value as TaskbarLyricsSettings['textEffect'] })}
                          >{label}</button>
                        {/each}
                      </div>
                    </div>
                    <label class="lyrics-preference-toggle">
                      <input type="checkbox" checked={taskbarLyricsSettings.showTranslation} onchange={(event) => setTaskbarLyricsSettings({ showTranslation: event.currentTarget.checked })} />
                      <span><strong>{tr("顯示譯文")}</strong><small>{tr("歌詞有翻譯時，在原文下方多顯示一行較小的譯文")}</small></span>
                    </label>
                    <label class="lyrics-preference-range">
                      <span><strong>{tr("譯文大小（相對原文）")}</strong><output>{taskbarLyricsSettings.translationScalePercent}%</output></span>
                      <input
                        type="range"
                        min={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.min}
                        max={TASKBAR_LYRICS_TRANSLATION_SCALE_PERCENT.max}
                        step="1"
                        value={taskbarLyricsSettings.translationScalePercent}
                        disabled={!taskbarLyricsSettings.showTranslation}
                        aria-label={tr("譯文大小（相對原文）")}
                        oninput={(event) => setTaskbarLyricsSettings({ translationScalePercent: Number(event.currentTarget.value) })}
                      />
                    </label>
                    {#if taskbarLyricsError}
                      <p class="settings-preference-status" role="status">{taskbarLyricsError}</p>
                    {/if}
                  </div>
                {/if}
              </div>
            {:else if settingsSection === 'shortcuts'}
              <div id="{scope}shortcuts-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}shortcuts-tab" tabindex="0">
                <div class="settings-panel-header">
                  <div>
                    <h3>{tr("快捷鍵")}</h3>
                    <p>{tr("可綁定鍵盤、滑鼠滾輪、滑鼠側鍵、手柄按鈕與搖桿方向。預設不含手柄。點「新增」後按下要使用的操作，Esc 取消。輸入文字時不會觸發。")}</p>
                  </div>
                </div>
                {#if capturingShortcut}
                  <p class="settings-preference-status" role="status">{tr("正在設定「{0}」。請按下按鍵、滾動滾輪、按滑鼠側鍵、手柄按鈕，或把搖桿推到要綁定的方向。", [SHORTCUT_ACTIONS.find((action) => action.id === capturingShortcut)?.label])}</p>
                {/if}
                <div class="shortcut-list">
                  {#each SHORTCUT_ACTIONS as action (action.id)}
                    <div class="shortcut-row">
                      <strong>{action.label}</strong>
                      <div class="shortcut-bindings">
                        {#each shortcutSettings[action.id] as binding (`${binding.device}:${binding.code}`)}
                          <button class="shortcut-chip" type="button" aria-label={tr("移除{0}的{1}", [action.label, shortcutBindingLabel(binding)])} onclick={() => deleteShortcutBinding(action.id, binding)}>
                            {shortcutBindingLabel(binding)}
                            <span aria-hidden="true">×</span>
                          </button>
                        {:else}
                          <span class="shortcut-empty">{tr("未綁定")}</span>
                        {/each}
                      </div>
                      <button class="soft-button" type="button" onclick={() => beginShortcutCapture(action.id)}>{tr("新增")}</button>
                    </div>
                  {/each}
                </div>
              </div>
            {:else if settingsSection === 'about'}
              <div id="{scope}about-panel" class="settings-panel" role="tabpanel" aria-labelledby="{scope}about-tab" tabindex="0">
                <AboutPage idPrefix={scope} platform={capabilities?.platform} {updateState} checkUpdate={() => updateCenter?.check()} showUpdate={() => updateCenter?.show()} />
              </div>
            {:else}
              <div id="{scope}sources-panel" class="settings-source-panel" role="tabpanel" aria-labelledby="{scope}sources-tab" tabindex="0">
                <section class="settings-view" aria-labelledby="{scope}source-settings-heading">
                  <div class="settings-panel-header settings-source-heading">
                    <div>
                      <h3 id="{scope}source-settings-heading">{tr("管理音樂來源")}</h3>
                    </div>
                  </div>
                  {#if capabilities?.platform === 'android' && allFilesAccess && !allFilesAccess.granted}
                    <div class="all-files-access-card" data-testid="all-files-access">
                      <div>
                        <strong>{tr("所有檔案存取權")}</strong>
                        <p class="source-muted-note">{tr("掃描共享音樂、直接寫入歌曲資訊與讀取系統桌布都需要此權限。")}{allFilesAccess.legacy ? '' : tr('按下後在系統頁面開啟「允許存取所有檔案」，再返回本程式。')}</p>
                      </div>
                      <button class="soft-button" type="button" disabled={allFilesAccessBusy} onclick={() => void requestAllFilesAccess()}>
                        {tr("前往授權")}
                      </button>
                    </div>
                  {/if}
            {#if syncProgress}
              <section
                class="sync-progress-banner"
                class:sync-progress-finished={!syncProgress.active}
                class:sync-progress-error={Boolean(syncProgress.error)}
                data-testid="sync-progress-banner"
                role="status"
                aria-live="polite"
                aria-label={tr("音樂來源同步狀態")}
              >
                <div class="sync-progress-heading">
                  <strong>{syncProgress.active ? tr('背景同步進行中') : tr('最近一次同步摘要')}</strong>
                  {#if syncProgress.active}
                    <span class="sync-progress-count">{syncProgressDoneCount} / {syncProgress.sourceCount} {tr("個來源完成")}</span>
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
                            aria-label={tr("{0} 階段進度", [source.displayName])}
                            max={Math.max(1, source.total)}
                            value={Math.min(source.processed, Math.max(1, source.total))}
                          ></progress>
                        {/if}
                      </li>
                    {/each}
                  </ul>
                {/if}
                {#if !syncProgress.active}
                  <SyncErrorDetails results={sourceSyncResults} sources={sources} />
                {/if}
              </section>
            {/if}
            {#snippet localPane()}
            {#if capabilities?.platform === 'windows' || capabilities?.platform === 'android'}
              {@const androidBlocked = capabilities?.platform === 'android' && allFilesAccess?.granted !== true}
              {#if sourceError}
                <div class="source-error-message" role="status">{sourceError}</div>
              {/if}
              {#if sourceSyncSummary && sourceSyncSummary !== syncProgress?.summary}
                <div class="source-result-message" role="status">{sourceSyncSummary}</div>
              {/if}
              {#if !syncProgress}
                <SyncErrorDetails results={sourceSyncResults} sources={sources} />
              {/if}
              <SourceSections
                folders={localFolderSources}
                playlists={localPlaylistSources}
                excluded={excludedFolders}
                loading={isLoadingSources}
                disabled={isUpdatingSource}
                pickFolderDisabled={!sourceSyncReady || isUpdatingSource || androidBlocked}
                pickPlaylistDisabled={!playlistExchangeReady || !sourceSyncReady || isUpdatingSource || isPlaylistOperation || androidBlocked}
                pickExcludedDisabled={!sourceSyncReady || isUpdatingSource || androidBlocked}
                playlistTooltip={showCapabilityDetail(capabilities?.playlistExchange)}
                onPickFolder={() => void (capabilities?.platform === 'android' ? pickAndroidFolderSource() : pickWindowsFolder())}
                onPickPlaylist={() => void pickWindowsPlaylistSource()}
                onPickExcluded={() => void pickExcludedFolder()}
                onToggle={(row, enabled) => { const source = sources.find((item) => item.id === row.id); if (source) void setSourceEnabled(source, enabled); }}
                onRemove={(row) => { const source = sources.find((item) => item.id === row.id); if (source) void removeSource(source); }}
                onRemoveExcluded={(row) => { const entry = excludedFolders.find((item) => item.id === row.id); if (entry) void removeExcludedFolder(entry); }}
              />
            {:else}
              <div class="source-action-card"><p>{showCapabilityDetail(capabilities?.sourceSync)}</p></div>
            {/if}
            {/snippet}

            {#if capabilities?.platform === 'windows' || capabilities?.platform === 'android'}
              <RemoteSourcesSettings
                disabled={!sourceSyncReady || isUpdatingSource}
                {remoteSources}
                {localPane}
                onSourceAdded={async () => { await loadSources(); await syncLibrary(); }}
                onToggleSource={(source, enabled) => void setSourceEnabled(source, enabled)}
                onRemoveSource={(source) => void removeSource(source)}
              />
            {:else}
              {@render localPane()}
            {/if}
                </section>
              </div>
            {/if}
{/snippet}

<div
  class="app-shell"
  class:has-now-playing-backdrop={isNowPlayingOpen}
  class:has-custom-titlebar={showCustomTitlebar}
  data-active-view={activeView}
  class:has-main-backdrop={effectiveMainBackgroundUrl !== null || androidWallpaperActive}
  style={`--np-background-blur: ${nowPlayingAppearancePreferences.backgroundBlurPx}px; --np-background-bleed: ${nowPlayingAppearancePreferences.backgroundBlurPx * 1.5}px; --np-background-opacity: ${(themePreferences.colorMode === 'autoBackgroundOpacity' ? nowPlayingBackgroundEffectiveOpacityPercent : nowPlayingAppearancePreferences.backgroundOpacityPercent) / 100}; --main-background-blur: ${themePreferences.mainBackgroundBlurPx}px; --main-background-bleed: ${themePreferences.mainBackgroundBlurPx * 1.5}px; --main-background-opacity: ${(themePreferences.colorMode === 'autoBackgroundOpacity' ? mainBackgroundEffectiveOpacityPercent : themePreferences.mainBackgroundOpacityPercent) / 100}; --quick-settings-opacity: ${themePreferences.quickSettingsOpacityPercent}%; --quick-settings-blur: ${themePreferences.quickSettingsBlurPx}px;`}
>
  {#if showCustomTitlebar}
    <WindowTitlebar transparent={isNowPlayingOpen || effectiveMainBackgroundUrl !== null} />
  {/if}
  {#if androidWallpaperActive}
    <div class="main-backdrop is-system-wallpaper" data-testid="main-backdrop" aria-hidden="true" style={`--system-wallpaper-opacity: ${androidWallpaperOpacityPercent / 100}`}></div>
  {:else if effectiveMainBackgroundUrl}
    <div class="main-backdrop" data-testid="main-backdrop" aria-hidden="true">
      <img src={effectiveMainBackgroundUrl} alt="" />
    </div>
  {/if}
  {#if isNowPlayingOpen}
    <NowPlayingBackdrop
      coverUrl={activeArtwork.objectUrl}
      mainBackgroundUrl={effectiveMainBackgroundUrl}
      onCoverError={(url) => artworkController.imageFailed(url)}
    />
  {/if}
  {#if isNavPanelOpen}
    <button class="nav-panel-scrim" type="button" tabindex="-1" aria-label={tr("關閉導覽")} onclick={() => closeNavPanel('outside', false)}></button>
  {/if}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
  <aside
    id="app-sidebar"
    class="sidebar"
    class:floating-surface={isNarrowNav}
    class:nav-panel-open={isNavPanelOpen}
    aria-label={tr("主要導覽")}
    inert={isNowPlayingOpen || (isNarrowNav && !isNavPanelOpen)}
    onclick={handleSidebarClick}
    use:swipeDismiss={{ direction: 'left', enabled: isNarrowNav && isNavPanelOpen, onDismiss: () => closeNavPanel('outside', false) }}
  >
    <div class="brand-lockup">
      <div class="brand-mark">
        <img src={brandIcon} alt="MOE" />
      </div>
      <div class="brand-wordmark">
        <strong>MOE</strong>
        <span>Music Otaku Edge</span>
      </div>
    </div>

    <label class="sidebar-search-field">
      <IconSearch size={16} stroke={1.6} aria-hidden="true" />
      <input
        bind:this={librarySearchInput}
        type="search"
        aria-label={tr("搜尋曲庫")}
        placeholder={tr("搜尋歌名、演出者或專輯")}
        value={query}
        oninput={handleSearchInput}
        disabled={!libraryReady}
      />
    </label>

    <div class="sidebar-scroll">
    <nav class="primary-nav">
      <button
        class="nav-link"
        class:active={activeView === 'library'}
        aria-current={activeView === 'library' ? 'page' : undefined}
        onclick={() => (activeView = 'library')}
      >
        <IconMusic size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">{tr("曲庫")}</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'artists' || activeView === 'artist-detail'}
        aria-current={activeView === 'artists' || activeView === 'artist-detail' ? 'page' : undefined}
        onclick={navigateToArtistSection}
      >
        <IconUsers size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">{tr("演出者")}</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'albums' || activeView === 'album-detail'}
        aria-current={activeView === 'albums' || activeView === 'album-detail' ? 'page' : undefined}
        onclick={navigateToAlbumSection}
      >
        <IconDisc size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">{tr("專輯")}</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
      <button
        class="nav-link"
        class:active={activeView === 'queue'}
        aria-current={activeView === 'queue' ? 'page' : undefined}
        onclick={() => (activeView = 'queue')}
      >
        <IconList size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">{tr("播放佇列")}</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
    </nav>

    <div class="sidebar-playlist-tree-host">
      <PlaylistTree
        {playlists}
        {selectedPlaylistId}
        active={activeView === 'playlists'}
        onOpen={openPlaylistPage}
        onSelect={selectPlaylist}
      />
    </div>
    </div>
    <nav class="sidebar-settings" aria-label={tr("設定")}>
      <button
        class="nav-link"
        class:active={activeView === 'settings'}
        aria-current={activeView === 'settings' ? 'page' : undefined}
        onclick={() => (activeView = 'settings')}
      >
        <IconSettings size={20} stroke={1.6} aria-hidden="true" />
        <span class="nav-label">{tr("設定")}</span>
        <span class="nav-arrow" aria-hidden="true"><IconChevronRight size={15} stroke={1.7} aria-hidden="true" /></span>
      </button>
    </nav>
  </aside>

  <main class="workspace" inert={isNowPlayingOpen}>
    <header class="topbar">
      <button
        bind:this={navPanelToggle}
        class="nav-panel-toggle"
        type="button"
        aria-label={isNavPanelOpen ? tr('關閉導覽') : tr('開啟導覽')}
        aria-expanded={isNavPanelOpen}
        aria-controls="app-sidebar"
        onclick={toggleNavPanel}
      >
        <img src={brandIcon} alt="" />
      </button>
      <div class="topbar-title-block">
        <h1 id="page-heading" class="page-title">
          {#if activeView === 'artist-detail'}
            <button class="artist-page-back" type="button" aria-label={tr("返回演出者首頁")} onclick={navigateToArtistIndex}>{tr("演出者")}</button><span aria-hidden="true">．</span><span>{selectedArtist?.displayName ?? ''}</span>
          {:else if activeView === 'album-detail'}
            <button class="artist-page-back" type="button" aria-label={tr("返回專輯首頁")} onclick={navigateToAlbumIndex}>{tr("專輯")}</button><span aria-hidden="true">．</span><span>{selectedAlbum?.unmarked ? tr('其它') : selectedAlbum?.displayName ?? ''}</span>
          {:else}
            {pageTitle}
          {/if}
        </h1>
        {#if pageTrackCountLabel}
          <p class="page-title-count">{pageTrackCountLabel}</p>
        {/if}
      </div>
      <div class="topbar-actions">
        {#if showSourceScopeSwitch}
          <SourceScopeSwitch value={librarySourceScope} onChange={(next) => void changeLibrarySourceScope(next)} />
        {/if}
        <button
          class="soft-button topbar-refresh-button"
          type="button"
          aria-label={tr("重新整理音樂來源")}
          use:appTooltip={sourceSyncReady ? tr('重新同步本機來源') : showCapabilityDetail(capabilities?.sourceSync)}
          onclick={() => void syncLibrary()}
          disabled={!sourceSyncReady || isSyncing}
        >
          <IconRefresh size={16} stroke={1.7} class={isSyncing ? 'spin' : ''} aria-hidden="true" />
          <span>{isSyncing ? tr('同步中') : tr('重新整理')}</span>
        </button>
      </div>
    </header>

    <div class="page-scroll" class:is-list-page={isListPage}>
      <div class="page-content" class:wide-list-page={isListPage} class:is-list-page={isListPage}>
        {#if activeView === 'library'}
          <section class="library-section" aria-labelledby="page-heading">
            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true">
                  <IconFolder size={24} stroke={1.6} aria-hidden="true" />
                </div>
                <div class="not-ready-copy">
                  <span class="state-label">NATIVE SERVICE</span>
                  <h3>{runtimeError ? tr('尚未連上桌面服務') : tr('曲庫服務尚未就緒')}</h3>
                  <p>{showCapabilityDetail(capabilities?.library)}</p>
                </div>
                <div class="not-ready-meta"><span class="status-dot" aria-hidden="true"></span> {tr("尚未載入曲目")}</div>
              </div>
            {:else}
              <TrackList
                query={query}
                fieldFilter={libraryFieldFilter}
                resetKey={libraryListRevision}
                selectedTrackId={selectedTrackId}
                playbackReady={playbackReady}
                isSendingPlaybackCommand={isSendingPlaybackCommand}
                columns={trackColumnPreferences}
                columnWidths={trackColumnWidths}
                rowHeightPx={trackRowHeightPx}
                groupingEnabled={groupingEnabled}
                sort={librarySort}
                livePlayCount={livePlayCount}
                locateTrackId={playback?.currentTrack?.id ?? null}
                onPlay={playTrack}
                onSort={handleLibrarySort}
                onColumnResize={handleTrackColumnResize}
                onTotalCount={(count) => (libraryTrackCount = count)}
              />
            {/if}
          </section>
        {/if}
        {#if activeView === 'artists' || (activeView === 'artist-detail' && keepArtistIndexForDetail)}
          <section class="artist-section" aria-labelledby="page-heading" hidden={activeView !== 'artists'}>
            {#if !libraryReady}
              <div class="not-ready-panel" role="status">
                <div class="not-ready-icon" aria-hidden="true"><IconUsers size={22} stroke={1.6} /></div>
                <div class="not-ready-copy"><span class="state-label">LIBRARY</span><h3>{tr("演出者服務尚未就緒")}</h3><p>{showCapabilityDetail(capabilities?.library)}</p></div>
              </div>
            {:else}
              <ArtistIndex
                refreshKey={libraryListRevision}
                rowHeightPx={trackRowHeightPx}
                groupingEnabled={groupingEnabled}
                onCountChange={(count) => (artistIndexCount = count)}
                onSelect={(artist) => openArtistEntry(artist, true)}
              />
            {/if}
          </section>
        {/if}
        {#if activeView === 'artist-detail' && selectedArtist}
          <section class="artist-detail-section" aria-labelledby="page-heading">
            {#if !libraryReady}
              <div class="artist-detail-empty" role="status">{showCapabilityDetail(capabilities?.library)}</div>
            {:else}
              {#key selectedArtist.key}
                <TrackList
                  query=""
                  resetKey={`${selectedArtist.key}:${libraryListRevision}`}
                  selectedTrackId={selectedTrackId}
                  playbackReady={playbackReady}
                  isSendingPlaybackCommand={isSendingPlaybackCommand}
                  columns={trackColumnPreferences}
                  columnWidths={trackColumnWidths}
                  rowHeightPx={trackRowHeightPx}
                  groupingEnabled={groupingEnabled}
                  listId={`artist-detail-${selectedArtist.key}`}
                  sort={librarySort}
                  livePlayCount={livePlayCount}
                  fetchPage={fetchArtistPage}
                  quickIndexArtistNames={selectedArtist.artistNames}
                  onPlay={playArtistTrack}
                  onSort={handleLibrarySort}
                  onColumnResize={handleTrackColumnResize}
                  onTotalCount={(count) => (artistTrackCount = count)}
                />
              {/key}
            {/if}
          </section>
        {/if}
        {#if activeView === 'albums' || (activeView === 'album-detail' && keepAlbumIndexForDetail)}
          <section class="album-section" aria-labelledby="page-heading" hidden={activeView !== 'albums'}>
            {#if !libraryReady}
              <div class="not-ready-panel" role="status">
                <div class="not-ready-icon" aria-hidden="true"><IconMusic size={22} stroke={1.6} /></div>
                <div class="not-ready-copy"><span class="state-label">LIBRARY</span><h3>{tr("專輯服務尚未就緒")}</h3><p>{showCapabilityDetail(capabilities?.library)}</p></div>
              </div>
            {:else}
              <AlbumIndex
                refreshKey={libraryListRevision}
                active={activeView === 'albums'}
                rowHeightPx={trackRowHeightPx}
                {albumCardMaxWidthPx}
                {groupingEnabled}
                onCountChange={(count) => (albumIndexCount = count)}
                onSelect={(album, coverSources) => openAlbumEntry(album, true, coverSources)}
              />
            {/if}
          </section>
        {/if}
        {#if activeView === 'album-detail' && selectedAlbum}
          <section class="album-detail-section" aria-labelledby="page-heading">
            {#if !libraryReady}
              <div class="artist-detail-empty" role="status">{showCapabilityDetail(capabilities?.library)}</div>
            {:else}
              {#key selectedAlbum.key}
                <AlbumDetail
                  album={selectedAlbum}
                  initialCoverSources={selectedAlbumInitialCoverSources}
                  selectedTrackId={selectedTrackId}
                  playbackReady={playbackReady}
                  isSendingPlaybackCommand={isSendingPlaybackCommand}
                  columns={trackColumnPreferences}
                  columnWidths={trackColumnWidths}
                  rowHeightPx={trackRowHeightPx}
                  groupingEnabled={groupingEnabled}
                  livePlayCount={livePlayCount}
                  refreshKey={libraryListRevision}
                  onPlay={playAlbumTrack}
                  onColumnResize={handleTrackColumnResize}
                  onTotalCount={(count) => (albumTrackCount = count)}
                />
              {/key}
            {/if}
          </section>
        {/if}
        {#if activeView === 'playlists'}
          <section class="playlist-section" aria-labelledby="page-heading">
            {#if playlistError}
              <div class="inline-message" role="alert">
                <span class="message-mark"><IconAlertCircle size={16} stroke={1.8} aria-hidden="true" /></span>
                <div><strong>{tr("播放清單操作失敗")}</strong><p>{playlistError}</p></div>
                <button class="text-button" type="button" onclick={() => { if (selectedPlaylistId) refreshPlaylistEntries(); else void loadPlaylists(); }}>{tr("再試一次")}</button>
              </div>
            {/if}

            {#if !libraryReady}
              <div class="not-ready-panel">
                <div class="not-ready-icon" aria-hidden="true"><IconMusic size={24} stroke={1.6} aria-hidden="true" /></div>
                <div class="not-ready-copy"><span class="state-label">NATIVE SERVICE</span><h3>{tr("播放清單服務尚未就緒")}</h3><p>{showCapabilityDetail(capabilities?.library)}</p></div>
              </div>
            {:else if isLoadingPlaylists && playlists.length === 0}
              <div class="loading-panel"><span class="loader-ring"></span><span>{tr("正在載入播放清單…")}</span></div>
            {:else if playlists.length === 0}
              <div class="empty-panel">
                <div class="empty-wave" aria-hidden="true"><span></span><span></span><span></span><span></span><span></span></div>
                <h3>{tr("尚無播放清單")}</h3>
                <p>{tr("在「設定 → 音樂來源」選擇播放清單檔案加入來源。")}</p>
              </div>
            {:else if selectedPlaylist}
              <div class="playlist-browser">
                <section class="playlist-detail" aria-label={selectedPlaylist.name.trim() || tr('未命名播放清單')}>
                  <PlaylistEntryList
                    playlistId={selectedPlaylist.id}
                    resetKey={playlistListRevision}
                    {playbackReady}
                    {isSendingPlaybackCommand}
                    columns={trackColumnPreferences}
                    columnWidths={trackColumnWidths}
                    rowHeightPx={trackRowHeightPx}
                    groupingEnabled={groupingEnabled}
                    livePlayCount={livePlayCount}
                    currentTrackId={playback?.currentTrack?.id ?? null}
                    onPlay={playPlaylistEntry}
                    onColumnResize={handleTrackColumnResize}
                    onTotalCount={(count) => (playlistTrackCount = count)}
                  />
                </section>
              </div>
            {:else}
              <div class="empty-panel">
                <h3>{tr("尚未選擇播放清單")}</h3>
                <p>{tr("從左側選擇播放清單。")}</p>
              </div>
            {/if}
          </section>
        {/if}
        {#if activeView === 'queue'}
          <section class="queue-page" aria-labelledby="page-heading">
            {#if !playbackReady}
              <div class="not-ready-panel" role="status">
                <div class="not-ready-icon" aria-hidden="true"><IconList size={22} stroke={1.6} /></div>
                <div class="not-ready-copy"><span class="state-label">PLAYBACK</span><h3>{tr("播放佇列服務尚未就緒")}</h3><p>{showCapabilityDetail(capabilities?.playback)}</p></div>
              </div>
            {:else}
              <PlaybackQueueList
                resetKey={playbackQueueResetKey}
                cursorChangeKey={playbackQueueCursorChangeKey}
                columns={trackColumnPreferences}
                columnWidths={trackColumnWidths}
                rowHeightPx={trackRowHeightPx}
                groupingEnabled={groupingEnabled && playback?.shuffle !== true}
                sort={queueSort}
                hideSortDirection={playback?.shuffle === true}
                onSort={handleQueueSort}
                livePlayCount={livePlayCount}
                onPlay={playQueueEntry}
                onTotalCount={(count) => (queueTrackCount = count)}
                onColumnResize={handleTrackColumnResize}
              />
            {/if}
          </section>
        {/if}
        {#if activeView === 'settings'}
          <section class="settings-page" aria-labelledby="page-heading">
          {@render settingsPanels('')}
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
      aria-label={tr("正在播放")}
      aria-hidden={!isNowPlayingOpen}
      inert={!isNowPlayingOpen}
    >
      <header class="topbar now-playing-overlay-topbar" inert={isQuickSettingsOpen}>
        <button
          bind:this={nowPlayingBackButton}
          class="soft-button icon-button now-playing-overlay-return"
          type="button"
          aria-label={tr("返回播放前頁面")}
          onclick={() => void closeNowPlaying()}
        >
          <IconArrowLeft size={17} stroke={1.8} aria-hidden="true" />
        </button>
        <div class="now-playing-header-track">
          <nav class="now-playing-track-info" aria-label={tr("曲目資訊與曲庫分類")} use:hideWrappedTrailingTrackSeparators>
            <span class="now-playing-track-block now-playing-artist-block" data-track-block="artist">
              {#if nowPlayingArtists.length === 0}
                <button type="button" data-track-field="artist" disabled use:appTooltip={tr('沒有演出者分類資料')} aria-label={tr("沒有演出者分類資料")}>—</button>
              {:else}
                {#each nowPlayingArtists as artist, index (artist)}
                  <span class="now-playing-artist-unit">
                    {#if index > 0}<span class="now-playing-artist-sep" aria-hidden="true">/</span>{/if}
                     <button type="button" data-track-field="artist" use:appTooltip={artist} aria-label={tr("開啟演出者「{0}」明細", [artist])} onclick={() => void openArtistDetailForName(artist)}>{artist}</button>
                  </span>
                {/each}
              {/if}
            </span>
            <span class="now-playing-track-block now-playing-album-block" data-track-block="album">
               <button type="button" data-track-field="album" disabled={!trackFieldValue(playback?.currentTrack, 'album')} use:appTooltip={trackFieldValue(playback?.currentTrack, 'album') ?? tr('沒有專輯分類資料')} aria-label={trackFieldValue(playback?.currentTrack, 'album') ? tr("開啟專輯「{0}」明細", [trackFieldValue(playback?.currentTrack, 'album')]) : tr('沒有專輯分類資料')} onclick={() => { const album = trackFieldValue(playback?.currentTrack, 'album'); if (album) void openAlbumDetailForName(album); }}>{trackFieldLabel(playback?.currentTrack, 'album')}</button>
            </span>
            <span class="now-playing-track-block now-playing-title-block" data-track-block="title">
              <button
                type="button"
                class="now-playing-track-title"
                data-track-field="title"
                aria-label={tr("在曲庫搜尋「{0}」", [trackFieldLabel(playback?.currentTrack, 'title')])}
                disabled={!playback?.currentTrack}
                use:appTooltip={displayTrackTitle(playback?.currentTrack, '') || tr('沒有曲名')}
                onclick={() => void searchNowPlayingTitle()}
              >{trackFieldLabel(playback?.currentTrack, 'title')}</button>
            </span>
          </nav>
          <p class="now-playing-format" aria-label={tr("音質格式、發行日期與播放次數")}>
            <span class="now-playing-audio-quality">
              <span>{formatAudioFormat(playback?.currentTrack ?? {})}</span>
              {#if isHiResTrack(playback?.currentTrack)}<img src={hiResBadgeUrl} alt="Hi-Res" use:appTooltip={'Hi-Res'} />{/if}
            </span>
            {#if nowPlayingReleaseDate !== '—'}
              <span class="now-playing-release-date-block now-playing-release-date" aria-label={tr("發行日期")}>{nowPlayingReleaseDate}</span>
            {/if}
            <span class="now-playing-play-count-block now-playing-play-count" aria-label={tr("播放次數")}>
              {tr("播放次數")} {formatTrackColumnValue('playCount', withLivePlayedMs(playback?.currentTrack ?? {}, livePlayCount), () => '—')}
            </span>
          </p>
        </div>
        <div class="now-playing-topbar-tools">
          {#if lyricsTopbarStatus}
            <div class="lyrics-topbar-status" role="group" aria-label={tr("歌詞來源與同步操作")}>
              <button
                type="button"
                class="soft-button lyrics-topbar-action"
                use:appTooltip={tr("目前來源：{0}（開啟候選以手動指定）", [lyricsTopbarStatus.source])}
                aria-label={tr("手動指定歌詞")}
                disabled={!playback?.currentTrack}
                onclick={() => lyricsTopbarStatus?.openManualSelection()}
              >{tr("歌詞")}</button>
              <button
                type="button"
                class="soft-button lyrics-topbar-action"
                class:is-active={lyricsTopbarStatus.timingPanelOpen}
                use:appTooltip={lyricsTopbarStatus.canAdjustTiming ? tr("目前：{0}（調整延遲 ±10 秒）", [lyricsTopbarStatus.sync]) : tr('目前不是同步歌詞，無法調整延遲')}
                aria-label={tr("同步歌詞延遲調整")}
                aria-pressed={lyricsTopbarStatus.timingPanelOpen}
                disabled={!lyricsTopbarStatus.canAdjustTiming}
                onclick={() => lyricsTopbarStatus?.toggleTimingOffset()}
              >{tr("同步")}</button>
            </div>
          {/if}
          <div class="lyrics-topbar-toggles" role="group" aria-label={tr("歌詞副行顯示")}>
            <button type="button" class="soft-button lyrics-toggle" aria-pressed={lyricsPreferences.showTranslation} aria-label={tr("切換譯文顯示")} onclick={() => updateLyricsPreferences({ showTranslation: !lyricsPreferences.showTranslation }, true)}>{tr("譯")}</button>
            <button type="button" class="soft-button lyrics-toggle" aria-pressed={lyricsPreferences.showRomanization} aria-label={tr("切換羅馬拼音顯示")} onclick={() => updateLyricsPreferences({ showRomanization: !lyricsPreferences.showRomanization }, true)}>{tr("羅")}</button>
          </div>
          <div class="sleep-timer-anchor" bind:this={sleepTimerAnchor}>
            <button
              bind:this={sleepTimerToggle}
              class="soft-button icon-button sleep-timer-toggle"
              class:is-active={sleepTimerActive}
              type="button"
              aria-label={tr("睡眠定時")}
              aria-haspopup="dialog"
              aria-expanded={isSleepTimerOpen}
              aria-pressed={sleepTimerActive}
              use:appTooltip={sleepTimerActive ? tr("睡眠定時（剩餘 {0}）", [sleepTimerLabel]) : tr('睡眠定時')}
              onclick={toggleSleepTimerPopover}
            >
              <IconAlarm size={18} stroke={1.7} aria-hidden="true" />
            </button>
            {#if isSleepTimerOpen}
              <div bind:this={sleepTimerPopover} class="sleep-timer-popover floating-surface" style:--sleep-timer-popover-shift-x={`${sleepTimerPopoverShiftX}px`} role="dialog" aria-label={tr("睡眠定時")}>
                <div class="sleep-timer-value" class:is-active={sleepTimerActive}>
                  <span
                    class="sleep-timer-field"
                    role="slider"
                    tabindex="0"
                    aria-label={tr("睡眠定時小時")}
                    aria-valuemin={0}
                    aria-valuemax={SLEEP_TIMER_MAX_HOURS}
                    aria-valuenow={Number(sleepTimerParts.hours)}
                    aria-valuetext={sleepTimerActive ? tr("剩餘 {0}", [sleepTimerLabel]) : tr('未啟用')}
                    use:sleepTimerWheel={'hours'}
                    onpointerdown={(event) => startSleepTimerDrag(event, 'hours')}
                    onpointermove={moveSleepTimerDrag}
                    onpointerup={endSleepTimerDrag}
                    onpointercancel={endSleepTimerDrag}
                    onkeydown={(event) => handleSleepTimerKeydown(event, 'hours')}
                  >{sleepTimerParts.hours}</span><span class="sleep-timer-colon" aria-hidden="true">:</span><span
                    class="sleep-timer-field"
                    role="slider"
                    tabindex="0"
                    aria-label={tr("睡眠定時分鐘")}
                    aria-valuemin={0}
                    aria-valuemax={SLEEP_TIMER_MAX_MINUTE_FIELD}
                    aria-valuenow={Number(sleepTimerParts.minutes)}
                    aria-valuetext={sleepTimerActive ? tr("剩餘 {0}", [sleepTimerLabel]) : tr('未啟用')}
                    use:sleepTimerWheel={'minutes'}
                    onpointerdown={(event) => startSleepTimerDrag(event, 'minutes')}
                    onpointermove={moveSleepTimerDrag}
                    onpointerup={endSleepTimerDrag}
                    onpointercancel={endSleepTimerDrag}
                    onkeydown={(event) => handleSleepTimerKeydown(event, 'minutes')}
                  >{sleepTimerParts.minutes}</span><span class="sleep-timer-colon" aria-hidden="true">:</span><span class="sleep-timer-seconds" aria-hidden="true">{sleepTimerParts.seconds}</span>
                </div>
                <p class="sleep-timer-hint" style="position:absolute;width:1px;height:1px;overflow:hidden;clip-path:inset(50%);white-space:nowrap;margin:0">{tr("在小時或分鐘上拖曳、滑動或滾動，每格 1；到 00:00 時暫停播放")}</p>
                {#if sleepTimerActive}
                  <button class="text-button sleep-timer-cancel" type="button" onclick={() => void setSleepTimerMinutes(0)}>{tr("取消定時")}</button>
                {/if}
              </div>
            {/if}
          </div>
          <button class="soft-button icon-button now-playing-track-details-trigger" type="button" aria-label={tr("歌曲資訊")} use:appTooltip={tr('歌曲資訊')} aria-haspopup="dialog" aria-expanded={trackDetailsTrackId !== null} disabled={!playback?.currentTrack} onclick={openTrackDetails}>
            <IconInfoCircle size={18} stroke={1.7} aria-hidden="true" />
          </button>
          <button bind:this={quickSettingsTrigger} class="soft-button icon-button now-playing-quick-settings-trigger" type="button" aria-label={tr("開啟快速設定")} use:appTooltip={tr('快速設定')} aria-haspopup="dialog" aria-expanded={isQuickSettingsOpen} onclick={() => void openQuickSettings()}>
            <IconSettings size={18} stroke={1.7} aria-hidden="true" />
          </button>
        </div>
      </header>
      <div class="now-playing-overlay-body" data-testid="now-playing-overlay-body" inert={isQuickSettingsOpen}>
        <div class="now-playing-overlay-content">
          <section class="now-playing-view" aria-label={tr("正在播放")}
            use:swipeDismiss={{ follow: false, damped: true, feedbackTarget: () => coverStageElement ?? null, enabled: isNowPlayingOpen, actions: {}, mouseActions: { left: () => void controlPlayback('playback_next'), right: () => void controlPlayback('playback_previous') } }}
            style:--np-cover-height={coverFrame ? `${coverFrame.height}px` : undefined}
            style:--np-cover-width={coverFrame ? `${coverFrame.width}px` : undefined}
            style:--np-cover-zoom={coverLayoutScale}
            style:--np-cover-reserve={coverLayoutScale === 1 ? 0 : coverLayoutScale}
            style:--np-art-col-a={`${(1 * coverShrinkShare).toFixed(4)}fr`}
            style:--np-art-col-b={`${(1 * coverShrinkShare).toFixed(4)}fr`}
            style:--np-art-row={`${(1.1 * coverShrinkShare).toFixed(4)}fr`}
            style:--np-lyr-col-a={`${(1 + 1 * (1 - coverShrinkShare)).toFixed(4)}fr`}
            style:--np-lyr-col-b={`${(1 + 1 * (1 - coverShrinkShare)).toFixed(4)}fr`}
            style:--np-lyr-row={`${(0.9 + 1.1 * (1 - coverShrinkShare)).toFixed(4)}fr`}>
            <NowPlayingArrangement lyricsSwipe={{ follow: false, damped: true, feedbackTarget: () => coverStageElement ?? null, enabled: isNowPlayingOpen, actions: { left: () => void controlPlayback('playback_next'), right: () => void controlPlayback('playback_previous') } }} layout={nowPlayingLayout} lyricsVisible={!nowPlayingLyricsRemoved} artworkVisible={nowPlayingArtworkUrl !== null}>
              {#snippet artwork()}
                <div
                  bind:this={coverStageElement}
                  class="cover-stage"
                  use:coverZoom={{ enabled: isNowPlayingOpen, onZoom: handleCoverZoom, initial: restoreCoverZoom, syncScale: coverZoomPersisted.scale }}
                  use:swipeDismiss={{ follow: false, damped: true, enabled: isNowPlayingOpen, actions: { left: () => void controlPlayback('playback_next'), right: () => void controlPlayback('playback_previous'), down: () => void closeNowPlaying() }, mouseActions: { down: () => void closeNowPlaying() } }}
                  class:has-artwork={nowPlayingArtworkUrl !== null}
                  class:cover-corner-square={nowPlayingAppearancePreferences.coverCornerStyle === 'square'}
                  style:width={coverFrame ? `${coverFrame.width}px` : undefined}
                  style:height={coverFrame ? `${coverFrame.height}px` : undefined}
                >
                  {#if nowPlayingArtworkUrl}
                    <img class="cover-stage-image" draggable="false" src={nowPlayingArtworkUrl} alt={tr("{0} 封面", [currentTrackTitle(playback?.currentTrack)])} onerror={() => artworkController.imageFailed(nowPlayingArtworkUrl!)} />
                  {:else}
                    <div class="cover-orbit cover-orbit-a"></div>
                    <div class="cover-orbit cover-orbit-b"></div>
                    <div class="cover-wave"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
                    <span class="cover-stage-label">MOE / LOCAL</span>
                    {#if activeArtwork.status === 'too-large'}
                      <span class="cover-fallback-message">{tr("原圖超過 32 MiB 或 64 百萬像素上限")}</span>
                    {:else if activeArtwork.status === 'error'}
                      <span class="cover-fallback-message">{tr("封面格式不支援或無法讀取")}</span>
                    {:else if activeArtwork.status === 'missing' && playback?.currentTrack}
                      <span class="cover-fallback-message">{tr("沒有可用封面")}</span>
                    {/if}
                  {/if}
                </div>
              {/snippet}
              {#snippet lyrics()}
                <LyricsView
                  trackId={playback?.currentTrack?.id ?? null}
                  trackTitle={playback?.currentTrack?.title ?? null}
                  trackArtist={playback?.currentTrack?.artist ?? null}
                  positionMs={playback?.positionMs ?? 0}
                  {playbackClock}
                  isPlaying={playback?.isPlaying ?? false}
                  playbackState={playback?.state ?? 'empty'}
                  {lyricsPreferences}
                  onPreferencesChange={(patch) => updateLyricsPreferences(patch, true)}
                  onLyricsRemovedChange={(removed) => { nowPlayingLyricsRemoved = removed; }}
                  onStatusChange={(status) => { lyricsTopbarStatus = status; }}
                  sharedController={lyricsController}
                  sharedState={lyricsState}
                />
              {/snippet}
            </NowPlayingArrangement>
          </section>
        </div>
      </div>
      {#if trackDetailsTrackId}
        <TrackDetailsDialog trackId={trackDetailsTrackId} onClose={() => { trackDetailsTrackId = null; }} onSaved={() => void reloadAfterTrackDetailsSaved()} onOpenSpectrogram={(id) => void openSpectrogram(id)} />
      {/if}
      {#if androidSpectrogramTrackId}
        <SpectrogramView trackId={androidSpectrogramTrackId} onClose={() => { androidSpectrogramTrackId = null; }} />
      {/if}
      {#if isQuickSettingsOpen}
        <button class="now-playing-quick-settings-scrim" type="button" tabindex="-1" aria-label={tr("關閉快速設定")} onclick={() => void closeQuickSettings()}></button>
        <dialog open
          bind:this={quickSettingsDialog}
          class="now-playing-quick-settings-drawer floating-surface"
          use:swipeDismiss={{ direction: 'right', onDismiss: () => void closeQuickSettings() }}
          aria-modal="true"
          aria-labelledby="now-playing-quick-settings-title"
          data-testid="now-playing-quick-settings"
          style:--quick-settings-opacity={`${themePreferences.quickSettingsOpacityPercent}%`}
          style:--quick-settings-blur={`${themePreferences.quickSettingsBlurPx}px`}
        >
          <header class="quick-settings-drawer-header">
            <h2 id="now-playing-quick-settings-title">{tr("快速設定")}</h2>
            <button bind:this={quickSettingsCloseButton} class="soft-button icon-button" type="button" aria-label={tr("關閉快速設定")} onclick={() => void closeQuickSettings()}><IconX size={18} stroke={1.8} aria-hidden="true" /></button>
          </header>
          <div class="quick-settings-drawer-nav">
            {@render settingsTabs('quick-')}
          </div>
          <div class="quick-settings-drawer-scroll" bind:this={quickSettingsScroll}>
            {@render settingsPanels('quick-', { includeTabs: false })}
          </div>
        </dialog>
      {/if}
    </section>
  {/if}

  {#if isQuickSettingsOpen}
    <button
      type="button"
      class="now-playing-quick-settings-dock-dismiss"
      tabindex="-1"
      aria-label={tr("關閉快速設定")}
      onclick={() => void closeQuickSettings()}
    ></button>
  {/if}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <footer class="player-dock" data-timeline-style={nowPlayingAppearancePreferences.timelineStyle} aria-label={tr("播放控制")} inert={isQuickSettingsOpen} onclick={toggleNowPlayingFromDock}>
    <div class="dock-track" class:dock-track-dismiss={isNowPlayingOpen}>
      {#if isNowPlayingOpen}
        <button
          class="dock-now-playing-dismiss"
          type="button"
          aria-label={tr("返回播放前頁面")}
           use:appTooltip={tr('返回播放前頁面')}
          onclick={() => void closeNowPlaying()}
        ></button>
      {:else}
        <button
          class="dock-art"
          type="button"
          bind:this={dockArtworkButton}
          aria-label={playback?.currentTrack ? tr("開啟正在播放：{0}", [currentTrackTitle(playback.currentTrack)]) : tr('尚未選擇歌曲')}
          disabled={!playback?.currentTrack}
        >
          <!-- While the next track's cover loads the controller still holds the previous image: keep it until the new one replaces it (no flash of the placeholder). -->
          {#if (activeArtwork.status === 'ready' || activeArtwork.status === 'loading') && activeArtwork.objectUrl}
            <img class="dock-art-image" src={activeArtwork.objectUrl} alt="" onerror={() => artworkController.imageFailed(activeArtwork.objectUrl!)} />
          {:else}
            <IconMusic size={22} stroke={1.6} aria-hidden="true" />
          {/if}
        </button>
        <div class="dock-track-copy">
          <strong class="dock-marquee"><span class="dock-marquee-text" use:dockMarquee>{currentTrackTitle(playback?.currentTrack)}</span></strong>
          <span class="dock-marquee"><span class="dock-marquee-text" use:dockMarquee>{currentTrackArtist(playback?.currentTrack)}</span></span>
        </div>
      {/if}
    </div>

    <div class="dock-center">
      <div class="dock-controls">
        <FavoritePopover
          trackId={playback?.currentTrack?.id ?? null}
          platform={capabilities?.platform ?? 'unsupported'}
          onChange={async () => { await loadPlaylists(); await loadSources(); }}
        />
        <button class="control-button secondary-control" type="button" aria-label={tr("隨機播放")} use:appTooltip={playbackControlTooltips.shuffle} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.shuffle} aria-pressed={playback?.shuffle === true} onclick={toggleShuffle}>
          {#if playback?.shuffle}
            <IconArrowsShuffle size={23} stroke={1.7} aria-hidden="true" />
          {:else}
            <IconArrowsRight size={23} stroke={1.7} aria-hidden="true" />
          {/if}
        </button>
        <button class="control-button navigation-control" type="button" aria-label={tr("上一首")} use:appTooltip={playbackControlTooltips.previous} disabled={!playbackNavigationReady || (!playback?.canPrevious && (playback?.positionMs ?? 0) <= 3000) || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_previous')}>
          <IconPlayerTrackPrev size={23} stroke={1.7} aria-hidden="true" />
        </button>
        <button class="play-button" type="button" aria-label={playback?.isPlaying ? tr('暫停') : tr('播放')} use:appTooltip={playbackControlTooltips.playPause} disabled={!playbackReady || isSendingPlaybackCommand} onclick={togglePlayback}>
          {#if playback?.isPlaying}
            <IconPlayerPause size={26} stroke={1.7} aria-hidden="true" />
          {:else}
            <IconPlayerPlay size={26} stroke={1.7} aria-hidden="true" />
          {/if}
        </button>
        <button class="control-button navigation-control" type="button" aria-label={tr("下一首")} use:appTooltip={playbackControlTooltips.next} disabled={!playbackNavigationReady || !playback?.canNext || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_next')}>
          <IconPlayerTrackNext size={23} stroke={1.7} aria-hidden="true" />
        </button>
        <button class="control-button secondary-control" type="button" aria-label={tr("循環播放")} use:appTooltip={playbackControlTooltips.repeat} disabled={!playbackModesReady || isSendingPlaybackCommand} class:control-active={playback?.repeatMode !== 'off' && playback?.repeatMode !== undefined} onclick={setRepeatMode}>
          {#if playback?.repeatMode === 'one'}
            <IconRepeatOnce size={23} stroke={1.7} aria-hidden="true" />
          {:else if playback?.repeatMode === 'all'}
            <IconRepeat size={23} stroke={1.7} aria-hidden="true" />
          {:else}
            <IconRepeatOff size={23} stroke={1.7} aria-hidden="true" />
          {/if}
        </button>
        <div
          class="dock-volume"
          class:expanded={volumeExpanded}
          bind:this={volumePanelElement}
        >
          <button
            class="volume-toggle"
            type="button"
            aria-label={tr("音量")}
            aria-expanded={volumeExpanded}
            aria-haspopup="dialog"
            use:appTooltip={volumeExpanded ? tr('收合音量') : tr('展開音量')}
            disabled={!playbackReady}
            onclick={toggleVolumeExpanded}
          >
            {#if volumeIconLevel === 'off'}
              <IconVolumeOff size={23} stroke={1.6} aria-hidden="true" />
            {:else if volumeIconLevel === 'low'}
              <IconVolume4 size={23} stroke={1.6} aria-hidden="true" />
            {:else if volumeIconLevel === 'medium'}
              <IconVolume2 size={23} stroke={1.6} aria-hidden="true" />
            {:else}
              <IconVolume size={23} stroke={1.6} aria-hidden="true" />
            {/if}
          </button>
          <div
            class="volume-popover floating-surface"
            role="dialog"
            aria-label={tr("音量控制")}
            hidden={!volumeExpanded}
            bind:this={volumePopoverElement}
            style:--volume-popover-shift-left={`${volumePopoverLeftShiftPx}px`}
          >
            <input
              class="volume-slider"
              type="range"
              min="0"
              max="1"
              step="0.01"
              value={volumeDraft ?? playback?.volume ?? confirmedVolume ?? 0}
              aria-label={tr("音量滑桿")}
              tabindex={volumeExpanded ? 0 : -1}
              disabled={!playbackReady || !volumeExpanded}
              onpointerdown={startVolumeInteraction}
              oninput={setPlaybackVolume}
              onchange={finishVolumeChange}
            />
            <span class="volume-value">{playback ? formatVolume(volumeDraft ?? playback.volume) : confirmedVolume === null ? '—' : formatVolume(volumeDraft ?? confirmedVolume)}</span>
          </div>
        </div>
      </div>
      <PlaybackProgress
        positionMs={playback?.positionMs ?? 0}
        durationMs={playbackDurationMs}
        trackId={playback?.currentTrack?.id ?? null}
        canControl={playbackReady}
        isSending={isSendingPlaybackCommand}
        timelineStyle={nowPlayingAppearancePreferences.timelineStyle}
        clock={playbackClock}
        onSeek={commitPlaybackSeek}
      />
      {#if playbackError || playback?.lastError}<span class="dock-error" role="status">{playbackError ?? playback?.lastError}</span>{/if}
    </div>

    <div class="dock-spacer" aria-hidden="true"></div>
  </footer>
</div>

{#if runtimePlatform === 'android'}
  <FolderBrowserDialog
    open={folderPickerRequestId !== null}
    style={`--quick-settings-opacity:${themePreferences.quickSettingsOpacityPercent}%;--quick-settings-blur:${themePreferences.quickSettingsBlurPx}px`}
    onFinish={finishFolderPicker}
  />
  <dialog
    bind:this={allFilesPromptDialog}
    class="update-dialog floating-surface"
    style={`--quick-settings-opacity:${themePreferences.quickSettingsOpacityPercent}%;--quick-settings-blur:${themePreferences.quickSettingsBlurPx}px`}
    aria-labelledby="all-files-prompt-title"
    aria-describedby="all-files-prompt-description"
    oncancel={(event) => { event.preventDefault(); closeAllFilesPrompt(); }}
  >
    <div class="update-dialog-content">
      <h2 id="all-files-prompt-title">{tr("需要「所有檔案存取權」")}</h2>
      <p id="all-files-prompt-description">{tr("MoeMusicPlayer 需要這個權限才能：掃描你的音樂資料夾與播放清單、把修改的歌曲資訊直接寫回音樂檔案，以及讀取系統桌布作為背景。沒有授權時，曲庫和這些功能無法使用。")}</p>
      <p>{tr("按下「前往授權」後，系統會開啟設定頁；請開啟「允許存取所有檔案」，再返回本程式。")}</p>
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeAllFilesPrompt}>{tr("取消")}</button>
        <button class="soft-button" type="button" onclick={confirmAllFilesPrompt}>{tr("前往授權")}</button>
      </div>
    </div>
  </dialog>
  <dialog
    bind:this={wallpaperPermissionPromptDialog}
    class="update-dialog floating-surface"
    style={`--quick-settings-opacity:${themePreferences.quickSettingsOpacityPercent}%;--quick-settings-blur:${themePreferences.quickSettingsBlurPx}px`}
    aria-labelledby="wallpaper-permission-title"
    aria-describedby="wallpaper-permission-description"
    oncancel={(event) => { event.preventDefault(); closeWallpaperPermissionPrompt(); }}
  >
    <div class="update-dialog-content">
      <h2 id="wallpaper-permission-title">{tr("需要「讀取圖片」權限")}</h2>
      <p id="wallpaper-permission-description">{tr("這個系統要求再多授權「讀取圖片」，才允許 MoeMusicPlayer 讀取系統桌布。沒有授權時，背景會顯示純色。")}</p>
      <p>{tr("按下「前往授權」後，系統會詢問是否允許；若系統不再詢問，會開啟本程式的設定頁，請在「權限」中允許「照片和影片」或「圖片」，再返回本程式。")}</p>
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeWallpaperPermissionPrompt}>{tr("取消")}</button>
        <button class="soft-button" type="button" onclick={() => void confirmWallpaperPermissionPrompt()}>{tr("前往授權")}</button>
      </div>
    </div>
  </dialog>
{/if}

<UpdateCenter
  bind:this={updateCenter}
  enabled={capabilities?.platform === 'windows' || capabilities?.platform === 'android'}
  platform={capabilities?.platform ?? 'unsupported'}
  bind:updateState
  surfaceOpacityPercent={themePreferences.quickSettingsOpacityPercent}
  surfaceBlurPx={themePreferences.quickSettingsBlurPx}
/>
