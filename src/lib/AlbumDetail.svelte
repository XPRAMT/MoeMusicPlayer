<script lang="ts">
  import { tr } from './i18n.js';
  import { onDestroy, onMount, untrack } from 'svelte';
  import { getAlbumThumbnailBytes, getAlbumThumbnailBytesForTrack, getErrorText, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytesForTrack, type QuickIndexSnapshot, type TrackPage, type TrackPageRequest, type TrackSummary } from './ipc';
  import type { AlbumIndexEntry as AlbumEntry, AlbumTrackOrder } from './ipc';
  import { advanceAlbumArtworkSelectionRevision, albumCoverSourcesHaveSameContent, copyToArrayBuffer, createAlbumThumbnail, resolveAlbumArtworkDisplaySources, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import { createAlbumDetailCoverScan } from './album-detail-cover-scan.js';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';
  import TrackList from './TrackList.svelte';
  import { formatDurationHours, formatAlbumTrackNumber } from './format';
  import { DEFAULT_ALBUM_DETAIL_SORT, normalizeAlbumDetailSort, toggleAlbumDetailSort } from './album-detail-sort.js';
  import { albumReflowProbeEnabled, startAlbumReflowProbe } from './album-reflow-debug.js';
  import { DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX, MIN_ALBUM_DETAIL_COVER_SIZE_PX, NARROW_ALBUM_DETAIL_COVER_SIZE_PX, getAlbumDetailCollapseRange } from './album-detail-collapse.js';

  interface Props {
    album: AlbumEntry;
    initialCoverSources?: string[];
    selectedTrackId: string | null;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    columns: import('./ipc').TrackListColumnPreference[];
    columnWidths: import('./ipc').TrackColumnWidths;
    rowHeightPx: number;
    groupingEnabled: boolean;
    livePlayCount: { trackId: string; playedMs: number }[];
    refreshKey: number;
    /** `order` is the page's current ordering (the `library_get_album_page` arguments), so the queue matches it. */
    onPlay: (track: TrackSummary, order: AlbumTrackOrder) => void | Promise<void>;
    onColumnResize: (columnId: import('./ipc').TrackListColumnPreference['id'], widthPx: number) => void;
    onTotalCount: (count: number | null) => void;
  }
  let { album, initialCoverSources = [], selectedTrackId, playbackReady, isSendingPlaybackCommand, columns, columnWidths, rowHeightPx, groupingEnabled, livePlayCount, refreshKey, onPlay, onColumnResize, onTotalCount }: Props = $props();
  let coverUrls = $state<string[]>([]);
  let coverScanResolved = $state(false);
  let ownedCoverUrls: string[] = [];
  let artists = $state<string[]>([]);
  let scanning = $state(true);
  let scanError = $state<string | null>(null);
  let detailSort = $state<import('./ipc').AlbumDetailSort>({ ...DEFAULT_ALBUM_DETAIL_SORT });
  let sortReady = $state(false);
  let sortSaveQueue: Promise<unknown> = Promise.resolve();
  let albumCoverBaseSize = $state(DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX);
  let albumHeadingBaseHeight = $state(DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX + 36);
  const supportsScrollDrivenAlbumOverlay = typeof CSS !== 'undefined'
    && CSS.supports('animation-timeline', 'scroll()')
    && CSS.supports('animation-range', '0px 1px')
    && CSS.supports('timeline-scope', '--album-detail-scroll')
    && CSS.supports('scroll-timeline-name', '--album-detail-scroll');
  /**
   * Current collapse offset in CSS px. Only the fallback for runtimes without CSS scroll timelines uses it (and only then
   * does the list report scroll positions); with scroll timelines the compositor drives the cover from the native scroll
   * offset and nothing here is written per scroll event.
   */
  let albumCoverScrollPeak = $state(0);
  let albumCollapseRange = $derived(getAlbumDetailCollapseRange(albumCoverBaseSize));
  let albumCollapseProgress = $derived(albumCollapseRange > 0 ? Math.min(1, Math.max(0, albumCoverScrollPeak / albumCollapseRange)) : 0);
  /** Cover scale when fully collapsed (80px / base size). The cover is always laid out at base size and only scaled. */
  let albumCoverEndScale = $derived(Math.min(1, MIN_ALBUM_DETAIL_COVER_SIZE_PX / albumCoverBaseSize));
  let shellStyle = $derived(
    `--album-detail-heading-base-height:${albumHeadingBaseHeight}px;--album-detail-cover-base-size:${albumCoverBaseSize}px;--album-detail-collapse-range:${albumCollapseRange}px;--album-detail-cover-end-scale:${albumCoverEndScale}`
    + (supportsScrollDrivenAlbumOverlay ? '' : `;--album-detail-collapse-progress:${albumCollapseProgress}`),
  );
  let albumDiscQuickIndices = $state<{
    ascending: QuickIndexSnapshot;
    descending: QuickIndexSnapshot;
  } | null>(null);
  const albumIndexSortActive = $derived(detailSort.field === 'position');
  let albumDiscQuickIndex = $derived(albumIndexSortActive
    ? albumDiscQuickIndices?.[detailSort.direction === 'asc' ? 'ascending' : 'descending']
    : undefined);
  const albumSortIndicator = $derived({ field: detailSort.field, direction: detailSort.direction });
  const detailTrackSort = $derived({
    field: detailSort.field === 'position' ? 'title' : detailSort.field,
    direction: detailSort.field === 'position' ? 'asc' : detailSort.direction,
  });
  const coverSourcesToDisplay = $derived(coverScanResolved ? coverUrls : initialCoverSources);
  const displayedCoverUrls = $derived(resolveAlbumArtworkDisplaySources(coverSourcesToDisplay, {
    fallbackUrl: defaultCoverUrl,
    resolved: !scanning && !scanError,
  }));
  let shellElement = $state<HTMLElement | null>(null);
  let summaryElement = $state<HTMLElement | null>(null);
  // Opt-in (localStorage moe.debug.albumReflow = "1"): logs the layout cost of the summary reflow while scrolling.
  $effect(() => {
    if (!shellElement || !summaryElement || !albumReflowProbeEnabled()) return;
    return startAlbumReflowProbe(shellElement, summaryElement);
  });
  let revision = 0;
  let detailDataRevision = 0;
  // The one cover scan of this page: it is started when the album is opened and ended when the page is left (the backend
  // stops reading files at once). It asks only through the detail commands, each call once.
  const coverScan = createAlbumDetailCoverScan({
    ensure: (args) => invokeCommand('album_detail_cover_ensure', args as never),
    cancel: (keys) => invokeCommand('album_detail_cover_cancel', { keys }),
  });
  function currentOrder(): AlbumTrackOrder {
    return {
      sort: detailSort.field === 'position' ? null : { field: detailSort.field, direction: detailSort.direction },
      indexSort: albumIndexSortActive ? detailSort.direction : null,
    };
  }

  async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
    return invokeCommand('library_get_album_page', {
      albumNames: album.aliases,
      ...currentOrder(),
      offset: request.offset,
      limit: request.limit,
    });
  }

  function playTrack(track: TrackSummary): void | Promise<void> {
    return onPlay(track, currentOrder());
  }

  function handleColumnSort(columnId: import('./ipc').TrackListColumnPreference['id']): void {
    saveDetailSort(toggleAlbumDetailSort(detailSort, columnId));
  }

  function handleIndexSort(): void {
    saveDetailSort(toggleAlbumDetailSort(detailSort, 'position'));
  }

  function handleListScrollTopChange(_scrollTop: number, collapseOffset: number): void {
    albumCoverScrollPeak = Math.min(albumCollapseRange, Math.max(0, collapseOffset));
  }

  function saveDetailSort(preference: import('./ipc').AlbumDetailSort): void {
    detailSort = preference;
    sortSaveQueue = sortSaveQueue
      .catch(() => undefined)
      .then(() => invokeCommand('settings_set_album_detail_sort', { preference }))
      .catch((error) => console.warn(tr('無法保存專輯歌曲排序設定'), error));
  }

  onMount(() => {
    let cancelled = false;
    const updateCoverBaseSize = () => {
      const narrow = window.matchMedia('(max-width: 620px)').matches;
      albumCoverBaseSize = narrow ? NARROW_ALBUM_DETAIL_COVER_SIZE_PX : DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX;
      // Heading = cover + vertical padding (14px narrow, 18px desktop on each side).
      albumHeadingBaseHeight = albumCoverBaseSize + (narrow ? 28 : 36);
    };
    updateCoverBaseSize();
    window.addEventListener('resize', updateCoverBaseSize);
    void invokeCommand('settings_get_album_detail_sort', {}).then((preference) => {
      if (!cancelled) detailSort = normalizeAlbumDetailSort(preference);
    }).catch((error) => {
      console.warn(tr('無法讀取專輯歌曲排序設定，使用曲目編號排序'), error);
    }).finally(() => {
      if (!cancelled) sortReady = true;
    });
    return () => {
      cancelled = true;
      window.removeEventListener('resize', updateCoverBaseSize);
    };
  });

  async function selectionKey(): Promise<string> {
    return sha256Hex(copyToArrayBuffer(new TextEncoder().encode(`${album.key}\0${album.trackCount}`)));
  }

  async function makeThumbnail(trackId: string): Promise<string | null> {
    return withAlbumArtworkSlot(async () => {
      let indexed = new Uint8Array();
      try { indexed = new Uint8Array(await getAlbumThumbnailBytesForTrack(trackId)); } catch { /* rebuild an absent or damaged track cache */ }
      if (indexed.length === 1 && indexed[0] === 0) return null;
      if (indexed.length > 3) return URL.createObjectURL(new Blob([copyToArrayBuffer(indexed)], { type: 'image/jpeg' }));

      let bytes: ArrayBuffer;
      try { bytes = await getTrackArtworkBytes(trackId); } catch { return null; }
      if (!bytes.byteLength) return null;
      const contentKey = await sha256Hex(bytes);
      let imageBytes: Uint8Array;
      try { imageBytes = new Uint8Array(await getAlbumThumbnailBytes(contentKey)); } catch { imageBytes = new Uint8Array(); }
      if (imageBytes.length === 0) {
        const mime = detectArtworkMimeType(bytes);
        if (!mime) return null;
        const blob = await createAlbumThumbnail(bytes, mime);
        imageBytes = new Uint8Array(await blob.arrayBuffer());
      }
      await putAlbumThumbnailBytesForTrack(trackId, contentKey, copyToArrayBuffer(imageBytes)).catch((error) => {
        console.warn(tr('無法保存專輯封面曲目索引'), error);
      });
      return URL.createObjectURL(new Blob([copyToArrayBuffer(imageBytes)], { type: 'image/jpeg' }));
    });
  }

  async function loadThumbnails(trackIds: string[], current: number): Promise<string[]> {
    const urls: string[] = [];
    for (const trackId of trackIds.slice(0, 4)) {
      if (current !== revision) break;
      try {
        const url = await makeThumbnail(trackId);
        if (url) urls.push(url);
      } catch (error) { console.warn(tr('無法載入專輯拼貼縮圖'), error); }
    }
    return urls;
  }

  /**
   * What the track list and the heading need (the quick index, the track count, the artists). Each is asked for and shown on
   * its own: none of it waits for the other, and none of it waits for the covers, which are looked through in the
   * background (`scanCovers`).
   */
  function loadDetailData(current: number): void {
    void invokeCommand('library_get_album_detail_facets', { albumNames: album.aliases })
      .then((facets) => {
        if (current !== detailDataRevision) return;
        albumDiscQuickIndices = {
          ascending: facets.quickIndexAscending,
          descending: facets.quickIndexDescending,
        };
        onTotalCount(facets.trackCount);
      })
      .catch((error) => { if (current === detailDataRevision) scanError = getErrorText(error); });
    void invokeCommand('library_get_album_detail_artists', { albumNames: album.aliases })
      .then((resolvedArtists) => { if (current === detailDataRevision) artists = resolvedArtists; })
      .catch(() => undefined);
  }

  async function scanCovers(): Promise<void> {
    const current = ++revision;
    scanning = true;
    scanError = null;
    try {
      const key = await selectionKey();

      // First the cover of the album card (its first readable cover, usually already cached): the page shows something at once
      // while the whole album is looked through for the representative covers below.
      if (!coverScanResolved && coverUrls.length === 0) {
        try {
          const quick = await coverScan.ensure({
            key: await sha256Hex(copyToArrayBuffer(new TextEncoder().encode(`${album.key} ${album.trackCount}`))),
            albumNames: album.aliases, trackCount: album.trackCount,
            representative: false, forceRebuild: false,
          });
          if (current !== revision) return;
          const quickUrls = await loadThumbnails(quick?.trackIds.slice(0, 1) ?? [], current);
          if (current !== revision) { quickUrls.forEach((url) => URL.revokeObjectURL(url)); return; }
          if (quickUrls.length > 0 && coverUrls.length === 0) {
            for (const url of ownedCoverUrls) URL.revokeObjectURL(url);
            ownedCoverUrls = quickUrls;
            coverUrls = quickUrls;
          } else {
            quickUrls.forEach((url) => URL.revokeObjectURL(url));
          }
        } catch { /* the full scan below still runs */ }
      }

      let selection = await coverScan.ensure({
        key, albumNames: album.aliases, trackCount: album.trackCount,
        representative: true, forceRebuild: false,
      });
      if (current !== revision || selection === null) return;
      let selectedIds = selection?.trackIds.slice(0, 4) ?? [];
      let urls = await loadThumbnails(selectedIds, current);
      if (current !== revision) { urls.forEach((url) => URL.revokeObjectURL(url)); return; }
      if (selectedIds.length > 0 && urls.length === 0) {
        // A stale selection can outlive removed/replaced source art; ask the backend to rebuild it.
        selection = await coverScan.ensure({
          key, albumNames: album.aliases, trackCount: album.trackCount,
          representative: true, forceRebuild: true,
        });
        if (current !== revision || selection === null) return;
        selectedIds = selection?.trackIds.slice(0, 4) ?? [];
        urls = await loadThumbnails(selectedIds, current);
      }
      // The scan picked exactly what is already on screen (same image bytes): keep the current sources,
      // so there is no src swap/reload and no selection-revision bump that would refresh caches.
      if (current === revision && !coverScanResolved
        && await albumCoverSourcesHaveSameContent(initialCoverSources, urls)) {
        urls.forEach((url) => URL.revokeObjectURL(url));
        return;
      }
      if (current === revision) {
        for (const url of ownedCoverUrls) URL.revokeObjectURL(url);
        ownedCoverUrls = urls;
        coverUrls = urls;
        coverScanResolved = true;
        advanceAlbumArtworkSelectionRevision(album.key);
        window.dispatchEvent(new CustomEvent('album-artwork-selection-updated', { detail: { albumKey: album.key } }));
      } else {
        urls.forEach((url) => URL.revokeObjectURL(url));
      }
    } catch (error) {
      if (current === revision) scanError = getErrorText(error);
    } finally {
      if (current === revision) scanning = false;
    }
  }

  /** The list, the quick index and the artists follow the library (`refreshKey`); the covers are not looked for again. */
  $effect(() => {
    void refreshKey;
    untrack(() => {
      const current = ++detailDataRevision;
      artists = [];
      albumDiscQuickIndices = null;
      loadDetailData(current);
    });
  });
  /** The covers are looked for once, when the album is opened (a remembered page at start counts as opened). */
  function startCoverScan(): void {
    void scanCovers();
  }
  onMount(() => { startCoverScan(); });
  onDestroy(() => {
    revision += 1;
    detailDataRevision += 1;
    coverScan.cancel();
    for (const url of ownedCoverUrls) URL.revokeObjectURL(url);
  });
