<script lang="ts">
  interface Props {
    sources: string[];
    label?: string;
  }

  let { sources, label = '專輯封面' }: Props = $props();
  const layout = $derived(`cover-count-${Math.max(1, Math.min(4, sources.length))}`);
</script>

<div class={`album-artwork-mosaic ${layout}`} role="img" aria-label={label}>
  {#each sources.slice(0, 4) as source, index (source)}
    <img src={source} alt="" aria-hidden="true" loading="lazy" />
  {/each}
</div>

<style>
  .album-artwork-mosaic { display:grid; width:100%; height:100%; min-width:0; min-height:0; aspect-ratio:1; overflow:hidden; grid-template-columns:repeat(2,minmax(0,1fr)); grid-template-rows:repeat(2,minmax(0,1fr)); background:#000; }
  .album-artwork-mosaic img { display:block; width:100%; height:100%; min-width:0; min-height:0; object-fit:cover; }
  .album-artwork-mosaic.cover-count-1 img { grid-column:1 / -1; grid-row:1 / -1; }
  .album-artwork-mosaic.cover-count-2 { grid-template-columns:repeat(2,minmax(0,1fr)); grid-template-rows:minmax(0,1fr); }
  .album-artwork-mosaic.cover-count-3 { grid-template-columns:repeat(3,minmax(0,1fr)); grid-template-rows:minmax(0,1fr); }
  .album-artwork-mosaic.cover-count-2 img,
  .album-artwork-mosaic.cover-count-3 img { grid-row:1; }
  .album-artwork-mosaic.cover-count-2 img:nth-child(1),
  .album-artwork-mosaic.cover-count-3 img:nth-child(1) { grid-column:1; }
  .album-artwork-mosaic.cover-count-2 img:nth-child(2) { grid-column:2; }
  .album-artwork-mosaic.cover-count-3 img:nth-child(2) { grid-column:2; }
  .album-artwork-mosaic.cover-count-3 img:nth-child(3) { grid-column:3; }
</style>
