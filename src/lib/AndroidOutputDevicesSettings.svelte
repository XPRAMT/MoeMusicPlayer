<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this list, and the two must always be the same list:
  // everything the list shows or edits lives here, once, and each mounted copy only draws it. Reading the devices
  // again is done by the copy that was mounted first (ADR 0044).
  import type {
    AndroidAudioOutputStatus,
    AndroidOutputDevice,
  } from './ipc';
  import { DEFAULT_ANDROID_USB_EXCLUSIVE } from './android-output-devices.js';

  const s = $state({
    /** The choice in settings.json (null: the system default). */
    saved: null as AndroidOutputDevice | null,
    status: null as AndroidAudioOutputStatus | null,
    loading: true,
    loaded: false,
    failure: null as string | null,
  });
  /** The output mode (ADR 0047, `androidUsbExclusive`: true for 獨佔), shared like the list. */
  const usbExclusive = $state({
    enabled: DEFAULT_ANDROID_USB_EXCLUSIVE,
    busy: false,
    failure: null as string | null,
  });
  const liveInstances = new Set<symbol>();
  // Saving is one queue for both copies, so an older answer never overwrites a newer choice.
  let saveRevision = 0;
  let saveQueue: Promise<void> = Promise.resolve();
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { isTauri } from '@tauri-apps/api/core';
  import { getErrorText, invokeCommand } from './ipc';
  import { tr } from './i18n.js';
  import { scrollChain } from './scroll-chain.js';
  import { appTooltip } from './app-tooltip.js';
  import {
    androidOutputHeader,
    androidOutputModes,
    buildAndroidRows,
    exclusiveSwitchNeedsPanel,
    sameChoice,
  } from './android-output-devices.js';
  import { chooseCardOnClick, chooseCardOnEnter } from './output-device-card.js';

  const instance = Symbol('android-output-devices');
  /** Android tells the playback service about devices at once (AudioDeviceCallback); the list reads the service's view
   *  this often while it is shown. */
  const STATUS_POLL_MS = 2000;
  /** The service pins the device and Media3 opens a new AudioTrack shortly after a choice; read again then. */
  const AFTER_CHOICE_MS = 400;

  let rows = $derived(buildAndroidRows(s.status, s.saved, usbExclusive.enabled));
  const modes = androidOutputModes();
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  function isPrimary(): boolean {
    return liveInstances.values().next().value === instance;
  }

  async function refreshStatus(): Promise<void> {
    if (!isTauri()) return;
    try {
      const next = await invokeCommand('android_audio_output_status', {});
      if (!disposed) s.status = next;
    } catch (error) {
      if (!disposed && !s.loaded) s.failure = tr('無法讀取播放裝置設定：{0}', [getErrorText(error)]);
    }
  }

  async function load(): Promise<void> {
    if (!s.loaded) s.loading = true;
    if (!isTauri()) {
      s.loading = false;
      s.failure = tr('瀏覽器預覽不會列出播放裝置。');
      return;
    }
    try {
      const [saved, status, exclusiveOn] = await Promise.all([
        invokeCommand('settings_get_android_output_device', {}),
        invokeCommand('android_audio_output_status', {}),
        invokeCommand('settings_get_android_usb_exclusive', {}),
      ]);
      if (disposed) return;
      s.saved = saved;
      usbExclusive.enabled = exclusiveOn;
      s.status = status;
      s.failure = null;
      s.loaded = true;
    } catch (error) {
      if (!disposed) s.failure = tr('無法讀取播放裝置設定：{0}', [getErrorText(error)]);
    } finally {
      s.loading = false;
    }
  }

  function choose(device: AndroidOutputDevice | null): void {
    if (!isTauri() || sameChoice(device, s.saved)) return;
    const previous = s.saved;
    s.saved = device;
    const revision = ++saveRevision;
    saveQueue = saveQueue.catch(() => undefined).then(async () => {
      if (revision !== saveRevision) return;
      try {
        const saved = await invokeCommand('settings_set_android_output_device', { device });
        if (revision !== saveRevision) return;
        s.saved = saved;
        s.failure = null;
      } catch (error) {
        if (revision !== saveRevision) return;
        s.saved = previous;
        s.failure = getErrorText(error);
      }
      await refreshStatus();
      setTimeout(() => void refreshStatus(), AFTER_CHOICE_MS);
    });
  }

  // ---- The output mode (ADR 0047): 共享 or 獨佔, as on Windows. Switching to 獨佔 opens the explanation panel only when
  // an attached USB device is not yet permitted (`exclusiveSwitchNeedsPanel`); only its 「前往授權」 asks for USB
  // permission (ADR 0039), and the mode is saved after it. With the permission already granted, or no USB audio device
  // attached (the card asks once one is attached), the mode is saved at once. Cancel leaves the mode 共享. Switching to 共享 hands the device back at once. ----
  let usbExclusiveDialog = $state<HTMLDialogElement>();
  let usbExclusivePanelOpen = $state(false);
  let usbExclusiveMessage = $state<string | null>(null);

  async function saveUsbExclusive(enabled: boolean): Promise<void> {
    usbExclusive.busy = true;
    usbExclusive.failure = null;
    try {
      usbExclusive.enabled = await invokeCommand('settings_set_android_usb_exclusive', { enabled });
    } catch (error) {
      usbExclusive.failure = getErrorText(error);
    } finally {
      usbExclusive.busy = false;
      void refreshStatus();
      setTimeout(() => void refreshStatus(), AFTER_CHOICE_MS);
    }
  }

  function setMode(exclusive: boolean): void {
    if (!isTauri() || usbExclusive.busy) return;
    if (!exclusive) {
      usbExclusivePanelOpen = false;
      if (usbExclusive.enabled) void saveUsbExclusive(false);
      return;
    }
    if (usbExclusive.enabled) return;
    // The panel only when an attached USB device still lacks permission; otherwise the mode is saved at once.
    if (exclusiveSwitchNeedsPanel(s.status)) openUsbExclusivePanel();
    else void saveUsbExclusive(true);
  }

  /** The panel, from the 獨佔 button or from a card's 「前往授權」 (the permission of a USB device attached later). */
  function openUsbExclusivePanel(): void {
    usbExclusiveMessage = null;
    usbExclusivePanelOpen = true;
  }

  async function authorizeUsbExclusive(): Promise<void> {
    if (!usbExclusivePanelOpen || usbExclusive.busy || !isTauri()) return;
    usbExclusive.busy = true;
    usbExclusiveMessage = null;
    let proceed = false;
    try {
      // The device the routing table plays the choice on: the chosen USB device, or the first one for the system
      // default (no name). The Kotlin side applies a grant to the playing track at once.
      const productName = s.saved?.kind === 'usb' ? s.saved.productName : null;
      const answer = await invokeCommand('android_request_usb_exclusive_permission', { productName });
      // No USB audio device attached: nothing to ask now; the mode still applies (shared until one is attached).
      proceed = Boolean(answer?.granted) || answer?.found === false;
      if (!proceed) usbExclusiveMessage = tr('尚未取得 USB 權限。');
    } catch (error) {
      usbExclusiveMessage = getErrorText(error);
    } finally {
      usbExclusive.busy = false;
    }
    if (!proceed) return;
    usbExclusivePanelOpen = false;
    if (usbExclusive.enabled) {
      void refreshStatus();
      return;
    }
    await saveUsbExclusive(true);
  }

  $effect(() => {
    const dialog = usbExclusiveDialog;
    if (!dialog) return;
    if (usbExclusivePanelOpen && !dialog.open) dialog.showModal();
    else if (!usbExclusivePanelOpen && dialog.open) dialog.close();
  });

  // ---- 「附近的裝置」 for the Bluetooth codec (ADR 0044; the explanation first, the request only on 「前往授權」) ----
  // The panel belongs to the copy whose button was pressed (it is an interaction, not shown data), so each copy has
  // its own dialog; what the list shows still lives in the shared state.
  let permissionDialog = $state<HTMLDialogElement>();
  let permissionPanelOpen = $state(false);
  let permissionBusy = $state(false);
  let permissionDeniedMessage = $state<string | null>(null);

  function openPermissionPanel(): void {
    permissionDeniedMessage = null;
    permissionPanelOpen = true;
  }

  function closePermissionPanel(): void {
    permissionPanelOpen = false;
    permissionDeniedMessage = null;
  }

  async function authorizeBluetooth(): Promise<void> {
    if (permissionBusy) return;
    permissionBusy = true;
    permissionDeniedMessage = null;
    try {
      const result = await invokeCommand('android_request_bluetooth_connect', {});
      if (result?.granted) {
        permissionPanelOpen = false;
      } else if (!result?.openedSettings) {
        permissionDeniedMessage = tr('尚未授予「附近的裝置」權限。');
      } else {
        permissionPanelOpen = false;
      }
    } catch (error) {
      permissionDeniedMessage = getErrorText(error);
    } finally {
      permissionBusy = false;
      void refreshStatus();
    }
  }

  $effect(() => {
    const dialog = permissionDialog;
    if (!dialog) return;
    if (permissionPanelOpen && !dialog.open) dialog.showModal();
    else if (!permissionPanelOpen && dialog.open) dialog.close();
  });

  function onVisibility(): void {
    if (document.visibilityState === 'visible' && isPrimary()) void refreshStatus();
  }

  onMount(() => {
    const alone = liveInstances.size === 0;
    liveInstances.add(instance);
    // Another copy already read everything and keeps it current; a copy that is alone reads it afresh.
    if (alone || !s.loaded) void load();
    pollTimer = setInterval(() => {
      if (isPrimary() && document.visibilityState === 'visible') void refreshStatus();
    }, STATUS_POLL_MS);
    document.addEventListener('visibilitychange', onVisibility);
  });

  onDestroy(() => {
    disposed = true;
    liveInstances.delete(instance);
    if (pollTimer) clearInterval(pollTimer);
    document.removeEventListener('visibilitychange', onVisibility);
  });
