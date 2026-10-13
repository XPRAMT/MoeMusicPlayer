<script lang="ts">
  import AlbumIndex from '../src/lib/AlbumIndex.svelte';

  let count = $state<number | null>(null);
</script>

<main class="album-test-workspace" data-testid="workspace">
  <header>專輯</header>
  <AlbumIndex
    refreshKey={0}
    active={false}
    onCountChange={(value) => (count = value)}
    onRowsChange={() => {}}
    onSelect={() => {}}
  />
  <output data-testid="album-count">{count ?? ''} 張專輯</output>
</main>

<style>
  .album-test-workspace { display:flex; width:640px; height:560px; flex-direction:column; }
  .album-test-workspace header { height:48px; flex:0 0 48px; }
</style>
