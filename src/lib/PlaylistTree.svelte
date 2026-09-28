<script lang="ts">
  import { onMount } from 'svelte';
  import { IconChevronRight, IconPlaylist } from '@tabler/icons-svelte-runes';
  import type { PlaylistSummary } from './ipc';

  interface Props {
    playlists: PlaylistSummary[];
    selectedPlaylistId: string | null;
    active: boolean;
    onOpen: () => void;
    onSelect: (playlistId: string) => void;
  }

  let { playlists, selectedPlaylistId, active, onOpen, onSelect }: Props = $props();
  let expanded = $state(true);

  onMount(() => {
    const compactLayout = window.matchMedia('(max-width: 820px)');
    if (compactLayout.matches) expanded = false;
    const collapseWhenCompact = (event: MediaQueryListEvent) => {
      if (event.matches) expanded = false;
    };
    compactLayout.addEventListener('change', collapseWhenCompact);
    return () => compactLayout.removeEventListener('change', collapseWhenCompact);
  });

  function choosePlaylist(playlistId: string): void {
    onSelect(playlistId);
    if (window.matchMedia('(max-width: 820px)').matches) expanded = false;
  }
</script>

<nav class="playlist-tree" aria-label="播放清單">
  <div class="playlist-tree-heading" class:active>
    <button class="playlist-tree-open" class:active type="button" aria-current={active ? 'page' : undefined} onclick={onOpen}>
      <IconPlaylist size={17} stroke={1.7} aria-hidden="true" />
      <span class="playlist-tree-open-label">播放清單</span>
      <span class="playlist-tree-count" aria-label={`${playlists.length} 份播放清單`}>{playlists.length.toLocaleString()}</span>
    </button>
    <button
      class="playlist-tree-toggle"
      type="button"
      aria-label={expanded ? '收合播放清單' : '展開播放清單'}
      aria-expanded={expanded}
      aria-controls="sidebar-playlist-items"
      onclick={() => { expanded = !expanded; }}
    >
      <IconChevronRight class={expanded ? 'playlist-tree-chevron expanded' : 'playlist-tree-chevron'} size={15} stroke={1.8} aria-hidden="true" />
    </button>
  </div>

  {#if expanded}
    <ul id="sidebar-playlist-items" class="playlist-tree-items">
      {#each playlists as playlist (playlist.id)}
        <li>
          <button
            class="playlist-tree-item"
            class:selected={selectedPlaylistId === playlist.id}
            type="button"
            aria-current={selectedPlaylistId === playlist.id ? 'page' : undefined}
            title={playlist.name.trim() || '未命名播放清單'}
            onclick={() => choosePlaylist(playlist.id)}
          >
            <IconPlaylist size={17} stroke={1.6} aria-hidden="true" />
            <span class="playlist-tree-copy">
              <strong>{playlist.name.trim() || '未命名播放清單'}</strong>
              <small>{playlist.entryCount.toLocaleString()} 個項目</small>
            </span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</nav>

<style>
  .playlist-tree {
    display: flex;
    width: 100%;
    min-width: 0;
    max-height: min(380px, 48vh, calc(100dvh - 230px));
    flex-direction: column;
    overflow: auto;
    padding: 6px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: rgba(var(--text-rgb), 0.014);
  }

  .playlist-tree-heading {
    display: grid;
    width: 100%;
    min-height: 39px;
    flex: 0 0 auto;
    grid-template-columns: minmax(0, 1fr) 29px;
    align-items: center;
    border-radius: 8px;
  }

  .playlist-tree-heading.active {
    background: rgba(var(--accent-rgb), 0.07);
  }

  .playlist-tree-open,
  .playlist-tree-toggle {
    display: flex;
    min-width: 0;
    min-height: 35px;
    align-items: center;
    justify-content: center;
    border: 0;
    border-radius: 7px;
    color: var(--text);
    background: transparent;
    font: inherit;
    font-size: 11px;
    font-weight: 650;
    cursor: pointer;
  }

  .playlist-tree-open {
    justify-content: flex-start;
    gap: 8px;
    padding: 0 9px;
    text-align: left;
  }

  .playlist-tree-open > :global(svg) {
    flex: 0 0 auto;
    color: var(--accent-text);
  }

  .playlist-tree-open-label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .playlist-tree-open.active {
    color: var(--accent-text);
  }

  .playlist-tree-toggle {
    width: 29px;
    padding: 0;
  }

  .playlist-tree-open:hover,
  .playlist-tree-open:focus-visible,
  .playlist-tree-toggle:hover,
  .playlist-tree-toggle:focus-visible {
    background: rgba(var(--text-rgb), 0.045);
  }

  .playlist-tree-open:focus-visible,
  .playlist-tree-toggle:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  :global(.playlist-tree-chevron) {
    color: var(--text-soft);
    transition: transform 120ms ease;
  }

  :global(.playlist-tree-chevron.expanded) {
    transform: rotate(90deg);
  }

  .playlist-tree-count {
    margin-left: auto;
    color: var(--text-soft);
    font-family: 'DM Mono', monospace;
    font-size: 9px;
    font-weight: 500;
  }

  .playlist-tree-toggle :global(svg) {
    color: var(--text-soft);
  }

  .playlist-tree-items {
    display: grid;
    min-width: 0;
    gap: 3px;
    margin: 4px 0 0;
    padding: 5px 0 2px 12px;
    border-top: 1px solid var(--line);
    list-style: none;
  }

  .playlist-tree-items li {
    min-width: 0;
  }

  .playlist-tree-item {
    display: grid;
    width: 100%;
    min-width: 0;
    min-height: 45px;
    grid-template-columns: 17px minmax(0, 1fr);
    align-items: center;
    gap: 9px;
    padding: 6px 9px;
    border: 1px solid transparent;
    border-radius: 8px;
    color: var(--text-soft);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }

  .playlist-tree-item:hover,
  .playlist-tree-item:focus-visible {
    color: var(--text);
    background: rgba(var(--text-rgb), 0.045);
  }

  .playlist-tree-item.selected {
    border-color: rgba(var(--accent-rgb), 0.28);
    color: var(--accent-text);
    background: rgba(var(--accent-rgb), 0.09);
  }

  .playlist-tree-item > :global(svg) {
    width: 17px;
    height: 17px;
  }

  .playlist-tree-copy {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
  }

  .playlist-tree-copy strong,
  .playlist-tree-copy small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .playlist-tree-copy strong {
    color: inherit;
    font-size: 10px;
    font-weight: 550;
  }

  .playlist-tree-copy small {
    color: var(--text-soft);
    font-size: 8px;
  }

  @media (max-width: 820px) {
    .playlist-tree-heading {
      grid-template-columns: minmax(0, 1fr) 22px;
    }

    .playlist-tree-open-label,
    .playlist-tree-count {
      display: none;
    }

    .playlist-tree-open {
      min-height: 38px;
      justify-content: center;
      padding: 0;
    }

    .playlist-tree-toggle {
      width: 22px;
      min-height: 38px;
    }
  }

  @media (max-width: 620px) {
    .playlist-tree-heading {
      grid-template-columns: 38px 18px;
    }
  }

  @media (max-width: 620px) {
    .playlist-tree {
      max-height: min(200px, 28vh);
    }
  }
</style>
