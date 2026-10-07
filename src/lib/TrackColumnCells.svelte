<script lang="ts">
  import type { Snippet } from 'svelte';

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
    trailingLabel?: string;
    trailing?: Snippet;
  }

  let {
    variant,
    columns,
    indexLabel = '',
    cells = [],
    placeholderTitle = '正在載入…',
    trailingLabel = '',
    trailing,
  }: Props = $props();
</script>

{#if variant === 'header'}
  <span class="column-index" role="columnheader">#</span>
  {#each columns as column (column.id)}
    <span class={`list-column list-column-${column.id}`} role="columnheader">{column.label}</span>
  {/each}
  <span class="column-action" role="columnheader" aria-label={trailingLabel}></span>
{:else if variant === 'item'}
  <span class="track-index column-index" role="gridcell">{indexLabel}</span>
  {#each cells as cell (cell.id)}
    <span class={`list-column list-column-${cell.id}`} role="gridcell" title={cell.title}>{cell.text}</span>
  {/each}
  {#if trailing}
    {@render trailing()}
  {:else}
    <span class="column-action" role="gridcell"></span>
  {/if}
{:else}
  <span class="track-index column-index" role="gridcell" aria-hidden="true">—</span>
  {#each columns as column (column.id)}
    <span class={`list-column list-column-${column.id}`} role="gridcell" aria-hidden="true">
      {column.id === 'title' ? placeholderTitle : '—'}
    </span>
  {/each}
  <span class="column-action" role="gridcell" aria-hidden="true"></span>
{/if}
