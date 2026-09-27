<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import {
    getErrorText,
    invokeCommand,
    isReady,
    type LibrarySource,
    type MediaStoreVolumeOption,
    type FeatureCapability,
    type PlaybackSnapshot,
    type RuntimeCapabilities,
    type TrackPage,
    type TrackSummary,
  } from './lib/ipc';
  import { formatDuration, formatTrackIndex, formatVolume } from './lib/format';

  type View = 'library' | 'now-playing' | 'settings';

  const PAGE_SIZE = 40;

  let activeView = $state<View>('library');
  let capabilities = $state<RuntimeCapabilities | null>(null);
  let runtimeError = $state<string | null>(null);
  let page = $state<TrackPage | null>(null);
  let pageError = $state<string | null>(null);
  let playbackError = $state<string | null>(null);
  let playback = $state<PlaybackSnapshot | null>(null);
  let sources = $state<LibrarySource[]>([]);
  let sourceError = $state<string | null>(null);
  let sourceSyncSummary = $state<string | null>(null);
  let mediaStoreVolumes = $state<MediaStoreVolumeOption[]>([]);
  let mediaPermissionGranted = $state<boolean | null>(null);
  let windowsFolderPath = $state('');
  let query = $state('');
  let offset = $state(0);
  let selectedTrackId = $state<string | null>(null);
  let isLoadingPage = $state(false);
  let isSyncing = $state(false);
  let isLoadingSources = $state(false);
  let isUpdatingSource = $state(false);
  let isLoadingVolumes = $state(false);
  let isSendingPlaybackCommand = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let pageRequestVersion = 0;
  let unlistenSync: UnlistenFn | undefined;

  const libraryReady = $derived(isReady(capabilities?.library));
  const sourceSyncReady = $derived(isReady(capabilities?.sourceSync));
  const playbackReady = $derived(isReady(capabilities?.playback));
  const hasPreviousPage = $derived((page?.offset ?? 0) > 0);
  const hasNextPage = $derived(
    page !== null && page.offset + page.items.length < page.totalCount,
  );
  const pageRangeLabel = $derived.by(() => {
    if (!page || page.totalCount === 0) return '0 首';
    const first = page.offset + 1;
    const last = Math.min(page.offset + page.items.length, page.totalCount);
    return `${first}–${last} 首，共 ${page.totalCount.toLocaleString()} 首`;
  });

  onMount(() => {
    void loadCapabilities();
    let disposed = false;
    if (isTauri()) {
      void listen('library-sync-finished', () => {
        if (isSyncing) return;
        if (libraryReady) void loadPage(0);
        if (sourceSyncReady) void loadSources();
      })
        .then((unlisten) => {
          if (disposed) unlisten();
          else unlistenSync = unlisten;
        })
        .catch(() => undefined);
    }
    return () => {
      disposed = true;
      unlistenSync?.();
    };
  });

  onDestroy(() => {
    if (searchTimer !== undefined) clearTimeout(searchTimer);
    unlistenSync?.();
    pageRequestVersion += 1;
  });

  async function loadCapabilities(): Promise<void> {
    runtimeError = null;
    try {
      capabilities = await invokeCommand('get_runtime_capabilities', {});
      if (isReady(capabilities.library)) void loadPage(0);
      if (isReady(capabilities.sourceSync)) void loadSources();
      if (isReady(capabilities.playback)) void loadPlaybackSnapshot();
    } catch (error) {
      capabilities = null;
      runtimeError = getErrorText(error);
    }
  }

  async function loadPage(nextOffset: number): Promise<void> {
    if (!libraryReady) return;

    const requestVersion = ++pageRequestVersion;
    isLoadingPage = true;
    pageError = null;
    try {
      const nextPage = await invokeCommand('library_get_page', {
        query: query.trim() || null,
        offset: nextOffset,
        limit: PAGE_SIZE,
      });
      if (requestVersion !== pageRequestVersion) return;
      page = nextPage;
      offset = nextPage.offset;
    } catch (error) {
      if (requestVersion !== pageRequestVersion) return;
      page = null;
      pageError = getErrorText(error);
    } finally {
      if (requestVersion === pageRequestVersion) isLoadingPage = false;
    }
  }

  function handleSearchInput(event: Event): void {
    query = (event.currentTarget as HTMLInputElement).value;
    if (searchTimer !== undefined) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void loadPage(0), 240);
  }

  async function syncLibrary(): Promise<void> {
    if (!sourceSyncReady) return;
    isSyncing = true;
    pageError = null;
    sourceError = null;
    try {
      const result = await invokeCommand('library_sync', {});
      const completed = result.sources.filter((source) => source.state === 'complete').length;
      const partial = result.sources.length - completed;
      sourceSyncSummary = result.sources.length === 0
        ? '尚未加入可同步的音樂來源。'
        : `${result.sources.length} 個來源已檢查；${completed} 個完整，${partial} 個需要留意。`;
      await Promise.all([loadPage(0), loadSources()]);
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

  async function addWindowsFolder(): Promise<void> {
    const path = windowsFolderPath.trim();
    if (!path || isUpdatingSource) return;
    isUpdatingSource = true;
    sourceError = null;
    try {
      await invokeCommand('library_add_windows_folder', { path });
      windowsFolderPath = '';
      await loadSources();
      await syncLibrary();
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

  async function loadPlaybackSnapshot(): Promise<void> {
    if (!playbackReady) return;
    playbackError = null;
    try {
      playback = await invokeCommand('playback_get_snapshot', {});
    } catch (error) {
      playbackError = getErrorText(error);
    }
  }

  async function playTrack(track: TrackSummary): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    selectedTrackId = track.id;
    await sendPlaybackCommand(() => invokeCommand('playback_play', { trackId: track.id }));
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

  async function seekPlayback(event: Event): Promise<void> {
    if (!playbackReady || isSendingPlaybackCommand) return;
    const positionMs = Number((event.currentTarget as HTMLInputElement).value);
    await sendPlaybackCommand(() => invokeCommand('playback_seek', { positionMs }));
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
      playback = await request();
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
    return track?.artist?.trim() || '播放引擎接通後即可播放本機音樂';
  }

  function nextPage(): void {
    if (hasNextPage && page) void loadPage(page.offset + PAGE_SIZE);
  }

  function previousPage(): void {
    if (hasPreviousPage && page) void loadPage(Math.max(0, page.offset - PAGE_SIZE));
  }
</script>

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
        class:active={activeView === 'settings'}
        aria-current={activeView === 'settings' ? 'page' : undefined}
        onclick={() => (activeView = 'settings')}
      >
        <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M12 8.7a3.3 3.3 0 1 0 0 6.6 3.3 3.3 0 0 0 0-6.6Z" stroke="currentColor" stroke-width="1.55" /><path d="m19.4 13.6 1.2.9-1.5 2.6-1.4-.7a7.5 7.5 0 0 1-1.5.9l-.2 1.6h-3l-.2-1.6a7.5 7.5 0 0 1-1.5-.9l-1.4.7-1.5-2.6 1.2-.9a7 7 0 0 1 0-1.8l-1.2-.9 1.5-2.6 1.4.7a7.5 7.5 0 0 1 1.5-.9l.2-1.6h3l.2 1.6a7.5 7.5 0 0 1 1.5.9l1.4-.7 1.5 2.6-1.2.9a7 7 0 0 1 0 1.8Z" stroke="currentColor" stroke-width="1.35" stroke-linejoin="round" /></svg>
        <span class="nav-label">來源設定</span>
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
      <div class="breadcrumbs"><span>MOEMUSIC</span><span class="breadcrumb-slash">/</span><strong>{activeView === 'library' ? 'LIBRARY' : activeView === 'now-playing' ? 'NOW PLAYING' : 'SOURCES'}</strong></div>
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
            <h1>{activeView === 'library' ? '把喜歡的聲音，留在身邊。' : activeView === 'now-playing' ? '正在播放' : '音樂來源'}</h1>
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

        {#if activeView === 'library'}
          <section class="library-section" aria-labelledby="library-heading">
            <div class="section-heading">
              <div>
                <p class="section-kicker">COLLECTION</p>
                <h2 id="library-heading">我的曲庫</h2>
              </div>
              <div class="section-heading-actions">
                <span class="page-count">{page?.totalCount.toLocaleString() ?? '—'} <small>首曲目</small></span>
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
            {:else if pageError}
              <div class="inline-message" role="status">
                <span class="message-mark">!</span>
                <div><strong>無法載入這一頁</strong><p>{pageError}</p></div>
                <button class="text-button" type="button" onclick={() => void loadPage(offset)}>再試一次</button>
              </div>
            {:else if isLoadingPage && !page}
              <div class="loading-panel"><span class="loader-ring"></span><span>正在載入曲庫頁面…</span></div>
            {:else if page && page.items.length === 0}
              <div class="empty-panel">
                <div class="empty-wave" aria-hidden="true"><span></span><span></span><span></span><span></span><span></span></div>
                <h3>{query.trim() ? '找不到相符曲目' : '曲庫目前是空的'}</h3>
                <p>{query.trim() ? '試著縮短關鍵字，或搜尋歌名、演出者與專輯。' : '加入本機音樂來源並完成首次同步後，曲目會顯示在這裡。'}</p>
              </div>
            {:else if page}
      <div class="track-table" role="table" aria-label="曲庫曲目">
                <div class="track-table-head">
                  <span class="column-index">#</span>
                  <span>曲目</span>
                  <span class="column-album">專輯</span>
                  <span class="column-duration">長度</span>
                  <span class="column-action"></span>
                </div>
                <div class="track-table-body" aria-live="polite">
                  {#each page.items as track (track.id)}
                    <div class="track-row" class:selected={selectedTrackId === track.id}>
                      <span class="track-index">{formatTrackIndex(track.trackNumber, track.discNumber)}</span>
                      <div class="track-main">
                        <span class="track-title">{track.title?.trim() || '未命名曲目'}</span>
                        <span class="track-artist">{track.artist?.trim() || '未知演出者'}</span>
                      </div>
                      <span class="track-album column-album">{track.album?.trim() || '未知專輯'}</span>
                      <span class="track-duration column-duration">{formatDuration(track.durationMs)}</span>
                      <button
                        class="row-play column-action"
                        type="button"
                        aria-label={`播放 ${track.title?.trim() || '未命名曲目'}`}
                        title={playbackReady ? '播放曲目' : showCapabilityDetail(capabilities?.playback)}
                        disabled={!playbackReady || isSendingPlaybackCommand}
                        onclick={() => void playTrack(track)}
                      >
                        <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.3 5.8 7 4.2-7 4.2V5.8Z" fill="currentColor" /></svg>
                      </button>
                    </div>
                  {/each}
                </div>
              </div>
              <div class="pagination-bar">
                <span>{pageRangeLabel}</span>
                <div class="pagination-actions">
                  <button type="button" class="page-button" onclick={previousPage} disabled={!hasPreviousPage || isLoadingPage} aria-label="上一頁">
                    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m12.5 4.5-5 5.5 5 5.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" /></svg>
                  </button>
                  <button type="button" class="page-button" onclick={nextPage} disabled={!hasNextPage || isLoadingPage} aria-label="下一頁">
                    <svg viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7.5 4.5 5 5.5-5 5.5" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" /></svg>
                  </button>
                </div>
              </div>
            {/if}
          </section>
        {:else if activeView === 'now-playing'}
          <section class="now-playing-view" aria-labelledby="now-playing-heading">
            <div class="now-playing-card">
              <div class="cover-stage" aria-hidden="true">
                <div class="cover-orbit cover-orbit-a"></div>
                <div class="cover-orbit cover-orbit-b"></div>
                <div class="cover-wave"><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i><i></i></div>
                <span class="cover-stage-label">MOE / LOCAL</span>
              </div>
              <div class="now-playing-copy">
                <p class="section-kicker">NOW PLAYING</p>
                <h2 id="now-playing-heading">{currentTrackTitle(playback?.currentTrack)}</h2>
                <p class="now-playing-artist">{currentTrackArtist(playback?.currentTrack)}</p>
                <div class="play-state-chip" class:ready={playbackReady}>
                  <span class="status-dot" aria-hidden="true"></span>
                  {playbackReady ? playback?.isPlaying ? '播放中' : '已暫停' : '播放引擎尚未就緒'}
                </div>
                {#if playbackError}<p class="error-note" role="status">{playbackError}</p>{/if}
                <button class="outline-button now-playing-return" type="button" onclick={() => (activeView = 'library')}>返回曲庫</button>
              </div>
            </div>
            <div class="playback-note">
              <span class="note-icon" aria-hidden="true">i</span>
              <p>{playbackReady ? '播放狀態由原生音訊服務提供。' : showCapabilityDetail(capabilities?.playback)}</p>
            </div>
          </section>
        {:else}
          <section class="settings-view" aria-labelledby="settings-heading">
            <div class="section-heading settings-heading">
              <div><p class="section-kicker">SOURCES</p><h2 id="settings-heading">管理音樂來源</h2></div>
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
              <form class="source-action-card source-folder-form" onsubmit={(event) => { event.preventDefault(); void addWindowsFolder(); }}>
                <label for="windows-folder-path">Windows 音樂資料夾</label>
                <div class="source-action-row">
                  <input id="windows-folder-path" bind:value={windowsFolderPath} type="text" placeholder="例如 D:\\Music" autocomplete="off" spellcheck="false" disabled={!sourceSyncReady || isUpdatingSource} />
                  <button class="primary-button" type="submit" disabled={!sourceSyncReady || !windowsFolderPath.trim() || isUpdatingSource}>
                    {isUpdatingSource ? '處理中' : '加入並掃描'}
                  </button>
                </div>
                <p>輸入現有資料夾的完整路徑。路徑保存在本機 SQLite，不會傳到前端以外的服務。</p>
              </form>
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
        {/if}
      </div>
    </div>
  </main>

  <footer class="player-dock" aria-label="播放控制">
    <div class="dock-track">
      <div class="dock-art" aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none"><path d="M9 17V5l9-2v12M9 17a2.8 2.8 0 1 1-2.8-2.8A2.8 2.8 0 0 1 9 17Zm9-2a2.8 2.8 0 1 1-2.8-2.8A2.8 2.8 0 0 1 18 15Z" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" stroke-linejoin="round" /></svg>
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
        <button class="control-button secondary-control" type="button" aria-label="隨機播放" title={playbackReady ? '切換隨機播放' : showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} class:control-active={playback?.shuffle} onclick={toggleShuffle}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M16 4h3v3M19 4l-6.5 7.2M5 6h2.2c1 0 1.9.5 2.5 1.2l5.6 7.6c.5.7 1.4 1.2 2.4 1.2H19m-3-3 3 3-3 3M5 16h2.2c.8 0 1.6-.4 2.1-1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="control-button" type="button" aria-label="上一首" title={showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_previous')}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M6 5v12m11-11-8 5 8 5V6Z" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="play-button" type="button" aria-label={playback?.isPlaying ? '暫停' : '播放'} title={showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} onclick={togglePlayback}>
          {#if playback?.isPlaying}
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M8 6v12M16 6v12" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" /></svg>
          {:else}
            <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="m8.5 5.8 10 6.2-10 6.2V5.8Z" fill="currentColor" /></svg>
          {/if}
        </button>
        <button class="control-button" type="button" aria-label="下一首" title={showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} onclick={() => void controlPlayback('playback_next')}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M16 5v12M5 6l8 5-8 5V6Z" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
        <button class="control-button secondary-control" type="button" aria-label="循環播放" title={playbackReady ? '切換循環模式' : showCapabilityDetail(capabilities?.playback)} disabled={!playbackReady || isSendingPlaybackCommand} class:control-active={playback?.repeatMode !== 'off' && playback?.repeatMode !== undefined} onclick={setRepeatMode}>
          <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M17 8h2.5l-3-3-3 3H16v6a3 3 0 0 1-3 3h-1M5 14H2.5l3 3 3-3H6V8a3 3 0 0 1 3-3h1" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /><circle cx="16" cy="15" r="3" fill="var(--dock-bg)" /><path d="M16 13.4v1.7l1.1.7" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" stroke-linejoin="round" /></svg>
        </button>
      </div>
      <div class="progress-row">
        <span>{formatDuration(playback?.positionMs)}</span>
        <input
          class="progress-slider"
          type="range"
          min="0"
          max={Math.max(1, playback?.durationMs ?? 0)}
          value={Math.min(playback?.positionMs ?? 0, playback?.durationMs ?? 0)}
          aria-label="播放進度"
          disabled={!playbackReady || !playback?.durationMs || isSendingPlaybackCommand}
          oninput={seekPlayback}
        />
        <span>{formatDuration(playback?.durationMs)}</span>
      </div>
      {#if playbackError}<span class="dock-error" role="status">{playbackError}</span>{/if}
    </div>

    <div class="dock-volume">
      <span class="volume-state">{playbackReady ? '音量' : '播放未就緒'}</span>
      <svg viewBox="0 0 22 22" fill="none" aria-hidden="true"><path d="M4 9v4h3.3l4.2 3.4V5.6L7.3 9H4Z" stroke="currentColor" stroke-width="1.45" stroke-linejoin="round" /><path d="M15 8a4.2 4.2 0 0 1 0 6m2.2-8a7.2 7.2 0 0 1 0 10" stroke="currentColor" stroke-width="1.45" stroke-linecap="round" /></svg>
      <input class="volume-slider" type="range" min="0" max="1" step="0.01" value={playback?.volume ?? 0} aria-label="音量" disabled={!playbackReady || isSendingPlaybackCommand} oninput={setPlaybackVolume} />
      <span class="volume-value">{playback ? formatVolume(playback.volume) : '—'}</span>
    </div>
  </footer>
</div>
