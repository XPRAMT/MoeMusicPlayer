<script lang="ts">
  import ArtistIndex from '../src/lib/ArtistIndex.svelte';

  let selectedArtist = $state('尚未選擇');
  let artistCount = $state<number | null>(null);
  let artistTrackCount = $state<number | null>(null);
  let activeView = $state<'artists' | 'artist-detail'>('artists');
  let rememberedArtistView = $state<'artists' | 'artist-detail'>('artists');
  let keepArtistIndexForDetail = $state(false);
</script>

<div class="workspace" style="height: 720px; width: 1120px; display: flex; flex-direction: column;">
  <header class="topbar" data-testid="topbar" style="height: 64px; min-height: 64px; flex: 0 0 64px;">
    {#if activeView === 'artist-detail'}
      <button type="button" aria-label="返回演出者首頁" onclick={() => { rememberedArtistView = 'artists'; activeView = 'artists'; }}>演出者</button>
    {:else}
      <div>演出者</div>
    {/if}
    {#if activeView === 'artist-detail'}
      <p class="page-title-count" data-testid="artist-track-count">{artistTrackCount === null ? '' : `${artistTrackCount.toLocaleString()} `}首曲目</p>
    {:else}
      <p class="page-title-count" data-testid="artist-count">{artistCount === null ? '' : `${artistCount.toLocaleString()} `}位演出者</p>
    {/if}
  <button type="button" data-testid="nav-library" onclick={() => { keepArtistIndexForDetail = false; activeView = 'library'; }}>曲庫</button>
    <button type="button" data-testid="nav-artists" onclick={() => (activeView = selectedArtist === '尚未選擇' ? 'artists' : rememberedArtistView)}>演出者</button>
  </header>
  <div class="page-scroll is-list-page" data-testid="page-scroll">
    <div class="page-content is-list-page wide-list-page" data-testid="page-content">
      {#if activeView === 'artists' || (activeView === 'artist-detail' && keepArtistIndexForDetail)}
        <section class="artist-section" data-testid="artist-section" hidden={activeView !== 'artists'}>
          <ArtistIndex
          refreshKey={0}
          onCountChange={(count) => (artistCount = count)}
          onRowsChange={() => {}}
          onSelect={(artist) => {
            selectedArtist = artist.displayName;
            artistTrackCount = null;
            keepArtistIndexForDetail = true;
            rememberedArtistView = 'artist-detail';
            activeView = 'artist-detail';
            window.setTimeout(() => (artistTrackCount = 160), 250);
          }}
          />
        </section>
      {/if}
      {#if selectedArtist !== '尚未選擇' && activeView === 'artist-detail'}
        <section class="artist-detail-section" data-testid="artist-detail">
          <div>演出者．{selectedArtist}</div>
        </section>
      {/if}
    </div>
  </div>
  <output data-testid="selected-artist">{selectedArtist}</output>
</div>
