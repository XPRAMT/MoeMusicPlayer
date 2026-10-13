<script lang="ts">
  import ArtistIndex from '../src/lib/ArtistIndex.svelte';

  let selectedArtist = $state('尚未選擇');
  let artistCount = $state<number | null>(null);
</script>

<div class="workspace" style="height: 720px; width: 1120px; display: flex; flex-direction: column;">
  <header class="topbar" data-testid="topbar" style="height: 64px; min-height: 64px; flex: 0 0 64px;">
    <div>演出者</div>
    {#if artistCount !== null}<p class="page-title-count" data-testid="artist-count">{artistCount.toLocaleString()} 位演出者</p>{/if}
  </header>
  <div class="page-scroll is-list-page" data-testid="page-scroll">
    <div class="page-content is-list-page wide-list-page" data-testid="page-content">
      <section class="artist-section" data-testid="artist-section">
        <ArtistIndex
          refreshKey={0}
          onCountChange={(count) => (artistCount = count)}
          onSelect={(artist) => (selectedArtist = artist.displayName)}
        />
      </section>
    </div>
  </div>
  <output data-testid="selected-artist">{selectedArtist}</output>
</div>
