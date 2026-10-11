<script lang="ts">
  import MiddleEllipsis from './MiddleEllipsis.svelte';
  import hiResBadgeUrl from '../assets/hi-res-badge.svg';
  import { RESIZABLE_TRACK_COLUMN_IDS } from './track-columns.js';

  interface Column {
    id: string;
    label: string;
  }

  interface Cell {
    id: string;
    text: string;
    title?: string;
    hiRes?: boolean;
  }

  interface SortState {
    field: string;
    direction: 'asc' | 'desc';
  }

  interface Props {
    variant: 'header' | 'item' | 'placeholder';
    columns: Column[];
    indexLabel?: string;
    cells?: Cell[];
    placeholderTitle?: string;
    sortable?: boolean;
    sort?: SortState | null;
    onSort?: (columnId: string) => void;
    resizable?: boolean;
    onResize?: (columnId: string, widthPx: number) => void;
  }

  let {
    variant,
    columns,
    indexLabel = '',
    cells = [],
    placeholderTitle = '正在載入…',
    sortable = false,
    sort = null,
    onSort,
    resizable = false,
    onResize,
  }: Props = $props();

  let resizeSession: {
    id: string;
    startX: number;
    startWidth: number;
  } | null = null;

  function ariaSortFor(columnId: string): 'ascending' | 'descending' | 'none' | undefined {
    if (!sortable) return undefined;
    if (sort?.field !== columnId) return 'none';
    return sort.direction === 'desc' ? 'descending' : 'ascending';
  }

  function handleSortClick(columnId: string): void {
    if (!sortable || !onSort) return;
    onSort(columnId);
  }

  function handleResizePointerDown(event: PointerEvent, columnId: string): void {
    if (!resizable || !RESIZABLE_TRACK_COLUMN_IDS.has(columnId as never) || !onResize) return;
    if (event.button !== 0) return;
    const header = (event.currentTarget as HTMLElement).closest('.list-column');
    if (!(header instanceof HTMLElement)) return;
    event.preventDefault();
    event.stopPropagation();
    resizeSession = {
      id: columnId,
      startX: event.clientX,
      startWidth: header.getBoundingClientRect().width,
    };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function handleResizePointerMove(event: PointerEvent): void {
    if (!resizeSession || !onResize) return;
    const delta = event.clientX - resizeSession.startX;
    onResize(resizeSession.id, resizeSession.startWidth + delta);
  }

  function handleResizePointerUp(event: PointerEvent): void {
    if (!resizeSession) return;
    try {
      (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    } catch {
      // Pointer may already be released.
    }
    resizeSession = null;
  }
</script>

{#if variant === 'header'}
  <span class="column-index" role="columnheader">#</span>
  {#each columns as column (column.id)}
    <span
      class={`list-column list-column-${column.id}`}
      class:is-sortable={sortable}
      class:is-resizable={resizable && RESIZABLE_TRACK_COLUMN_IDS.has(column.id as never)}
      role="columnheader"
      aria-sort={ariaSortFor(column.id)}
    >
      {#if sortable}
        <button
          type="button"
          class="column-sort-button"
          onclick={() => handleSortClick(column.id)}
          title={`依${column.label}排序`}
        >
          <span class="column-sort-label">{column.label}</span>
          {#if sort?.field === column.id}
            <span class="column-sort-indicator" aria-hidden="true">{sort.direction === 'asc' ? '▲' : '▼'}</span>
          {/if}
        </button>
      {:else}
        <span class="column-sort-label">{column.label}</span>
      {/if}
      {#if resizable && RESIZABLE_TRACK_COLUMN_IDS.has(column.id as never)}
        <span
          class="column-resize-handle"
          role="separator"
          aria-orientation="vertical"
          aria-label={`調整${column.label}欄寬`}
          onpointerdown={(event) => handleResizePointerDown(event, column.id)}
          onpointermove={handleResizePointerMove}
          onpointerup={handleResizePointerUp}
          onpointercancel={handleResizePointerUp}
        ></span>
      {/if}
    </span>
  {/each}
{:else if variant === 'item'}
  <span class="track-index column-index" role="gridcell">{indexLabel}</span>
  {#each cells as cell (cell.id)}
    <span class={`list-column list-column-${cell.id}`} role="gridcell" title={cell.title || cell.text || undefined}>
      {#if cell.id === 'audioFormat'}
        <span class="list-audio-format">
          <MiddleEllipsis text={cell.text} />
          {#if cell.hiRes}
            <img class="list-hi-res-badge" src={hiResBadgeUrl} alt="Hi-Res" title="Hi-Res" />
          {/if}
        </span>
      {:else if cell.id === 'title' || cell.id === 'artist' || cell.id === 'album'}
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

<style>
  .list-column.is-sortable,
  .list-column.is-resizable {
    position: relative;
    display: flex;
    min-width: 0;
    align-items: center;
  }

  .column-sort-button {
    display: inline-flex;
    min-width: 0;
    max-width: 100%;
    align-items: center;
    gap: 4px;
    margin: 0;
    padding: 0;
    border: 0;
    color: inherit;
    background: transparent;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }

  .column-sort-button:hover .column-sort-label,
  .column-sort-button:focus-visible .column-sort-label {
    color: var(--accent-text);
  }

  .column-sort-button:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: 4px;
  }

  .column-sort-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .column-sort-indicator {
    flex: 0 0 auto;
    font-size: 9px;
    line-height: 1;
    opacity: 0.85;
  }

  .column-resize-handle {
    position: absolute;
    top: 0;
    right: -5px;
    z-index: 2;
    width: 10px;
    height: 100%;
    cursor: col-resize;
    touch-action: none;
  }

  .column-resize-handle::after {
    content: '';
    position: absolute;
    top: 20%;
    bottom: 20%;
    left: 4px;
    width: 2px;
    border-radius: 1px;
    background: transparent;
  }

  .list-column.is-resizable:hover .column-resize-handle::after,
  .column-resize-handle:hover::after {
    background: rgba(var(--text-rgb), 0.35);
  }

  .list-audio-format {
    display: inline-flex;
    min-width: 0;
    max-width: 100%;
    align-items: center;
    gap: 6px;
  }

  .list-audio-format :global(.middle-ellipsis) {
    min-width: 0;
    flex: 1 1 auto;
  }

  .list-hi-res-badge {
    display: block;
    width: 20.4px;
    height: 12.2px;
    flex: 0 0 auto;
    align-self: center;
    object-fit: contain;
    transform: translateY(1px);
  }
</style>
