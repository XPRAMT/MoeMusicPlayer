<script lang="ts">
import { onDestroy } from 'svelte';
  import { getAlbumThumbnailBytes, getErrorText, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytes, type TrackPage, type TrackPageRequest, type TrackSummary } from './ipc';
  import type { AlbumEntry } from './album-identities.js';
import { ALBUM_ART_SIMILARITY_DISTANCE, copyToArrayBuffer, createAlbumThumbnail, perceptualHashArtwork, rankAlbumArtwork, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import TrackList from './TrackList.svelte';
  import { formatAlbumTrackNumber } from './format';

  interface Props {
    album: AlbumEntry;
    selectedTrackId: string | null;
    playbackReady: boolean;
    isSendingPlaybackCommand: boolean;
    columns: import('./ipc').TrackListColumnPreference[];
    columnWidths: import('./ipc').TrackColumnWidths;
    rowHeightPx: number;
    sort: import('./ipc').LibraryListSort;
    livePlayCount: { trackId: string; playedMs: number }[];
    refreshKey: number;
    onPlay: (track: TrackSummary) => void | Promise<void>;
    onSort: (columnId: import('./ipc').TrackListColumnPreference['id']) => void;
    onColumnResize: (columnId: import('./ipc').TrackListColumnPreference['id'], widthPx: number) => void;
    onTotalCount: (count: number | null) => void;
  }
  let { album, selectedTrackId, playbackReady, isSendingPlaybackCommand, columns, columnWidths, rowHeightPx, sort, livePlayCount, refreshKey, onPlay, onSort, onColumnResize, onTotalCount }: Props = $props();
  let coverUrls = $state<string[]>([]);
  let scanning = $state(true);
  let scanError = $state<string | null>(null);
  let revision = 0;
  const PAGE_SIZE = 100;

  async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
    return invokeCommand('library_get_album_page', { albumNames: album.aliases, offset: request.offset, limit: request.limit });
  }

  async function readHash(trackId: string): Promise<bigint | null> {
    return withAlbumArtworkSlot(async () => {
      let bytes: ArrayBuffer;
      try { bytes = await getTrackArtworkBytes(trackId); } catch { return null; }
      if (bytes.byteLength === 0) return null;
      const mime = detectArtworkMimeType(bytes);
      if (!mime) return null;
      try { return await perceptualHashArtwork(bytes, mime); } catch { return null; }
    });
  }

  async function makeThumbnail(trackId: string): Promise<string | null> {
    return withAlbumArtworkSlot(async () => {
      let bytes: ArrayBuffer;
      try { bytes = await getTrackArtworkBytes(trackId); } catch { return null; }
      if (!bytes.byteLength) return null;
      const key = await sha256Hex(bytes);
      let imageBytes: Uint8Array;
      try { imageBytes = new Uint8Array(await getAlbumThumbnailBytes(key)); } catch { imageBytes = new Uint8Array(); }
      if (imageBytes.length === 0) {
        const mime = detectArtworkMimeType(bytes);
        if (!mime) return null;
        const blob = await createAlbumThumbnail(bytes, mime);
        imageBytes = new Uint8Array(await blob.arrayBuffer());
        await putAlbumThumbnailBytes(key, copyToArrayBuffer(imageBytes)).catch((error) => {
          console.warn('無法保存專輯拼貼縮圖快取', error);
        });
      }
      return URL.createObjectURL(new Blob([copyToArrayBuffer(imageBytes)], { type: 'image/jpeg' }));
    });
  }

  async function scanCovers(): Promise<void> {
    const current = ++revision;
    scanning = true;
    scanError = null;
    const candidates: { trackId: string; hash: bigint }[] = [];
    try {
      let total = 0;
      for (let offset = 0; ; offset += PAGE_SIZE) {
        const page = await fetchPage({ query: '', offset, limit: PAGE_SIZE });
        if (current !== revision) return;
        total = page.totalCount;
        for (const track of page.items) {
          const hash = await readHash(track.id);
          if (hash !== null) candidates.push({ trackId: track.id, hash });
          if (current !== revision) return;
        }
        if (page.items.length === 0 || offset + page.items.length >= total) break;
      }
      const selected = rankAlbumArtwork(candidates, ALBUM_ART_SIMILARITY_DISTANCE).slice(0, 4);
      const urls: string[] = [];
      for (const item of selected) {
        if (current !== revision) return;
        try {
          const url = await makeThumbnail(item.trackId);
          if (url) urls.push(url);
        } catch (error) {
          console.warn('無法快取專輯拼貼封面', error);
        }
      }
      if (current === revision) {
        for (const url of coverUrls) URL.revokeObjectURL(url);
        coverUrls = urls;
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
  onDestroy(() => { revision += 1; for (const url of coverUrls) URL.revokeObjectURL(url); });

  const layoutClass = $derived(`cover-count-${Math.max(1, coverUrls.length)}`);
</script>

<div class="album-detail-shell">
  <div class="album-detail-heading">
<div class={`album-detail-art ${layoutClass}`}>
      {#each coverUrls as url, index (url)}<img src={url} alt="專輯封面 {index + 1}" />{/each}
      {#if coverUrls.length === 0}<span aria-hidden="true"></span>{/if}
    </div>
    <div class="album-detail-summary">
      <strong>{album.unmarked ? '未標記專輯' : album.displayName}</strong>
      <span>{scanning ? '正在整理封面…' : scanError ? `封面整理失敗：${scanError}` : `${album.trackCount.toLocaleString()} 首曲目`}</span>
      {#if !scanning && coverUrls.length > 0}<span class="album-detail-cover-count">{coverUrls.length} 張代表封面</span>{/if}
    </div>
  </div>
  <div class="album-detail-tracks">
    <TrackList
      query=""
      resetKey={`${album.key}:${refreshKey}`}
      listId={`album-detail-${album.key}`}
      {selectedTrackId} {playbackReady} {isSendingPlaybackCommand} {columns} {columnWidths} {rowHeightPx} {sort} {livePlayCount}
      fetchPage={fetchPage} {onPlay} {onSort} {onColumnResize} {onTotalCount} sortable={false}
      getIndexLabel={(track) => formatAlbumTrackNumber(track.trackNumber, track.discNumber)}
    />
  </div>
</div>

<style>
  .album-detail-shell { display:flex; min-width:0; min-height:0; flex:1 1 auto; flex-direction:column; }
  .album-detail-heading { display:flex; flex:0 0 auto; align-items:center; gap:18px; padding:18px clamp(18px,2vw,34px) 20px; }
  .album-detail-art { display:grid; width:148px; flex:0 0 148px; aspect-ratio:1; overflow:hidden; grid-template-columns:repeat(2,minmax(0,1fr)); grid-template-rows:repeat(2,minmax(0,1fr)); border:1px solid var(--line); border-radius:10px; background:#000; }
  .album-detail-art img { display:block; width:100%; height:100%; min-width:0; min-height:0; object-fit:cover; }
  .album-detail-art.cover-count-1 img { grid-column:1 / -1; grid-row:1 / -1; }
  .album-detail-art.cover-count-2 { grid-template-columns:1fr; grid-template-rows:repeat(2,minmax(0,1fr)); }
  .album-detail-art.cover-count-3 { grid-template-columns:1fr; grid-template-rows:repeat(3,minmax(0,1fr)); }
  .album-detail-art.cover-count-3 img { grid-column:1; }
  .album-detail-art.cover-count-3 img:nth-child(1) { grid-row:1; }
  .album-detail-art.cover-count-3 img:nth-child(2) { grid-row:2; }
  .album-detail-art.cover-count-3 img:nth-child(3) { grid-row:3; }
  .album-detail-summary { display:flex; min-width:0; flex-direction:column; gap:8px; }
  .album-detail-summary strong { overflow:hidden; font-size:var(--font-title); font-weight:600; text-overflow:ellipsis; }
  .album-detail-summary span { color:var(--text-soft); font-size:var(--font-body); }
  .album-detail-summary .album-detail-cover-count { color:var(--muted); font-size:var(--font-caption); }
  .album-detail-tracks { display:flex; min-height:0; flex:1 1 auto; flex-direction:column; }
  @media (max-width:620px) { .album-detail-heading { gap:14px; padding:14px; } .album-detail-art { width:116px; flex-basis:116px; } }
</style>
