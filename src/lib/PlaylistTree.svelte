<script lang="ts">
  import { IconPlaylist } from '@tabler/icons-svelte-runes';
  import type { PlaylistSummary } from './ipc';
  import { appTooltip } from './app-tooltip.js';

  interface Props {
    playlists: PlaylistSummary[];
    selectedPlaylistId: string | null;
    active: boolean;
    onOpen: () => void;
    onSelect: (playlistId: string) => void;
  }

  let { playlists, selectedPlaylistId, active, onOpen, onSelect }: Props = $props();
  function choosePlaylist(playlistId: string): void {
    onSelect(playlistId);
  }
</script>

<nav class="playlist-tree" aria-label="播放清單">
  <div class="playlist-tree-heading" class:active>
    <button class="playlist-tree-open" class:active type="button" aria-current={active ? 'page' : undefined} onclick={onOpen}>
      <IconPlaylist size={17} stroke={1.7} aria-hidden="true" />
      <span class="playlist-tree-open-label">播放清單</span>
      <span class="playlist-tree-count" aria-label={`${playlists.length} 份播放清單`}>{playlists.length.toLocaleString()}</span>
    </button>
  </div>

  <ul id="sidebar-playlist-items" class="playlist-tree-items">
    {#each playlists as playlist (playlist.id)}
      <li>
        <button
          class="playlist-tree-item"
          class:selected={active && selectedPlaylistId === playlist.id}
          type="button"
          aria-current={active && selectedPlaylistId === playlist.id ? 'page' : undefined}
          use:appTooltip={playlist.name.trim() || '未命名播放清單'}
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
</nav>

<style>
  .playlist-tree {
    display: flex;
    width: 100%;
    min-width: 0;
    flex-direction: column;
    padding: 6px;
    border: 1px solid var(--line);
    border-radius: 12px;
    background: transparent;
  }

  .playlist-tree-heading {
    display: grid;
    width: 100%;
    min-height: 39px;
    flex: 0 0 auto;
    grid-template-columns: minmax(0, 1fr);
    align-items: center;
    border-radius: 8px;
  }
  .playlist-tree-heading.active .playlist-tree-open,
  .playlist-tree-heading.active .playlist-tree-open:hover,
  .playlist-tree-heading.active .playlist-tree-open:focus-visible {
    background: transparent;
  }
  .playlist-tree-open {
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
    font-size: var(--font-caption);
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
  .playlist-tree-open:hover,
  .playlist-tree-open:focus-visible {
    background: rgba(var(--text-rgb), 0.05);
  }
  .playlist-tree-open:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .playlist-tree-count {
    margin-left: auto;
    color: var(--text-soft);
    font-family: 'DM Mono', monospace;
    font-size: var(--font-caption);
    font-weight: 500;
  }

  .playlist-tree-items {
    display: grid;
    min-width: 0;
    gap: 3px;
    margin: 4px 0 0;
    padding: 5px 0 2px 12px;
    border-top: var(--divider-width) solid var(--divider-color);
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
    position: relative;
    isolation: isolate;
  }

  .playlist-tree-item:hover,
  .playlist-tree-item:focus-visible,
  .playlist-tree-item.selected {
    color: var(--text);
  }

  .playlist-tree-item::before {
    content: '';
    position: absolute;
    z-index: 0;
    inset: 0;
    border-radius: inherit;
    background: rgb(var(--text-rgb));
    opacity: 0;
    pointer-events: none;
  }

  .playlist-tree-item > * {
    position: relative;
    z-index: 1;
  }

  .playlist-tree-item:hover::before,
  .playlist-tree-item:focus-visible::before {
    opacity: 0.05;
  }

  .playlist-tree-item.selected::before,
  .playlist-tree-item.selected:hover::before,
  .playlist-tree-item.selected:focus-visible::before {
    opacity: 0.1;
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
    font-size: var(--font-caption);
    font-weight: 550;
  }

  .playlist-tree-copy small {
    color: var(--text-soft);
    font-size: var(--font-caption);
  }
</style>
