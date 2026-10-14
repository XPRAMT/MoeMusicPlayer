<script lang="ts">
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
  const labels = ['#', 'A', 'B', 'C', 'ㄅ'];
  const played: Array<{ id: string; index: number }> = [];

  function itemAt(index: number): Entry | null {
    return entries[index] ?? null;
  }

  function getKey(entry: Entry): string {
    return entry.id;
  }

  function onRange(): void {}

  function onPlay(entry: Entry, index: number): void {
    played.push({ id: entry.id, index });
  }

  declare global {
    interface Window {
      quickIndexHarness: {
        played: Array<{ id: string; index: number }>;
      };
    }
  }

  if (typeof window !== 'undefined') window.quickIndexHarness = { played };
</script>

<main class="page-content" style="height: 420px; padding: 20px;">
  {#snippet renderRow(row: VirtualListRow<Entry>)}
    <div class="harness-row">{row.item?.title ?? 'Loading'}</div>
  {/snippet}

  <PagedVirtualList
    totalCount={entries.length}
    revision={0}
    generation={0}
    itemAt={itemAt}
    getKey={getKey}
    rowHeight={40}
    headerHeight={37}
    listId="quick-index-harness"
    ariaLabel="首字索引測試歌曲"
    columnCount={1}
    gridTemplate="minmax(0,1fr)"
    listName="首字索引測試"
    onRange={onRange}
    onPlay={onPlay}
    groupRows={groups}
    quickIndexLabels={labels}
    row={renderRow}
  />
</main>

<style>
  .harness-row {
    display: flex;
    height: 100%;
    align-items: center;
    padding: 0 12px;
  }
</style>