</script>

<div class="settings-panel-header">
  <div>
    <h3>{tr("播放裝置")}</h3>
    <p>{androidOutputHeader(usbExclusive.enabled)}</p>
  </div>
</div>

<div class="output-mode" role="group" aria-label={tr('輸出模式')}>
  {#each modes as mode (mode.exclusive)}
    <button
      type="button"
      class="layout-choice-button"
      class:chosen={usbExclusive.enabled === mode.exclusive}
      aria-pressed={usbExclusive.enabled === mode.exclusive}
      disabled={usbExclusive.busy}
      onclick={() => setMode(mode.exclusive)}
    >
      <strong>{mode.title}</strong>
      <span>{mode.description}</span>
    </button>
  {/each}
</div>

{#if s.failure}<p class="source-error-message" role="alert">{s.failure}</p>{/if}
{#if usbExclusive.failure}<p class="source-error-message" role="alert">{usbExclusive.failure}</p>{/if}

<div class="output-device-list" role="radiogroup" aria-label={tr('播放裝置')} aria-busy={s.loading} use:scrollChain>
  {#each rows as row (row.key)}
    {@const locked = !row.connected || (row.key !== 'system' && row.identity === null)}
    <!-- The card is the control: its radio is out of sight and keeps the keyboard (Tab, arrows, Space, Enter). -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="output-device"
      class:is-playing={row.chosen}
      class:is-missing={!row.connected}
      class:is-locked={locked}
      onclick={chooseCardOnClick}
    >
      <div class="output-device-head">
        <label class="output-device-toggle">
          <input
            class="output-device-input"
            type="radio"
            name="android-output-device"
            checked={row.chosen}
            disabled={locked}
            aria-label={row.label}
            onchange={() => choose(row.identity)}
            onkeydown={chooseCardOnEnter}
          />
          <span class="output-device-dot is-{row.path?.tone ?? 'idle'}" aria-hidden="true"></span>
          <span class="output-device-name" use:appTooltip={row.label}>
            <strong>{row.label}</strong>
            {#if row.sub}<small>{row.sub}</small>{/if}
          </span>
        </label>
        {#if !row.connected}
          <button type="button" class="soft-button" onclick={() => choose(null)}>{tr('移除')}</button>
        {/if}
      </div>
      {#if row.capabilities}
        <p class="output-device-path is-idle">{row.capabilities}</p>
      {/if}
      {#if row.bluetooth}
        <p class="output-device-path" class:is-warn={row.bluetooth.needsPermission}>
          {row.bluetooth.text}
          {#if row.bluetooth.needsPermission}
            <button type="button" class="soft-button output-device-permission" onclick={openPermissionPanel}>{tr('前往授權')}</button>
          {/if}
        </p>
      {/if}
      {#if row.path?.text}
        <p class="output-device-path is-{row.path.tone}">
          {row.path.text}{#if row.path.notice}<span class="output-device-note"> — {row.path.notice}</span>{/if}
        </p>
      {/if}
      {#if row.hint}
        <p class="output-device-path is-idle" class:is-warn={row.hint.needsPermission}>
          {row.hint.text}
          {#if row.hint.needsPermission}
            <button type="button" class="soft-button output-device-permission" onclick={openUsbExclusivePanel}>{tr('前往授權')}</button>
          {/if}
        </p>
      {/if}
    </div>
  {/each}
</div>

<dialog
  bind:this={permissionDialog}
  class="update-dialog floating-surface"
  aria-labelledby="bluetooth-permission-title"
  aria-describedby="bluetooth-permission-description"
  oncancel={(event) => { event.preventDefault(); closePermissionPanel(); }}
>
  <div class="update-dialog-content">
    <h2 id="bluetooth-permission-title">{tr('需要「附近的裝置」權限')}</h2>
    <p id="bluetooth-permission-description">{tr('MoeMusicPlayer 需要「附近的裝置」權限，才能讀取藍牙耳機目前使用的編碼（例如 LDAC）與取樣率；不會搜尋或連線到其他裝置。')}</p>
    <p>{tr('按下「前往授權」後，Android 會詢問是否允許；若系統不再詢問，會開啟本程式的設定頁，請在「權限」中允許「附近的裝置」，再返回本程式。')}</p>
    {#if permissionDeniedMessage}<p class="error-text" role="alert">{permissionDeniedMessage}</p>{/if}
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" disabled={permissionBusy} onclick={closePermissionPanel}>{tr('取消')}</button>
      <button class="soft-button" type="button" disabled={permissionBusy} onclick={() => void authorizeBluetooth()}>{permissionBusy ? tr('授權中') : tr('前往授權')}</button>
    </div>
  </div>
</dialog>

<dialog
  bind:this={usbExclusiveDialog}
  class="update-dialog floating-surface"
  aria-labelledby="usb-exclusive-title"
  aria-describedby="usb-exclusive-description"
  oncancel={(event) => { event.preventDefault(); usbExclusivePanelOpen = false; }}
>
  <div class="update-dialog-content">
    <h2 id="usb-exclusive-title">{tr('獨佔模式')}</h2>
    <p id="usb-exclusive-description">{tr('切換到獨佔模式後，播放到 USB 音訊裝置時，本程式會直接驅動它，不經過 Android 音訊系統，以檔案的取樣率與位元深度原樣輸出。這段期間其他 App 與通知音會改從手機或平板的喇叭播放，系統音量鍵對這台裝置沒有作用。藍牙與內建喇叭不支援獨佔，照常以共享方式播放。')}</p>
    <p>{tr('位元完美輸出需要把本程式的音量調到 100%；低於 100% 時會先調整音量再送出。裝置可能沒有硬體音量，請先調低 USB 音訊裝置或耳機的音量。')}</p>
    <p>{tr('按下「前往授權」後，Android 會詢問是否允許本程式存取 USB 音訊裝置；目前沒有接上 USB 音訊裝置時，會先切換為獨佔模式，接上後再授權。')}</p>
    {#if usbExclusiveMessage}<p class="error-text" role="alert">{usbExclusiveMessage}</p>{/if}
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" disabled={usbExclusive.busy} onclick={() => { usbExclusivePanelOpen = false; }}>{tr('取消')}</button>
      <button class="soft-button" type="button" disabled={usbExclusive.busy} onclick={() => void authorizeUsbExclusive()}>{usbExclusive.busy ? tr('授權中') : tr('前往授權')}</button>
    </div>
  </div>
</dialog>

<style>
  /* The same mode group as the Windows list (OutputDevicesSettings.svelte). */
  .output-mode {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 8px;
    margin: 0 0 14px;
  }

  .output-device-permission {
    margin-left: 8px;
  }
</style>
