<script lang="ts">
  import { onDestroy } from 'svelte';
  import { getAlbumThumbnailBytes, getAlbumThumbnailBytesForTrack, getErrorText, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytesForTrack, type QuickIndexSnapshot, type TrackPage, type TrackPageRequest, type TrackSummary } from './ipc';
  import type { AlbumIndexEntry as AlbumEntry } from './ipc';
  import { copyToArrayBuffer, createAlbumThumbnail, resolveAlbumArtworkDisplaySources, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
  import defaultCoverUrl from '../../icon/DefaultCover.jpg?inline';
  import TrackList from './TrackList.svelte';
  import { formatAlbumDetailSummary, formatAlbumTrackNumber } from './format';

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
    sort: import('./ipc').LibraryListSort;
    livePlayCount: { trackId: string; playedMs: number }[];
    refreshKey: number;
    onPlay: (track: TrackSummary) => void | Promise<void>;
    onSort: (columnId: import('./ipc').TrackListColumnPreference['id']) => void;
    onColumnResize: (columnId: import('./ipc').TrackListColumnPreference['id'], widthPx: number) => void;
    onTotalCount: (count: number | null) => void;
  }
  let { album, initialCoverSources = [], selectedTrackId, playbackReady, isSendingPlaybackCommand, columns, columnWidths, rowHeightPx, groupingEnabled, sort, livePlayCount, refreshKey, onPlay, onSort, onColumnResize, onTotalCount }: Props = $props();
  let coverUrls = $state<string[]>([]);
  let coverScanResolved = $state(false);
  let ownedCoverUrls: string[] = [];
  let artists = $state<string[]>([]);
  let scanning = $state(true);
  let scanError = $state<string | null>(null);
  let albumDiscQuickIndices = $state<{
    ascending: QuickIndexSnapshot;
    descending: QuickIndexSnapshot;
  } | null>(null);
  let albumIndexSortActive = $state(true);
  let albumIndexSortDirection = $state<'asc' | 'desc'>('asc');
  let albumDiscQuickIndex = $derived(albumIndexSortActive
    ? albumDiscQuickIndices?.[albumIndexSortDirection === 'asc' ? 'ascending' : 'descending']
    : undefined);
  const albumSortIndicator = $derived(albumIndexSortActive
    ? { field: 'position', direction: albumIndexSortDirection }
    : sort);
  const coverSourcesToDisplay = $derived(coverScanResolved ? coverUrls : initialCoverSources);
  const displayedCoverUrls = $derived(resolveAlbumArtworkDisplaySources(coverSourcesToDisplay, {
    fallbackUrl: defaultCoverUrl,
    resolved: !scanning && !scanError,
  }));
  let revision = 0;
  async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
    return invokeCommand('library_get_album_page', {
      albumNames: album.aliases,
      sort: albumIndexSortActive ? null : request.sort,
      indexSort: albumIndexSortActive ? albumIndexSortDirection : null,
      offset: request.offset,
      limit: request.limit,
    });
  }

  function handleColumnSort(columnId: import('./ipc').TrackListColumnPreference['id']): void {
    albumIndexSortActive = false;
    onSort(columnId);
  }

  function handleIndexSort(): void {
    if (albumIndexSortActive) {
      albumIndexSortDirection = albumIndexSortDirection === 'asc' ? 'desc' : 'asc';
    } else {
      albumIndexSortActive = true;
      albumIndexSortDirection = 'asc';
    }
  }

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
        console.warn('無法保存專輯封面曲目索引', error);
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
      } catch (error) { console.warn('無法載入專輯拼貼縮圖', error); }
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
      if (current === revision) {
        for (const url of ownedCoverUrls) URL.revokeObjectURL(url);
        ownedCoverUrls = urls;
        coverUrls = urls;
        coverScanResolved = true;
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

<div class="album-detail-shell">
  <div class="album-detail-heading">
    <div class="album-detail-art">
      <AlbumArtworkMosaic sources={displayedCoverUrls} label={`${album.displayName} 封面拼貼`} />
    </div>
    <div class="album-detail-summary">
      <strong>{album.displayName}</strong>
      <span>{artists.length ? artists.join('．') : '—'}</span>
      <span>{formatAlbumDetailSummary(album.trackCount, album.playedMs, album.durationMs)}</span>
      {#if scanError}<span class="album-detail-error" role="status">封面整理失敗：{scanError}</span>{/if}
      {#if scanning && coverUrls.length === 0}<span class="album-detail-status" role="status">正在整理封面…</span>{/if}
    </div>
  </div>
  <div class="album-detail-tracks">
    <TrackList
      query=""
      resetKey={`${album.key}:${refreshKey}:${albumIndexSortActive ? `index-${albumIndexSortDirection}` : `column-${sort.field}-${sort.direction}`}`}
      listId={`album-detail-${album.key}`}
      {selectedTrackId} {playbackReady} {isSendingPlaybackCommand} {columns} {columnWidths} {rowHeightPx} {sort} {livePlayCount}
      fetchPage={fetchPage} {onPlay} onSort={handleColumnSort} {onColumnResize} {onTotalCount} sortable={true}
      sortableIndex={true} sortIndicator={albumSortIndicator} indexSortLabel="依專輯曲目編號排序" onIndexSort={handleIndexSort}
      quickIndexOverride={albumDiscQuickIndex}
      quickIndexAlbumNames={album.aliases}
      {groupingEnabled}
      getIndexLabel={(track) => formatAlbumTrackNumber(track.trackNumber)}
    />
  </div>
</div>

<style>
  .album-detail-shell { display:flex; min-width:0; min-height:0; flex:1 1 auto; flex-direction:column; }
  .album-detail-heading { display:flex; flex:0 0 auto; align-items:center; gap:18px; padding:18px clamp(18px,2vw,34px) 20px; }
  .album-detail-art { width:148px; flex:0 0 148px; aspect-ratio:1; overflow:hidden; border-radius:10px; background:#000; }
  .album-detail-summary { display:flex; min-width:0; flex-direction:column; gap:8px; }
  .album-detail-summary strong { overflow:hidden; font-size:var(--font-title); font-weight:600; text-overflow:ellipsis; }
  .album-detail-summary span { color:var(--text-soft); font-size:var(--font-body); }
  .album-detail-summary .album-detail-error,
  .album-detail-summary .album-detail-status { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-tracks { display:flex; min-height:0; flex:1 1 auto; flex-direction:column; }
  @media (max-width:620px) { .album-detail-heading { gap:14px; padding:14px; } .album-detail-art { width:116px; flex-basis:116px; } }
</style>
