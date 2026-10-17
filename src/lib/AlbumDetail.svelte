<script lang="ts">
  import { tr } from './i18n.js';
  import { onDestroy, onMount } from 'svelte';
  import { getAlbumThumbnailBytes, getAlbumThumbnailBytesForTrack, getErrorText, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytesForTrack, type QuickIndexSnapshot, type TrackPage, type TrackPageRequest, type TrackSummary } from './ipc';
  import type { AlbumIndexEntry as AlbumEntry } from './ipc';
  import { advanceAlbumArtworkSelectionRevision, albumCoverSourcesHaveSameContent, copyToArrayBuffer, createAlbumThumbnail, resolveAlbumArtworkDisplaySources, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';
  import TrackList from './TrackList.svelte';
  import { formatDurationHours, formatAlbumTrackNumber } from './format';
  import { DEFAULT_ALBUM_DETAIL_SORT, normalizeAlbumDetailSort, toggleAlbumDetailSort } from './album-detail-sort.js';
  import { DESKTOP_ALBUM_DETAIL_COVER_SIZE_PX, MIN_ALBUM_DETAIL_COVER_SIZE_PX, NARROW_ALBUM_DETAIL_COVER_SIZE_PX, getAlbumDetailCollapseRange, getAlbumDetailCoverSize } from './album-detail-collapse.js';

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
    onPlay: (track: TrackSummary) => void | Promise<void>;
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
  let albumCoverScrollPeak = $state(0);
  let albumCoverSize = $derived(getAlbumDetailCoverSize(albumCoverBaseSize, albumCoverScrollPeak));
  let albumCollapseRange = $derived(getAlbumDetailCollapseRange(albumCoverBaseSize));
  let albumCollapseOffset = $derived(Math.min(albumCollapseRange, albumCoverScrollPeak));
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
  let revision = 0;
  async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
    return invokeCommand('library_get_album_page', {
      albumNames: album.aliases,
      sort: detailSort.field === 'position' ? null : { field: detailSort.field, direction: detailSort.direction },
      indexSort: albumIndexSortActive ? detailSort.direction : null,
      offset: request.offset,
      limit: request.limit,
    });
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

  async function scanCovers(): Promise<void> {
    const current = ++revision;
    scanning = true;
    scanError = null;
    artists = [];
    albumDiscQuickIndices = null;
    try {
      const key = await selectionKey();
      const facets = await invokeCommand('library_get_album_detail_facets', { albumNames: album.aliases });
      const resolvedArtists = await invokeCommand('library_get_album_detail_artists', {
        albumNames: album.aliases,
      });
      if (current !== revision) return;
      artists = resolvedArtists;
      albumDiscQuickIndices = {
        ascending: facets.quickIndexAscending,
        descending: facets.quickIndexDescending,
      };
      onTotalCount(facets.trackCount);

      let selection = await invokeCommand('album_artwork_selection_ensure', {
        key, albumNames: album.aliases, trackCount: album.trackCount,
        representative: true, forceRebuild: false,
      });
      if (current !== revision) return;
      let selectedIds = selection?.trackIds.slice(0, 4) ?? [];
      let urls = await loadThumbnails(selectedIds, current);
      if (current !== revision) { urls.forEach((url) => URL.revokeObjectURL(url)); return; }
      if (selectedIds.length > 0 && urls.length === 0) {
        // A stale selection can outlive removed/replaced source art; ask the backend to rebuild it.
        selection = await invokeCommand('album_artwork_selection_ensure', {
          key, albumNames: album.aliases, trackCount: album.trackCount,
          representative: true, forceRebuild: true,
        });
        if (current !== revision) return;
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

  $effect(() => {
    const key = refreshKey;
    const albumKey = album.key;
    void key;
    void albumKey;
    void scanCovers();
  });
  onDestroy(() => { revision += 1; for (const url of ownedCoverUrls) URL.revokeObjectURL(url); });
</script>

<div
  class="album-detail-shell"
  class:scroll-driven-collapse={supportsScrollDrivenAlbumOverlay}
  style={`--album-detail-heading-base-height:${albumHeadingBaseHeight}px;--album-detail-heading-height:${albumHeadingBaseHeight - albumCollapseOffset}px;--album-detail-cover-base-size:${albumCoverBaseSize}px;--album-detail-collapse-range:${albumCollapseRange}px;--album-detail-cover-min-size:${MIN_ALBUM_DETAIL_COVER_SIZE_PX}px`}
>
  <div class="album-detail-tracks">
    {#if sortReady}
      <TrackList
        query=""
        resetKey={`${album.key}:${refreshKey}:${detailSort.field}-${detailSort.direction}`}
        listId={`album-detail-${album.key}`}
        {selectedTrackId} {playbackReady} {isSendingPlaybackCommand} {columns} {columnWidths} {rowHeightPx} sort={detailTrackSort} {livePlayCount}
        fetchPage={fetchPage} {onPlay} onSort={handleColumnSort} {onColumnResize} {onTotalCount} sortable={true}
        sortableIndex={true} sortIndicator={albumSortIndicator} indexSortLabel={tr("依專輯曲目編號排序")} onIndexSort={handleIndexSort}
        quickIndexOverride={albumDiscQuickIndex}
        quickIndexAlbumNames={album.aliases}
        {groupingEnabled}
        getIndexLabel={(track) => formatAlbumTrackNumber(track.trackNumber)}
        leadingOverlayHeight={albumHeadingBaseHeight}
        leadingCollapseRange={albumCollapseRange}
        onViewportScrollTopChange={handleListScrollTopChange}
      />
    {/if}
  </div>
  <div
    class="album-detail-heading"
    style={`--album-detail-cover-size:${albumCoverSize}px`}
  >
    <div class="album-detail-art">
      <AlbumArtworkMosaic sources={displayedCoverUrls} label={tr("{0} 封面拼貼", [album.displayName])} />
    </div>
    <div class="album-detail-summary">
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
  .album-detail-heading { position:absolute; z-index:4; top:0; right:0; left:0; display:flex; height:var(--album-detail-heading-height); box-sizing:border-box; align-items:center; gap:18px; padding:18px clamp(18px,2vw,34px); pointer-events:none; }
  @supports (animation-timeline: scroll()) and (timeline-scope: --album-detail-scroll) {
    .album-detail-shell.scroll-driven-collapse { timeline-scope: --album-detail-scroll; }

    .album-detail-shell.scroll-driven-collapse .album-detail-heading {
      animation: album-detail-heading-collapse auto linear both;
      animation-timeline: --album-detail-scroll;
      animation-range: 0px var(--album-detail-collapse-range);
    }

    .album-detail-shell.scroll-driven-collapse .album-detail-art {
      animation: album-detail-cover-collapse auto linear both;
      animation-timeline: --album-detail-scroll;
      animation-range: 0px var(--album-detail-collapse-range);
    }

    @keyframes album-detail-heading-collapse {
      from { height: var(--album-detail-heading-base-height); }
      to { height: calc(var(--album-detail-heading-base-height) - var(--album-detail-collapse-range)); }
    }

    @keyframes album-detail-cover-collapse {
      from { width: var(--album-detail-cover-base-size); height: var(--album-detail-cover-base-size); flex-basis: var(--album-detail-cover-base-size); }
      to { width: var(--album-detail-cover-min-size); height: var(--album-detail-cover-min-size); flex-basis: var(--album-detail-cover-min-size); }
    }
  }
  .album-detail-heading > * { pointer-events:auto; }
  .album-detail-art { width:var(--album-detail-cover-size,200px); height:var(--album-detail-cover-size,200px); flex:0 0 var(--album-detail-cover-size,200px); aspect-ratio:1; overflow:hidden; border-radius:10px; background:#000; }
  .album-detail-summary { display:flex; min-width:0; flex-direction:column; gap:10px; }
  .album-detail-artists,
  .album-detail-stats,
  .album-detail-stats > span { display:flex; min-width:0; flex-wrap:wrap; align-items:baseline; gap:8px; }
  .album-detail-stats { gap:18px; }
  .album-detail-summary .album-detail-label { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-summary .album-detail-value { color:var(--text-soft); font-size:var(--font-body); }
  .album-detail-summary .album-detail-error,
  .album-detail-summary .album-detail-status { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-tracks { position:absolute; inset:0; min-height:0; }
  @media (max-width:620px) { .album-detail-heading { gap:14px; padding:14px; } }
</style>
