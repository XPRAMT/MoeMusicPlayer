<script lang="ts">
  import ArtistIndex from '../src/lib/ArtistIndex.svelte';

  let selectedArtist = $state('尚未選擇');
</script>

<div class="workspace" style="height: 720px; width: 1120px; display: flex; flex-direction: column;">
  <header class="topbar" data-testid="topbar" style="height: 64px; min-height: 64px; flex: 0 0 64px;">演出者</header>
  <div class="page-scroll is-list-page" data-testid="page-scroll">
    <div class="page-content is-list-page wide-list-page" data-testid="page-content">
      <section class="artist-section" data-testid="artist-section">
        <ArtistIndex
          refreshKey={0}
          onSelect={(artist) => (selectedArtist = artist.displayName)}
        />
      </section>
    </div>
  </div>
  <output data-testid="selected-artist">{selectedArtist}</output>
</div>
