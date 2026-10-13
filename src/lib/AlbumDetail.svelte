<script lang="ts">
  import { onDestroy } from 'svelte';
  import { getAlbumThumbnailBytes, getAlbumThumbnailBytesForTrack, getErrorText, getTrackArtworkBytes, invokeCommand, putAlbumThumbnailBytesForTrack, type TrackPage, type TrackPageRequest, type TrackSummary } from './ipc';
  import type { AlbumEntry } from './album-identities.js';
  import { ALBUM_ART_SIMILARITY_DISTANCE, copyToArrayBuffer, createAlbumThumbnail, perceptualHashArtwork, rankAlbumArtwork, sha256Hex, withAlbumArtworkSlot } from './album-artwork.js';
  import { detectArtworkMimeType } from './active-track-artwork';
  import { splitArtists } from './artist-names.js';
  import { buildArtistIdentityMappings, preferredArtistName } from './artist-identities.js';
  import AlbumArtworkMosaic from './AlbumArtworkMosaic.svelte';
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
  let artists = $state<string[]>([]);
  let scanning = $state(true);
  let scanError = $state<string | null>(null);
  let revision = 0;
  const PAGE_SIZE = 100;

  async function fetchPage(request: TrackPageRequest): Promise<TrackPage> {
    return invokeCommand('library_get_album_page', { albumNames: album.aliases, sort: request.sort, offset: request.offset, limit: request.limit });
  }

  async function selectionKey(): Promise<string> {
    return sha256Hex(copyToArrayBuffer(new TextEncoder().encode(`${album.key}\0${album.trackCount}`)));
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

  function countArtists(counts: Map<string, number>, track: TrackSummary): void {
    for (const name of splitArtists(track.artist)) {
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }

  async function resolveArtists(counts: Map<string, number>): Promise<string[]> {
    const identities = await buildArtistIdentityMappings([...counts.keys()]);
    const groups = new Map<string, { names: { name: string; count: number }[]; traditionalByName: Map<string, string> }>();
    for (const identity of identities) {
      let group = groups.get(identity.key);
      if (!group) {
        group = { names: [], traditionalByName: new Map() };
        groups.set(identity.key, group);
      }
      group.names.push({ name: identity.name, count: counts.get(identity.name) ?? 0 });
      group.traditionalByName.set(identity.name, identity.traditional);
    }
    return [...groups]
      .sort(([left], [right]) => left.localeCompare(right, 'zh-Hant', { numeric: true, sensitivity: 'base' }))
      .map(([, group]) => preferredArtistName(group.names, group.traditionalByName));
  }

  async function saveSelection(key: string, trackIds: string[], representative: boolean): Promise<void> {
    if (trackIds.length === 0) return;
    await invokeCommand('album_artwork_selection_put', {
      key,
      selection: { trackCount: album.trackCount, trackIds, representative },
    });
    if (representative) {
      window.dispatchEvent(new CustomEvent('album-artwork-selection-updated', { detail: { albumKey: album.key } }));
    }
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

  async function scanHashes(current: number): Promise<{ trackId: string; hash: bigint }[]> {
    const candidates: { trackId: string; hash: bigint }[] = [];
    let total = 0;
    for (let offset = 0; ; offset += PAGE_SIZE) {
      const page = await fetchPage({ query: '', offset, limit: PAGE_SIZE });
      if (current !== revision) return [];
      total = page.totalCount;
      for (const track of page.items) {
        const hash = await readHash(track.id);
        if (hash !== null) candidates.push({ trackId: track.id, hash });
        if (current !== revision) return [];
      }
      if (page.items.length === 0 || offset + page.items.length >= total) break;
    }
    return candidates;
  }

  async function scanCovers(): Promise<void> {
    const current = ++revision;
    scanning = true;
    scanError = null;
    artists = [];
    try {
      const key = await selectionKey();
      let stored: import('./ipc').AlbumArtworkSelection | null = null;
      try { stored = await invokeCommand('album_artwork_selection_get', { key }); } catch { /* rebuild cache on older installations */ }
      const usable = stored !== null && stored.trackCount === album.trackCount && stored.representative;
      const artistCounts = new Map<string, number>();
      let total = 0;
      for (let offset = 0; ; offset += PAGE_SIZE) {
        const page = await fetchPage({ query: '', offset, limit: PAGE_SIZE });
        if (current !== revision) return;
        total = page.totalCount;
        for (const track of page.items) countArtists(artistCounts, track);
        if (page.items.length === 0 || offset + page.items.length >= total) break;
      }
      const resolvedArtists = await resolveArtists(artistCounts);
      if (current !== revision) return;
      artists = resolvedArtists;

      let selectedIds = usable && stored ? stored.trackIds.slice(0, 4) : [];
      if (selectedIds.length === 0) {
        const candidates = await scanHashes(current);
        if (current !== revision) return;
        selectedIds = rankAlbumArtwork(candidates, ALBUM_ART_SIMILARITY_DISTANCE)
          .slice(0, 4).map((item) => item.trackId);
        if (selectedIds.length) await saveSelection(key, selectedIds, true).catch((error) => {
          console.warn('無法保存專輯代表封面索引', error);
        });
      }
      let urls = await loadThumbnails(selectedIds, current);
      if (current !== revision) { urls.forEach((url) => URL.revokeObjectURL(url)); return; }
      if (usable && urls.length === 0) {
        // A stale selection can outlive a removed or replaced track; rebuild it once.
        const candidates = await scanHashes(current);
        if (current !== revision) return;
        selectedIds = rankAlbumArtwork(candidates, ALBUM_ART_SIMILARITY_DISTANCE)
          .slice(0, 4).map((item) => item.trackId);
        urls = await loadThumbnails(selectedIds, current);
        if (selectedIds.length) await saveSelection(key, selectedIds, true).catch((error) => {
          console.warn('無法更新專輯代表封面索引', error);
        });
      }
      if (current === revision) {
        for (const url of coverUrls) URL.revokeObjectURL(url);
        coverUrls = urls;
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
  onDestroy(() => { revision += 1; for (const url of coverUrls) URL.revokeObjectURL(url); });
</script>

<div class="album-detail-shell">
  <div class="album-detail-heading">
    <div class="album-detail-art">
      <AlbumArtworkMosaic sources={coverUrls} label={`${album.displayName} 封面拼貼`} />
    </div>
    <div class="album-detail-summary">
      <strong>{album.unmarked ? '未標記專輯' : album.displayName}</strong>
      <span>{artists.length ? artists.join('．') : '—'}</span>
      <span>{album.trackCount.toLocaleString()} 首曲目</span>
      {#if scanError}<span class="album-detail-error" role="status">封面整理失敗：{scanError}</span>{/if}
      {#if scanning && coverUrls.length === 0}<span class="album-detail-status" role="status">正在整理封面…</span>{/if}
    </div>
  </div>
  <div class="album-detail-tracks">
    <TrackList
      query=""
      resetKey={`${album.key}:${refreshKey}`}
      listId={`album-detail-${album.key}`}
      {selectedTrackId} {playbackReady} {isSendingPlaybackCommand} {columns} {columnWidths} {rowHeightPx} {sort} {livePlayCount}
      fetchPage={fetchPage} {onPlay} {onSort} {onColumnResize} {onTotalCount} sortable={true}
      getIndexLabel={(track) => formatAlbumTrackNumber(track.trackNumber, track.discNumber)}
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
