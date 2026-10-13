<script lang="ts">
  import AlbumIndex from '../src/lib/AlbumIndex.svelte';
  import AlbumDetail from '../src/lib/AlbumDetail.svelte';
  import AlbumArtworkMosaic from '../src/lib/AlbumArtworkMosaic.svelte';
  import type { AlbumEntry } from '../src/lib/album-identities.js';
  import { DEFAULT_LIBRARY_SORT, DEFAULT_TRACK_COLUMN_PREFERENCES, DEFAULT_TRACK_COLUMN_WIDTHS, DEFAULT_TRACK_ROW_HEIGHT_PX } from '../src/lib/track-columns.js';

  let count = $state<number | null>(null);
  let selectedAlbum = $state<AlbumEntry | null>(null);
</script>

<main class="album-test-workspace" data-testid="workspace">
  <header>專輯</header>
  <div class="mosaic-layout-fixtures" aria-hidden="true">
    <div data-testid="mosaic-one"><AlbumArtworkMosaic sources={['/cover-1.jpg']} /></div>
    <div data-testid="mosaic-two"><AlbumArtworkMosaic sources={['/cover-1.jpg', '/cover-2.jpg']} /></div>
    <div data-testid="mosaic-three"><AlbumArtworkMosaic sources={['/cover-1.jpg', '/cover-2.jpg', '/cover-3.jpg']} /></div>
    <div data-testid="mosaic-four"><AlbumArtworkMosaic sources={['/cover-1.jpg', '/cover-2.jpg', '/cover-3.jpg', '/cover-4.jpg']} /></div>
  </div>
  {#if selectedAlbum}
    <button data-testid="album-detail-back" type="button" onclick={() => (selectedAlbum = null)}>返回專輯</button>
    <AlbumDetail
      album={selectedAlbum}
      selectedTrackId={null}
      playbackReady={true}
      isSendingPlaybackCommand={false}
      columns={DEFAULT_TRACK_COLUMN_PREFERENCES}
      columnWidths={DEFAULT_TRACK_COLUMN_WIDTHS}
      rowHeightPx={DEFAULT_TRACK_ROW_HEIGHT_PX}
      sort={DEFAULT_LIBRARY_SORT}
      livePlayCount={[]}
      refreshKey={0}
      onPlay={() => {}}
      onSort={() => {}}
      onColumnResize={() => {}}
      onTotalCount={() => {}}
    />
  {/if}
  <div class="album-index-holder" hidden={!!selectedAlbum}>
    <AlbumIndex
      refreshKey={0}
      active={!selectedAlbum}
      onCountChange={(value) => (count = value)}
      onRowsChange={() => {}}
      onSelect={(album) => (selectedAlbum = album)}
    />
  </div>
  <output data-testid="album-count">{count ?? ''} 張專輯</output>
</main>

<style>
  .album-test-workspace { display:flex; width:640px; height:560px; flex-direction:column; }
  .album-test-workspace header { height:48px; flex:0 0 48px; }
  .album-index-holder { display:flex; min-width:0; min-height:0; flex:1 1 auto; }
  .album-index-holder[hidden] { display:none; }
  .mosaic-layout-fixtures { position:fixed; top:0; left:-1000px; width:168px; height:504px; pointer-events:none; }
  .mosaic-layout-fixtures > div { width:168px; height:168px; }
</style>
