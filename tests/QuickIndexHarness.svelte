<script lang="ts">
  import { tick } from 'svelte';
  import PagedVirtualList from '../src/lib/PagedVirtualList.svelte';
  import type { QuickIndexGroup, VirtualListRow } from '../src/lib/PagedVirtualList.svelte';

  interface Entry {
    id: string;
    title: string;
  }

  const entries: Entry[] = Array.from({ length: 80 }, (_, index) => ({
    id: `entry-${index}`,
    title: `Track ${index + 1}`,
  }));
  const groups: QuickIndexGroup[] = [
    { label: 'A', beforeIndex: 0 },
    { label: 'B', beforeIndex: 40 },
  ];
  const labels = ['#', 'A', 'B', 'C', 'ㄅ', ...Array.from({ length: 30 }, (_, index) => `測${index + 1}`)];
  const played: Array<{ id: string; index: number }> = [];
  const requestedRanges: Array<{ start: number; end: number }> = [];
  let showList = $state(true);
  let sortKind = $state<'title' | 'date'>('title');

  function itemAt(index: number): Entry | null {
    return entries[index] ?? null;
  }

  function getKey(entry: Entry): string {
    return entry.id;
  }

  function onRange(range: { start: number; end: number }): void {
    requestedRanges.push(range);
  }

  function onPlay(entry: Entry, index: number): void {
    played.push({ id: entry.id, index });
  }

  declare global {
    interface Window {
      quickIndexHarness: {
        played: Array<{ id: string; index: number }>;
        requestedRanges: Array<{ start: number; end: number }>;
        navigateAwayAndBack: () => Promise<void>;
        setDateSort: () => void;
      };
    }
  }

  if (typeof window !== 'undefined') window.quickIndexHarness = {
    played,
    requestedRanges,
    navigateAwayAndBack: async () => {
      showList = false;
      await tick();
      showList = true;
      await tick();
    },
    setDateSort: () => { sortKind = 'date'; },
  };
</script>

<main class="page-content" style="height: 420px; padding: 20px;">
  <button type="button" data-testid="date-sort" onclick={() => { sortKind = 'date'; }}>日期／數字排序</button>
  <button type="button" data-testid="navigate-away-back" onclick={() => void window.quickIndexHarness.navigateAwayAndBack()}>切換側欄再返回</button>
  {#snippet renderRow(row: VirtualListRow<Entry>)}
    <div class="harness-row">{row.item?.title ?? 'Loading'}</div>
  {/snippet}

  {#if showList}<PagedVirtualList
    totalCount={entries.length}
    revision={0}
    generation={0}
    itemAt={itemAt}
    getKey={getKey}
    rowHeight={40}
    headerHeight={37}
    listId="numeric-date-sort-harness"
    ariaLabel="首字索引測試歌曲"
    columnCount={1}
    gridTemplate="minmax(0,1fr)"
    listName="首字索引測試"
    onRange={onRange}
    onPlay={onPlay}
    groupRows={sortKind === 'title' ? groups : []}
    quickIndexLabels={labels}
    row={renderRow}
  />{/if}
</main>

<style>
  .harness-row {
    display: flex;
    height: 100%;
    align-items: center;
    padding: 0 12px;
  }
</style>
