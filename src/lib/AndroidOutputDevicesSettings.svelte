<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this list, and the two must always be the same list:
  // everything the list shows or edits lives here, once, and each mounted copy only draws it. Reading the devices
  // again is done by the copy that was mounted first (ADR 0044).
  import type {
    AndroidAudioOutputStatus,
    AndroidOutputDevice,
    AndroidUsbAudioDescriptors,
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
  /** The USB descriptor reader (ADR 0045): which USB card asked, and the answer (shared like the list). */
  const usb = $state({
    rowKey: null as string | null,
    busy: false,
    result: null as AndroidUsbAudioDescriptors | null,
    failure: null as string | null,
    copied: null as string | null,
  });
  /** The USB exclusive switch (ADR 0047, `androidUsbExclusive`), shared like the list. */
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
  import { buildAndroidRows, sameChoice } from './android-output-devices.js';
  import { chooseCardOnClick, chooseCardOnEnter } from './output-device-card.js';
  import { describeMixerAttributes, describeUsbDevice, spacedHex } from './usb-audio-descriptors.js';

  const instance = Symbol('android-output-devices');
  /** Android tells the playback service about devices at once (AudioDeviceCallback); the list reads the service's view
   *  this often while it is shown. */
  const STATUS_POLL_MS = 2000;
  /** The service pins the device and Media3 opens a new AudioTrack shortly after a choice; read again then. */
  const AFTER_CHOICE_MS = 400;

  let rows = $derived(buildAndroidRows(s.status, s.saved));
  let usbViews = $derived((usb.result?.devices ?? []).map(describeUsbDevice));
  let mixerView = $derived(describeMixerAttributes(usb.result?.mixerAttributes));
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

  // ---- USB DAC descriptors (ADR 0045): read only; the system asks for USB permission when the button is pressed. The
  // card's own output device (its row key is `device-<id>`) is also asked for its system mixer formats. ----
  async function readUsbDescriptors(rowKey: string): Promise<void> {
    if (usb.busy || !isTauri()) return;
    usb.rowKey = rowKey;
    usb.busy = true;
    usb.failure = null;
    usb.copied = null;
    try {
      const deviceId = rowKey.startsWith('device-') ? Number(rowKey.slice('device-'.length)) : null;
      usb.result = await invokeCommand('android_usb_audio_descriptors', {
        request: true,
        deviceId: Number.isFinite(deviceId) ? deviceId : null,
      });
    } catch (error) {
      usb.result = null;
      usb.failure = tr('無法讀取 USB 規格：{0}', [getErrorText(error)]);
    } finally {
      usb.busy = false;
    }
  }

  async function copyRaw(raw: string): Promise<void> {
    const text = spacedHex(raw);
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const area = document.createElement('textarea');
      area.value = text;
      document.body.append(area);
      area.select();
      document.execCommand('copy');
      area.remove();
    }
    usb.copied = raw;
  }

  // ---- USB exclusive (ADR 0047): the app drives the chosen USB audio device itself. Turning it on opens the
  // explanation panel first; only its 「前往授權」 asks for USB permission (ADR 0039), and only a granted permission
  // turns the switch on. Turning it off hands the device back at once. ----
  let usbExclusiveDialog = $state<HTMLDialogElement>();
  let usbExclusivePanel = $state<{ name: string; identity: AndroidOutputDevice | null } | null>(null);
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

  function toggleUsbExclusive(event: Event, identity: AndroidOutputDevice | null): void {
    const input = event.currentTarget as HTMLInputElement;
    if (!input.checked) {
      void saveUsbExclusive(false);
      return;
    }
    // Not on until the panel's 「前往授權」 got the permission.
    input.checked = usbExclusive.enabled;
    usbExclusiveMessage = null;
    usbExclusivePanel = { name: identity?.productName ?? '', identity };
  }

  async function authorizeUsbExclusive(): Promise<void> {
    const panel = usbExclusivePanel;
    if (!panel || usbExclusive.busy || !isTauri()) return;
    usbExclusive.busy = true;
    usbExclusiveMessage = null;
    let granted = false;
    try {
      const answer = await invokeCommand('android_request_usb_exclusive_permission', { productName: panel.name || null });
      granted = Boolean(answer?.granted);
      if (!granted) usbExclusiveMessage = answer?.found === false ? tr('找不到這台 USB 音訊裝置。') : tr('尚未取得 USB 權限。');
    } catch (error) {
      usbExclusiveMessage = getErrorText(error);
    } finally {
      usbExclusive.busy = false;
    }
    if (!granted) return;
    usbExclusivePanel = null;
    // The USB output applies to the chosen device: turning it on for this card also chooses the card.
    if (panel.identity && !sameChoice(panel.identity, s.saved)) choose(panel.identity);
    await saveUsbExclusive(true);
  }

  $effect(() => {
    const dialog = usbExclusiveDialog;
    if (!dialog) return;
    if (usbExclusivePanel && !dialog.open) dialog.showModal();
    else if (!usbExclusivePanel && dialog.open) dialog.close();
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
    <p>{tr("選擇要輸出的裝置；拔除後改由系統預設播放，重新接上時自動切回。音訊經 Android 系統混音器輸出，系統可能再轉換取樣率，不是 bit-perfect；USB 音訊裝置可改用 USB 獨佔，由本程式直接輸出。")}</p>
  </div>
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
      {#if row.connected && row.identity?.kind === 'usb'}
        <!-- This area acts on its own controls and never chooses the card (the descriptor reader is read only, ADR 0045). -->
        <div class="output-device-usb" data-own-click>
          <label class="lyrics-preference-toggle output-device-usb-exclusive">
            <input
              type="checkbox"
              checked={usbExclusive.enabled}
              disabled={usbExclusive.busy}
              onchange={(event) => toggleUsbExclusive(event, row.identity)}
            />
            <span>
              <strong>{tr('USB 獨佔（App 直接驅動，不經過系統）')}</strong>
              <small>{tr('以裝置支援的格式原樣輸出，不重新取樣；選這台裝置時才會使用')}</small>
            </span>
          </label>
          <button type="button" class="soft-button" disabled={usb.busy} onclick={() => void readUsbDescriptors(row.key)}>
            {usb.busy && usb.rowKey === row.key ? tr('讀取中…') : tr('讀取 USB 規格')}
          </button>
          {#if usb.rowKey === row.key && !usb.busy}
            {#if usb.failure}
              <p class="output-device-path is-error" role="alert">{usb.failure}</p>
            {:else if usb.result && !usbViews.length}
              <p class="output-device-path is-idle">{tr('沒有找到 USB 音訊裝置')}</p>
            {/if}
            {#each usbViews as view (view.title)}
              <div class="output-device-usb-device">
                {#if usbViews.length > 1}<strong>{view.title}</strong>{/if}
                <ul class="output-device-usb-list">
                  {#each view.lines as line}<li>{line}</li>{/each}
                  {#each view.streams as line}<li>{line}</li>{/each}
                </ul>
                {#if view.raw}
                  <details class="output-device-usb-raw">
                    <summary>{tr('原始描述元')}</summary>
                    <code>{spacedHex(view.raw)}</code>
                    <button type="button" class="soft-button" onclick={() => void copyRaw(view.raw)}>
                      {usb.copied === view.raw ? tr('已複製') : tr('複製')}
                    </button>
                  </details>
                {/if}
              </div>
            {/each}
            {#if usb.result?.mixerAttributes}
              <div class="output-device-usb-device">
                <strong>{tr('系統混音器格式')}</strong>
                {#if mixerView.notice}<p class="output-device-path is-idle">{mixerView.notice}</p>{/if}
                {#each mixerView.devices as device, index (index)}
                  {#if mixerView.devices.length > 1}<span>{device.title}</span>{/if}
                  <ul class="output-device-usb-list">
                    {#each device.lines as line}<li>{line}</li>{/each}
                  </ul>
                {/each}
              </div>
            {/if}
          {/if}
        </div>
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
  oncancel={(event) => { event.preventDefault(); usbExclusivePanel = null; }}
>
  <div class="update-dialog-content">
    <h2 id="usb-exclusive-title">{tr('USB 獨佔')}</h2>
    <p id="usb-exclusive-description">{tr('開啟後，播放到這台 USB 音訊裝置時，本程式會直接驅動它，不經過 Android 音訊系統，以檔案的取樣率與位元深度原樣輸出。這段期間其他 App 與通知音會改從手機或平板的喇叭播放，系統音量鍵對這台裝置沒有作用。')}</p>
    <p>{tr('位元完美輸出需要把本程式的音量調到 100%；低於 100% 時會先調整音量再送出。裝置可能沒有硬體音量，請先調低 USB 音訊裝置或耳機的音量。')}</p>
    <p>{tr('按下「前往授權」後，Android 會詢問是否允許本程式存取這台 USB 裝置。')}</p>
    {#if usbExclusiveMessage}<p class="error-text" role="alert">{usbExclusiveMessage}</p>{/if}
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" disabled={usbExclusive.busy} onclick={() => { usbExclusivePanel = null; }}>{tr('取消')}</button>
      <button class="soft-button" type="button" disabled={usbExclusive.busy} onclick={() => void authorizeUsbExclusive()}>{usbExclusive.busy ? tr('授權中') : tr('前往授權')}</button>
    </div>
  </div>
</dialog>

<style>
  .output-device-permission {
    margin-left: 8px;
  }

  .output-device-usb {
    display: grid;
    gap: 6px;
    justify-items: start;
    margin-top: 6px;
    padding-left: 28px;
    cursor: auto;
  }

  .output-device-usb-device {
    display: grid;
    gap: 4px;
    width: 100%;
    color: var(--text-soft);
    font-size: var(--font-caption);
  }

  .output-device-usb-list {
    margin: 0;
    padding-left: 1.2em;
  }

  .output-device-usb-exclusive {
    width: 100%;
  }

  .output-device-usb-raw summary {
    cursor: pointer;
  }

  .output-device-usb-raw code {
    display: block;
    margin: 4px 0;
    overflow-wrap: anywhere;
    user-select: text;
  }

  @media (max-width: 520px) {
    .output-device-usb {
      padding-left: 0;
    }
  }
</style>
