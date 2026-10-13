<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { NowPlayingLayout } from './ipc';

  interface Props {
    layout: NowPlayingLayout;
    lyricsVisible?: boolean;
    artworkVisible?: boolean;
    artwork: Snippet;
    lyrics: Snippet;
  }

  let { layout, lyricsVisible = true, artworkVisible = true, artwork, lyrics }: Props = $props();
  let paneOrder = $derived(layout === 'b' ? ['lyrics', 'artwork'] : ['artwork', 'lyrics']);
</script>

<div class="now-playing-card" class:layout-b={layout === 'b'} class:lyrics-hidden={!lyricsVisible} class:artwork-hidden={!artworkVisible} data-layout={layout} data-lyrics-visible={lyricsVisible} data-artwork-visible={artworkVisible}>
  {#each paneOrder as pane (pane)}
    {#if pane === 'artwork'}
      {#if artworkVisible}
        <div class="now-playing-artwork" data-layout-pane="artwork">
          {@render artwork()}
        </div>
      {/if}
    {:else}
      <section class="now-playing-lyrics" data-layout-pane="lyrics" aria-hidden={!lyricsVisible} aria-labelledby="lyrics-heading">
        {@render lyrics()}
      </section>
    {/if}
  {/each}
</div>
