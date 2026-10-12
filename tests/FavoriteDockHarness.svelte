<script lang="ts">
  import FavoritePopover from '../src/lib/FavoritePopover.svelte';

  const tracks = ['track-one', 'track-two', 'track-three'];
  let trackIndex = $state(0);
  const platform = 'windows' as const;

  function nextTrack(): void {
    trackIndex = (trackIndex + 1) % tracks.length;
  }
</script>

<div class="app-shell" style="--text: rgb(230, 230, 230); --text-soft: rgb(170, 170, 170); --text-rgb: 230, 230, 230; --accent: rgb(0, 220, 255); --accent-text: rgb(0, 220, 255);">
  <main>
    <button type="button" data-testid="next-track" onclick={nextTrack}>下一首測試曲目</button>
    <output data-testid="current-track">{tracks[trackIndex]}</output>
  </main>
  <footer class="player-dock">
    <div class="dock-controls">
      <FavoritePopover trackId={tracks[trackIndex]} {platform} />
      <button class="control-button control-active" type="button" aria-label="隨機播放" aria-pressed="true">隨機</button>
    </div>
  </footer>
</div>
