<script lang="ts">
  import { tick } from 'svelte';
  import { listen } from '@tauri-apps/api/event';
  import { isTauri } from '@tauri-apps/api/core';
  import { invokeCommand, type UpdateInstallMode, type UpdateState } from './ipc';
  import { createUpdateController, EMPTY_UPDATE_STATE, formatBuildTime, updateProgressPercent, updateStatusText } from './update-controller';

  let { enabled, updateState = $bindable<UpdateState>({ ...EMPTY_UPDATE_STATE }) }: { enabled: boolean; updateState?: UpdateState } = $props();
  let visible = $state(false);
  let dialog = $state<HTMLDialogElement>();
  let ignoreButton = $state<HTMLButtonElement>();
  let returnFocus: HTMLElement | null = null;
  let started = false;
  const progress = $derived(updateProgressPercent(updateState));
  const canDownload = $derived(Boolean(updateState.latest) && ['available', 'error'].includes(updateState.status));
  const controller = createUpdateController({
    subscribe: (receive) => listen<UpdateState>('update-state-changed', (event) => receive(event.payload)),
    getState: () => invokeCommand('update_get_state', {}),
    check: () => invokeCommand('update_check', {}),
    ignore: () => invokeCommand('update_ignore', {}),
    download: (mode) => invokeCommand('update_download', { mode }),
    onChange: (next, show) => { updateState = next; visible = show; },
  });
  export function check() { void controller.check(); }
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

<dialog bind:this={dialog} class="update-dialog" aria-labelledby="update-title" aria-describedby="update-description" oncancel={dismiss} onkeydown={handleDialogKeydown}>
  <div class="update-dialog-content">
    <h2 id="update-title">發現新版本{updateState.latest ? ` ${updateState.latest.version}` : ''}</h2>
    <p id="update-description">選擇更新方式。下載期間可繼續播放；下載並驗證成功後才會依選擇完成更新。</p>
    {#if updateState.latest}
      <p>發布時間：{formatBuildTime(updateState.latest.publishedAt)}</p>
      {#if updateState.latest.notes}<div class="update-release-notes">{updateState.latest.notes}</div>{/if}
    {/if}
    <p class:error-text={updateState.status === 'error'} role="status" aria-live="polite">{updateStatusText(updateState)}</p>
    {#if updateState.status === 'downloading'}
      <progress aria-label="更新下載進度" max="100" value={progress ?? undefined}></progress>
      <p>{progress == null ? '正在下載' : `${progress}%`} · {(updateState.downloadedBytes / 1048576).toFixed(1)} MiB{updateState.totalBytes ? ` / ${(updateState.totalBytes / 1048576).toFixed(1)} MiB` : ''}</p>
    {/if}
    <div class="update-dialog-actions">
      <button bind:this={ignoreButton} class="soft-button" type="button" onclick={() => dismiss()}>忽略</button>
      <button class="soft-button" type="button" disabled={!canDownload} onclick={() => download('restartNow')}>自動重新啟動</button>
      <button class="soft-button" type="button" disabled={!canDownload} onclick={() => download('nextLaunch')}>下次啟動完成更新</button>
    </div>
  </div>
</dialog>