</script>

<div
  class="album-detail-shell"
  class:scroll-driven-collapse={supportsScrollDrivenAlbumOverlay}
  style={shellStyle}
  bind:this={shellElement}
>
  <div class="album-detail-tracks">
    {#if sortReady}
      <TrackList
        query=""
        resetKey={`${album.key}:${refreshKey}:${detailSort.field}-${detailSort.direction}`}
        listId={`album-detail-${album.key}`}
        {selectedTrackId} {playbackReady} {isSendingPlaybackCommand} {columns} {columnWidths} {rowHeightPx} sort={detailTrackSort} {livePlayCount}
        fetchPage={fetchPage} onPlay={playTrack} onSort={handleColumnSort} {onColumnResize} {onTotalCount} sortable={true}
        sortableIndex={true} sortIndicator={albumSortIndicator} indexSortLabel={tr("依專輯曲目編號排序")} onIndexSort={handleIndexSort}
        quickIndexOverride={albumDiscQuickIndex}
        quickIndexAlbumNames={album.aliases}
        {groupingEnabled}
        getIndexLabel={(track) => formatAlbumTrackNumber(track.trackNumber)}
        leadingOverlayHeight={albumHeadingBaseHeight}
        leadingCollapseRange={albumCollapseRange}
        onViewportScrollTopChange={supportsScrollDrivenAlbumOverlay ? undefined : handleListScrollTopChange}
      />
    {/if}
  </div>
  <div class="album-detail-heading">
    <div class="album-detail-art">
      <AlbumArtworkMosaic sources={displayedCoverUrls} label={tr("{0} 封面拼貼", [album.displayName])} />
    </div>
    <div class="album-detail-summary" bind:this={summaryElement}>
      <div class="album-detail-artists">
        <span class="album-detail-value">{artists.length ? artists.join('．') : tr('未知')}</span>
      </div>
      <div class="album-detail-stats">
        <span><span class="album-detail-label">{tr("累積播放")}</span><span class="album-detail-value">{formatDurationHours(album.playedMs)}</span></span>
        <span><span class="album-detail-label">{tr("播放時長")}</span><span class="album-detail-value">{formatDurationHours(album.durationMs)}</span></span>
      </div>
      {#if scanError}<span class="album-detail-error" role="status">{tr("封面整理失敗：")}{scanError}</span>{/if}
      {#if scanning && coverUrls.length === 0}<span class="album-detail-status" role="status">{tr("正在整理封面…")}</span>{/if}
    </div>
  </div>
</div>

<style>
  .album-detail-shell { position:relative; min-width:0; min-height:0; flex:1 1 auto; overflow:hidden; }
  /*
   * The cover is always laid out at its base size (200px, 160px narrow) and the heading keeps its base height, so scrolling
   * never changes layout: the cover only gets transform: scale() and the summary only translate(), both driven by the
   * list's native scroll offset (CSS scroll timeline, compositor) when supported. Cover and summary follow one linear
   * progress, so the cover's visual size is base - scrollTop px and the summary stays beside it and vertically centred on it.
   * Without scroll timelines, the script writes --album-detail-collapse-progress (0..1) and the same declarations apply.
   */
  .album-detail-heading { position:absolute; z-index:4; top:0; right:0; left:0; display:flex; height:var(--album-detail-heading-base-height); box-sizing:border-box; align-items:center; gap:18px; padding:18px clamp(18px,2vw,34px); pointer-events:none; contain:layout style size; }
  @supports (animation-timeline: scroll()) and (timeline-scope: --album-detail-scroll) {
    .album-detail-shell.scroll-driven-collapse { timeline-scope: --album-detail-scroll; }

    .album-detail-shell.scroll-driven-collapse .album-detail-art {
      animation: album-detail-cover-collapse auto linear both;
      animation-duration: 1s;
      animation-timeline: --album-detail-scroll;
      animation-range: 0px var(--album-detail-collapse-range);
    }

    .album-detail-shell.scroll-driven-collapse .album-detail-summary {
      animation: album-detail-summary-collapse auto linear both;
      animation-duration: 1s;
      animation-timeline: --album-detail-scroll;
      animation-range: 0px var(--album-detail-collapse-range);
    }

    @keyframes album-detail-cover-collapse {
      from { transform: scale(1); }
      to { transform: scale(var(--album-detail-cover-end-scale, 1)); }
    }

    @keyframes album-detail-summary-collapse {
      from { transform: translateY(0); margin-left: 0; max-height: var(--album-detail-heading-base-height); }
      to {
        transform: translateY(calc(var(--album-detail-collapse-range, 0px) * -0.5));
        margin-left: calc(var(--album-detail-collapse-range, 0px) * -1);
        max-height: calc(var(--album-detail-heading-base-height) - var(--album-detail-collapse-range, 0px));
      }
    }
  }
  .album-detail-heading > * { pointer-events:auto; }
  .album-detail-art {
    width:var(--album-detail-cover-base-size,200px); height:var(--album-detail-cover-base-size,200px); flex:0 0 var(--album-detail-cover-base-size,200px); aspect-ratio:1; overflow:hidden; background:#000;
    border-radius:0;
    transform-origin:0 0;
    transform:scale(calc(1 - (1 - var(--album-detail-cover-end-scale, 1)) * var(--album-detail-collapse-progress, 0)));
    will-change:transform;
  }
  /*
   * Exception to the transform-only rule (ADR 0012), for this text block only: the text beside the cover re-wraps into the
   * room the shrinking cover frees. Its left edge moves left by the collapse range with the cover (margin-left) so its width
   * grows by the range, and its height limit shrinks by the range so it stays between the top of the page and the column
   * header, which rises by the same range (bottom edge = header top at every progress; the artists, the one block that may
   * not fit, are clipped, the stats stay). Containment (the heading's `contain: layout style size`, the summary's own
   * `contain: layout style`) keeps this layout work out of the cover, the header and the track list. The cover, the header
   * and the list keep their transform-only collapse. Reflow is driven by the same progress (timeline or fallback variable).
   */
  .album-detail-summary {
    display:flex; min-width:0; flex:1 1 0; flex-direction:column; gap:10px; overflow:hidden; contain:layout style;
    margin-left:calc(var(--album-detail-collapse-range, 0px) * var(--album-detail-collapse-progress, 0) * -1);
    max-height:calc(var(--album-detail-heading-base-height) - var(--album-detail-collapse-range, 0px) * var(--album-detail-collapse-progress, 0));
    transform:translateY(calc(var(--album-detail-collapse-range, 0px) * var(--album-detail-collapse-progress, 0) * -0.5));
  }
  .album-detail-artists { flex:0 1 auto; min-height:0; overflow:hidden; -webkit-mask-image:linear-gradient(to bottom, #000 calc(100% - 6px), transparent 100%); mask-image:linear-gradient(to bottom, #000 calc(100% - 6px), transparent 100%); }
  .album-detail-stats { flex:0 0 auto; }
  .album-detail-artists,
  .album-detail-stats,
  .album-detail-stats > span { display:flex; min-width:0; flex-wrap:wrap; align-items:baseline; gap:8px; }
  .album-detail-stats { column-gap:15px; row-gap:10px; }
  .album-detail-summary .album-detail-label { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-summary .album-detail-value { color:var(--text-soft); font-size:var(--font-body); }
  .album-detail-summary .album-detail-error,
  .album-detail-summary .album-detail-status { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-tracks { position:absolute; inset:0; min-height:0; }
  @media (max-width:620px) { .album-detail-heading { gap:14px; padding:14px; } }
</style>
