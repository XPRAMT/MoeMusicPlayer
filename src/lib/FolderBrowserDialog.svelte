<script lang="ts">
  // The in-app folder browser for Android (the system picker refuses a storage root even with All files access).
  // The backend asked for it with `android-folder-picker-request`; the answer goes back through `onFinish`.
  import { invokeCommand, getErrorText, type DirectoryListing } from './ipc';
  import { tr } from './i18n.js';

  let { open, style = '', onFinish }: {
    open: boolean;
    style?: string;
    onFinish: (path: string | null) => void;
  } = $props();

  let dialog = $state<HTMLDialogElement>();
  let listing = $state<DirectoryListing | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let requestSerial = 0;

  async function load(path: string | null): Promise<void> {
    const serial = ++requestSerial;
    loading = true;
    error = null;
    try {
      const result = await invokeCommand('android_list_directories', { path });
      if (serial === requestSerial) listing = result;
    } catch (cause) {
      if (serial === requestSerial) error = getErrorText(cause);
    } finally {
      if (serial === requestSerial) loading = false;
    }
  }

  $effect(() => {
    const element = dialog;
    if (!element) return;
    if (open && !element.open) {
      element.showModal();
      listing = null;
      void load(null);
    } else if (!open && element.open) {
      element.close();
    }
  });
</script>

<dialog
  bind:this={dialog}
  class="update-dialog folder-browser floating-surface"
  {style}
  aria-labelledby="folder-browser-title"
  oncancel={(event) => { event.preventDefault(); onFinish(null); }}
>
  <div class="update-dialog-content">
    <h2 id="folder-browser-title">{tr("選擇資料夾")}</h2>
    {#if listing && listing.roots.length > 1}
      <div class="folder-browser-roots" role="group" aria-label={tr("儲存空間")}>
        {#each listing.roots as root (root.path)}
          <button class="soft-button" type="button" onclick={() => void load(root.path)}>{root.label === '內部儲存空間' ? tr('內部儲存空間') : root.label}</button>
        {/each}
      </div>
    {/if}
    <div class="folder-browser-path" title={listing?.path ?? ''}>{listing?.path ?? '…'}</div>
    {#if error}<p class="error-text" role="alert">{error}</p>{/if}
    <div class="folder-browser-list" aria-busy={loading}>
      {#if listing?.parent}
        <button class="folder-browser-entry" type="button" onclick={() => void load(listing?.parent ?? null)}>
          <span aria-hidden="true">⬆</span><span>{tr("上一層")}</span>
        </button>
      {/if}
      {#each listing?.entries ?? [] as entry (entry.path)}
        <button class="folder-browser-entry" type="button" onclick={() => void load(entry.path)}>
          <span aria-hidden="true">📁</span><span>{entry.name}</span>
        </button>
      {/each}
      {#if listing && listing.entries.length === 0 && !loading}
        <p class="folder-browser-empty">{tr("這個資料夾沒有子資料夾")}</p>
      {/if}
    </div>
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" onclick={() => onFinish(null)}>{tr("取消")}</button>
      <button class="soft-button" type="button" disabled={!listing} onclick={() => listing && onFinish(listing.path)}>{tr("使用這個資料夾")}</button>
    </div>
  </div>
</dialog>
