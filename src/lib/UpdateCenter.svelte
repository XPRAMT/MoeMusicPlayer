<script lang="ts">
  import { tr } from './i18n.js';
  import { tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import { invokeCommand, type UpdateInstallMode, type UpdateState } from './ipc';
  import { createUpdateController, EMPTY_UPDATE_STATE, formatBuildTime, UPDATE_DOWNLOADABLE_STATUSES, updateProgressPercent, updateStatusText } from './update-controller';

  let {
    enabled,
    platform = 'windows',
    updateState = $bindable<UpdateState>({ ...EMPTY_UPDATE_STATE }),
    surfaceOpacityPercent = 70,
    surfaceBlurPx = 12,
  }: {
    enabled: boolean;
    /** Android: one 「下載並安裝」 action (system installer); Windows: restart now / next launch. */
    platform?: 'windows' | 'android' | 'unsupported';
    updateState?: UpdateState;
    surfaceOpacityPercent?: number;
    surfaceBlurPx?: number;
  } = $props();
  let visible = $state(false);
  let dialog = $state<HTMLDialogElement>();
  let ignoreButton = $state<HTMLButtonElement>();
  let returnFocus: HTMLElement | null = null;
  let started = false;
  const progress = $derived(updateProgressPercent(updateState));
  const canDownload = $derived(Boolean(updateState.latest) && UPDATE_DOWNLOADABLE_STATUSES.includes(updateState.status));
  const controller = createUpdateController({
    subscribe: (receive) => listen<UpdateState>('update-state-changed', (event) => receive(event.payload)),
    getState: () => invokeCommand('update_get_state', {}),
    check: (manual) => invokeCommand('update_check', { manual }),
    ignore: () => invokeCommand('update_ignore', {}),
    download: (mode) => invokeCommand('update_download', { mode }),
    onChange: (next, show) => { updateState = next; visible = show; },
  });
  /** Settings → About: a version the user ignored is offered again. */
  export function check() { void controller.check(true); }
  export function show() { controller.show(); }
  function download(mode: UpdateInstallMode) { void controller.download(mode); }
  function dismiss(event?: Event) { event?.preventDefault(); void controller.ignore(); }
  function handleDialogKeydown(event: KeyboardEvent) {
    if (event.key !== 'Tab' || !dialog) return;
    const buttons = Array.from(dialog.querySelectorAll<HTMLButtonElement>('button:not([disabled])'));
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    if (!first || !last) return;
    if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  }

  $effect(() => {
    if (enabled && isTauri() && !started) { started = true; void controller.start(); }
  });
  $effect(() => () => controller.dispose());
  $effect(() => {
    if (!dialog) return;
    if (visible && !dialog.open) {
      returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      dialog.showModal();
      void tick().then(() => ignoreButton?.focus());
    } else if (!visible && dialog.open) {
      dialog.close();
      if (returnFocus?.isConnected) returnFocus.focus();
    }
  });
</script>

<dialog
  bind:this={dialog}
  class="update-dialog floating-surface"
  style={`--quick-settings-opacity:${surfaceOpacityPercent}%;--quick-settings-blur:${surfaceBlurPx}px`}
  aria-labelledby="update-title"
  aria-describedby="update-description"
  oncancel={dismiss}
  onkeydown={handleDialogKeydown}
>
  <div class="update-dialog-content">
    <h2 id="update-title">{tr("發現新版本")}{updateState.latest ? ` ${updateState.latest.version}` : ''}</h2>
    {#if platform === 'android'}
      <p id="update-description">{tr("下載期間可繼續播放；APK 下載並通過 SHA-256 驗證後交給系統安裝程式，由你確認安裝。")}</p>
    {:else}
      <p id="update-description">{tr("選擇更新方式。下載期間可繼續播放；下載並驗證成功後，會把目前的程式檔改名並換上新版。")}</p>
    {/if}
    {#if updateState.latest}
      <p>{tr("發布時間：")}{formatBuildTime(updateState.latest.publishedAt)}</p>
      {#if updateState.latest.notes}<div class="update-release-notes">{updateState.latest.notes}</div>{/if}
    {/if}
    <p class:error-text={updateState.status === 'error'} role="status" aria-live="polite">{updateStatusText(updateState)}</p>
    {#if updateState.status === 'downloading'}
      <progress aria-label={tr("更新下載進度")} max="100" value={progress ?? undefined}></progress>
      <p>{progress == null ? tr('正在下載') : `${progress}%`} · {(updateState.downloadedBytes / 1048576).toFixed(1)} MiB{updateState.totalBytes ? ` / ${(updateState.totalBytes / 1048576).toFixed(1)} MiB` : ''}</p>
    {/if}
    <div class="update-dialog-actions">
      <button bind:this={ignoreButton} class="soft-button" type="button" onclick={() => dismiss()}>{tr("忽略")}</button>
      {#if platform === 'android'}
        <button class="soft-button" type="button" disabled={!canDownload} onclick={() => download('restartNow')}>{tr("下載並安裝")}</button>
      {:else}
        <button class="soft-button" type="button" disabled={!canDownload} onclick={() => download('nextLaunch')}>{tr("自動安裝")}</button>
        <button class="soft-button" type="button" disabled={!canDownload} onclick={() => download('restartNow')}>{tr("自動安裝並重啟")}</button>
      {/if}
    </div>
  </div>
</dialog>
