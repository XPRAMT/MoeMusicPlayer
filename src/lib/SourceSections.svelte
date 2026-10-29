<script lang="ts">
  // The three blocks of a music-source page — folders, playlists, excluded folders — in one layout for every kind of
  // source (this device or a remote server). The pages only supply the lists and what the buttons do.
  import { tr } from './i18n.js';
  import { appTooltip } from './app-tooltip.js';
  import { displayPlaylistName } from './unknown-label.js';

  export type SourceRow = {
    id: string;
    displayName: string;
    location: string;
    enabled?: boolean;
  };

  let {
    folders,
    playlists,
    excluded,
    loading = false,
    showExcluded = true,
    disabled = false,
    pickFolderDisabled = false,
    pickPlaylistDisabled = false,
    pickExcludedDisabled = false,
    playlistTooltip = null,
    onPickFolder,
    onPickPlaylist,
    onPickExcluded,
    onToggle,
    onRemove,
    onRemoveExcluded,
  }: {
    folders: SourceRow[];
    playlists: SourceRow[];
    excluded: SourceRow[];
    loading?: boolean;
    showExcluded?: boolean;
    /** Disables the checkboxes and remove buttons. */
    disabled?: boolean;
    pickFolderDisabled?: boolean;
    pickPlaylistDisabled?: boolean;
    pickExcludedDisabled?: boolean;
    playlistTooltip?: string | null;
    onPickFolder: () => void;
    onPickPlaylist: () => void;
    onPickExcluded: () => void;
    onToggle: (row: SourceRow, enabled: boolean) => void;
    onRemove: (row: SourceRow) => void;
    onRemoveExcluded: (row: SourceRow) => void;
  } = $props();
</script>

{#snippet sourceRows(rows: SourceRow[], removable: 'source' | 'excluded')}
  {#each rows as row (row.id)}
    <div class="configured-source">
      <div class="configured-source-copy"><strong>{displayPlaylistName(row.displayName)}</strong><small class="configured-source-location" use:appTooltip={row.location}>{row.location}</small></div>
      <div class="configured-source-actions">
        {#if removable === 'source'}
          <label><input type="checkbox" checked={row.enabled} {disabled} onchange={(event) => onToggle(row, event.currentTarget.checked)} />{tr("啟用")}</label>
          <button class="text-button" type="button" {disabled} onclick={() => onRemove(row)}>{tr("移除")}</button>
        {:else}
          <button class="text-button" type="button" {disabled} onclick={() => onRemoveExcluded(row)}>{tr("移除")}</button>
        {/if}
      </div>
    </div>
  {/each}
{/snippet}

<div class="source-action-row source-action-buttons source-folder-picker">
  <button class="soft-button" type="button" disabled={pickFolderDisabled} onclick={onPickFolder}>{tr("選擇資料夾")}</button>
</div>
<div class="configured-sources" aria-live="polite">
  <div class="configured-sources-heading"><strong>{tr("資料夾")}</strong><span>{folders.length}</span></div>
  {#if loading && folders.length === 0}
    <p class="source-muted-note">{tr("正在讀取來源…")}</p>
  {:else if folders.length === 0}
    <p class="source-muted-note">{tr("尚未加入資料夾。加入後會自動開始同步。")}</p>
  {:else}
    {@render sourceRows(folders, 'source')}
  {/if}
</div>

<div class="source-action-row source-action-buttons source-folder-picker">
  <button class="soft-button" type="button" disabled={pickPlaylistDisabled} onclick={onPickPlaylist} use:appTooltip={playlistTooltip ?? ''}>{tr("選擇播放清單")}</button>
</div>
<div class="configured-sources" aria-live="polite">
  <div class="configured-sources-heading"><strong>{tr("播放清單")}</strong><span>{playlists.length}</span></div>
  {#if playlists.length === 0}
    <p class="source-muted-note">{tr("尚未加入播放清單。可選擇 .m3u 或 .m3u8 檔案，清單內的歌曲會加入曲庫。")}</p>
  {:else}
    {@render sourceRows(playlists, 'source')}
  {/if}
</div>

{#if showExcluded}
  <div class="source-action-row source-action-buttons source-folder-picker excluded-folder-picker">
    <button class="soft-button" type="button" disabled={pickExcludedDisabled} onclick={onPickExcluded}>{tr("選擇排除資料夾")}</button>
  </div>
  <div class="configured-sources excluded-folders" aria-live="polite">
    <div class="configured-sources-heading"><strong>{tr("已排除的來源")}</strong><span>{excluded.length}</span></div>
    {#if loading && excluded.length === 0}
      <p class="source-muted-note">{tr("正在讀取排除清單…")}</p>
    {:else if excluded.length === 0}
      <p class="source-muted-note">{tr("尚未排除資料夾。排除後，同步會略過該資料夾內的曲目。")}</p>
    {:else}
      {@render sourceRows(excluded, 'excluded')}
    {/if}
  </div>
{/if}
