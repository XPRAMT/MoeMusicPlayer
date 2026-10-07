<script lang="ts">
  import MiddleEllipsis from './MiddleEllipsis.svelte';

  interface Column {
    id: string;
    label: string;
  }

  interface Cell {
    id: string;
    text: string;
    title?: string;
  }

  interface Props {
    variant: 'header' | 'item' | 'placeholder';
    columns: Column[];
    indexLabel?: string;
    cells?: Cell[];
    placeholderTitle?: string;
  }

  let {
    variant,
    columns,
    indexLabel = '',
    cells = [],
    placeholderTitle = '正在載入…',
  }: Props = $props();
</script>

{#if variant === 'header'}
  <span class="column-index" role="columnheader">#</span>
  {#each columns as column (column.id)}
    <span class={`list-column list-column-${column.id}`} role="columnheader">{column.label}</span>
  {/each}
{:else if variant === 'item'}
  <span class="track-index column-index" role="gridcell">{indexLabel}</span>
  {#each cells as cell (cell.id)}
    <span class={`list-column list-column-${cell.id}`} role="gridcell" title={cell.title || cell.text || undefined}>
      {#if cell.id === 'title' || cell.id === 'artist' || cell.id === 'album' || cell.id === 'audioFormat'}
        <MiddleEllipsis text={cell.text} />
      {:else}
        {cell.text}
      {/if}
    </span>
  {/each}
{:else}
  <span class="track-index column-index" role="gridcell" aria-hidden="true">—</span>
  {#each columns as column (column.id)}
    <span class={`list-column list-column-${column.id}`} role="gridcell" aria-hidden="true">
      {column.id === 'title' ? placeholderTitle : '—'}
    </span>
  {/each}
{/if}
