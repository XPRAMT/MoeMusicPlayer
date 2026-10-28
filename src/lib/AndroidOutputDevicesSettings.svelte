<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this list, and the two must always be the same list:
  // everything the list shows or edits lives here, once, and each mounted copy only draws it. Reading the devices
  // again is done by the copy that was mounted first (ADR 0044).
  import type {
    AndroidAudioOutputStatus,
    AndroidMixerRateSourceStatus,
    AndroidOutputDevice,
  } from './ipc';
  import {
    DEFAULT_ANDROID_BLUETOOTH_CODEC_CUSTOM,
    DEFAULT_ANDROID_BLUETOOTH_RATE_FOLLOW,
    DEFAULT_ANDROID_USB_EXCLUSIVE,
  } from './android-output-devices.js';

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
  /** The Bluetooth sample-rate follow switch (ADR 0051, `androidBluetoothRateFollow`), shared like the list. */
  const btRate = $state({
    enabled: DEFAULT_ANDROID_BLUETOOTH_RATE_FOLLOW,
    busy: false,
    failure: null as string | null,
  });
  /** The Bluetooth codec priority list (ADR 0052): the switch and the one stored order, shared like the list. */
  const btCodec = $state({
    custom: DEFAULT_ANDROID_BLUETOOTH_CODEC_CUSTOM,
    order: [] as string[],
    busy: false,
    failure: null as string | null,
  });
  /** The sources of the real mixer rate (ADR 0048), shared like the list; null until read. */
  const mixerSource = $state({ status: null as AndroidMixerRateSourceStatus | null });
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
    bluetoothCodecSection,
    bluetoothPermissionNotice,
    buildAndroidRows,
    buildCodecRows,
    moveCodecInOrder,
    pickCodecDevice,
    sameChoice,
  } from './android-output-devices.js';
  import { chooseCardOnClick, chooseCardOnEnter } from './output-device-card.js';
  import { exceedsLongPressSlop, longPressDragGuard } from './long-press-reorder.js';
  import { QUEUE_REORDER_LONG_PRESS_MS, QUEUE_REORDER_MOVE_CANCEL_PX } from './queue-reorder.js';
  import { trackColumnDropIndexFromPointer } from './track-columns.js';
  import { mixerPanelShouldClose, mixerRateEntryVisible, mixerRatePanel, mixerRateSummary } from './mixer-rate-source.js';

  interface Props {
    /** Told when the output mode is read or changed (the DSD passthrough text under 「DSD」 depends on it). */
    onExclusiveChange?: (exclusive: boolean) => void;
  }

  let { onExclusiveChange = () => {} }: Props = $props();

  $effect(() => {
    onExclusiveChange(usbExclusive.enabled);
  });

  const instance = Symbol('android-output-devices');
  /** Android tells the playback service about devices at once (AudioDeviceCallback); the list reads the service's view
   *  this often while it is shown. */
  const STATUS_POLL_MS = 500;
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
      const [saved, status, exclusiveOn, rateFollowOn, codecCustom, codecOrder] = await Promise.all([
        invokeCommand('settings_get_android_output_device', {}),
        invokeCommand('android_audio_output_status', {}),
        invokeCommand('settings_get_android_usb_exclusive', {}),
        invokeCommand('settings_get_android_bluetooth_rate_follow', {}),
        invokeCommand('settings_get_android_bluetooth_codec_custom', {}),
        invokeCommand('settings_get_android_bluetooth_codec_order', {}),
      ]);
      if (disposed) return;
      s.saved = saved;
      usbExclusive.enabled = exclusiveOn;
      btRate.enabled = rateFollowOn;
      btCodec.custom = codecCustom;
      btCodec.order = codecOrder;
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

  // ---- Bluetooth sample-rate follow (ADR 0051): a plain switch, saved at once. Without root or an authorized Shizuku it
  // only shows a notice (the mixer-rate panel below is the place to get either). ----
  async function setBluetoothRateFollow(enabled: boolean): Promise<void> {
    if (!isTauri() || btRate.busy) return;
    btRate.busy = true;
    btRate.failure = null;
    try {
      btRate.enabled = await invokeCommand('settings_set_android_bluetooth_rate_follow', { enabled });
    } catch (error) {
      btRate.failure = getErrorText(error);
    } finally {
      btRate.busy = false;
    }
  }

  let btNotice = $derived(bluetoothPermissionNotice(mixerSource.status));

  // ---- Bluetooth codec priority list (ADR 0052): a switch and a list the user reorders by long-press drag (the same
  // gesture as 設定 → 曲目欄位). The order is one list for every headset; with the switch off it is still editable and
  // stored but nothing is applied to the headset. Without root or an authorized Shizuku only a notice is shown. ----
  let codecSection = $derived(bluetoothCodecSection(mixerSource.status, s.status?.bluetoothCodecChoices));
  let codecDevice = $derived(pickCodecDevice(s.status));
  let codecRows = $derived(buildCodecRows(s.status?.bluetoothCodecChoices?.local ?? [], btCodec.order, codecDevice));

  async function setBluetoothCodecCustom(enabled: boolean): Promise<void> {
    if (!isTauri() || btCodec.busy) return;
    btCodec.busy = true;
    btCodec.failure = null;
    try {
      btCodec.custom = await invokeCommand('settings_set_android_bluetooth_codec_custom', { enabled });
    } catch (error) {
      btCodec.failure = getErrorText(error);
    } finally {
      btCodec.busy = false;
      void refreshStatus();
      setTimeout(() => void refreshStatus(), 3500);
    }
  }

  let codecSaveQueue: Promise<void> = Promise.resolve();

  function saveCodecOrder(order: string[]): void {
    if (!isTauri()) return;
    const previous = btCodec.order;
    btCodec.order = order;
    btCodec.failure = null;
    codecSaveQueue = codecSaveQueue.catch(() => undefined).then(async () => {
      try {
        btCodec.order = await invokeCommand('settings_set_android_bluetooth_codec_order', { order });
      } catch (error) {
        btCodec.order = previous;
        btCodec.failure = getErrorText(error);
      }
      void refreshStatus();
      setTimeout(() => void refreshStatus(), 3500);
    });
  }

  function moveCodec(from: number, to: number): void {
    const local = s.status?.bluetoothCodecChoices?.local ?? [];
    const next = moveCodecInOrder(btCodec.order, local, codecRows.map((row) => row.key), from, to);
    if (next.join() !== btCodec.order.join()) saveCodecOrder(next);
  }

  let codecDrag = $state<{ pointerId: number; fromIndex: number; toIndex: number } | null>(null);
  let codecPending: { pointerId: number; fromIndex: number; startX: number; startY: number } | null = null;
  let codecTimer: ReturnType<typeof setTimeout> | null = null;

  function clearCodecTimer(): void {
    if (codecTimer !== null) clearTimeout(codecTimer);
    codecTimer = null;
  }

  function codecPointerDown(event: PointerEvent, index: number): void {
    if (event.button !== 0) return;
    clearCodecTimer();
    codecPending = { pointerId: event.pointerId, fromIndex: index, startX: event.clientX, startY: event.clientY };
    const target = event.currentTarget as HTMLElement;
    codecTimer = setTimeout(() => {
      const pending = codecPending;
      if (!pending || pending.pointerId !== event.pointerId) return;
      codecPending = null;
      codecDrag = { pointerId: pending.pointerId, fromIndex: pending.fromIndex, toIndex: pending.fromIndex };
      try {
        target.setPointerCapture(pending.pointerId);
      } catch {
        // The pointer may have ended while the long-press callback was queued.
      }
    }, QUEUE_REORDER_LONG_PRESS_MS);
  }

  function codecPointerMove(event: PointerEvent): void {
    const pending = codecPending;
    if (pending?.pointerId === event.pointerId && !codecDrag) {
      if (exceedsLongPressSlop(event.clientX - pending.startX, event.clientY - pending.startY, QUEUE_REORDER_MOVE_CANCEL_PX)) {
        clearCodecTimer();
        codecPending = null;
      }
      return;
    }
    const drag = codecDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    event.preventDefault();
    const list = (event.currentTarget as HTMLElement).closest<HTMLOListElement>('.bluetooth-codec-list');
    const rowElements = list ? [...list.querySelectorAll<HTMLElement>('[data-codec-index]')] : [];
    if (rowElements.length === 0) return;
    const centers = rowElements.map((row) => {
      const rect = row.getBoundingClientRect();
      return rect.top + rect.height / 2;
    });
    const destination = trackColumnDropIndexFromPointer(event.clientY, centers, drag.fromIndex);
    if (destination !== drag.toIndex) codecDrag = { ...drag, toIndex: destination };
  }

  function codecPointerUp(event: PointerEvent): void {
    if (codecPending?.pointerId === event.pointerId) {
      clearCodecTimer();
      codecPending = null;
    }
    const drag = codecDrag;
    if (!drag || drag.pointerId !== event.pointerId) return;
    event.preventDefault();
    try {
      (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already have been released.
    }
    codecDrag = null;
    if (drag.fromIndex !== drag.toIndex) moveCodec(drag.fromIndex, drag.toIndex);
  }

  function codecPointerCancel(event: PointerEvent): void {
    if (codecPending?.pointerId === event.pointerId || codecDrag?.pointerId === event.pointerId) {
      clearCodecTimer();
      codecPending = null;
      codecDrag = null;
    }
  }

  // ---- The output mode (ADR 0047): 共享 or 獨佔, as on Windows. Both buttons save at once; there is no panel of ours. In
  // 獨佔 the playback service asks Android's USB permission dialog by itself whenever the USB audio device the choice plays
  // on is attached and not permitted (the mode choice is the request, the system dialog the consent, so ADR 0039's panel
  // rule is not applied). A refused device is asked again only by the card's 「前往授權」 (`requestUsbPermission`). ----
  let usbPermissionAsking = $state(false);

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
    if (!isTauri() || usbExclusive.busy || usbExclusive.enabled === exclusive) return;
    void saveUsbExclusive(exclusive);
  }

  /** The card's 「前往授權」: Android's USB permission dialog for the device the routing table plays the choice on. */
  async function requestUsbPermission(): Promise<void> {
    if (usbPermissionAsking || !isTauri()) return;
    usbPermissionAsking = true;
    usbExclusive.failure = null;
    try {
      // The chosen USB device, or the first one for the system default (no name). The Kotlin side applies a grant to the
      // playing track at once.
      const productName = s.saved?.kind === 'usb' ? s.saved.productName : null;
      await invokeCommand('android_request_usb_exclusive_permission', { productName });
    } catch (error) {
      usbExclusive.failure = getErrorText(error);
    } finally {
      usbPermissionAsking = false;
      void refreshStatus();
    }
  }

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

  // ---- 讀取實際的系統取樣率 (ADR 0048): the explanation first; root or Shizuku is requested only by a panel button ----
  let mixerDialog = $state<HTMLDialogElement>();
  let mixerPanelOpen = $state(false);
  let mixerBusy = $state(false);
  let mixerFailure = $state<string | null>(null);
  let mixerPanel = $derived(mixerSource.status ? mixerRatePanel(mixerSource.status) : null);

  async function readMixerSource(): Promise<void> {
    if (!isTauri()) return;
    try {
      const next = await invokeCommand('android_mixer_rate_source_status', {});
      if (!disposed && next) mixerSource.status = next;
    } catch {
      // The line is only a hint; a failed read leaves it out.
    }
  }

  function openMixerPanel(): void {
    mixerFailure = null;
    mixerPanelOpen = true;
    void readMixerSource();
  }

  function closeMixerPanel(): void {
    mixerPanelOpen = false;
    mixerFailure = null;
  }

  async function runMixerAction(action: 'checkRoot' | 'requestShizuku' | 'openShizuku' | 'recheck'): Promise<void> {
    if (mixerBusy) return;
    mixerBusy = true;
    mixerFailure = null;
    try {
      const next = await invokeCommand('android_mixer_rate_source_action', { action });
      if (next) mixerSource.status = next;
    } catch (error) {
      mixerFailure = getErrorText(error);
    } finally {
      mixerBusy = false;
      void refreshStatus();
      setTimeout(() => void refreshStatus(), 2500);
    }
  }

  // Once root or Shizuku is available (the authorization result, coming back to the app, 「重新檢查」) the panel closes by
  // itself; the entries are already hidden by the same status.
  $effect(() => {
    if (mixerPanelOpen && mixerPanelShouldClose(mixerSource.status)) closeMixerPanel();
  });

  $effect(() => {
    const dialog = mixerDialog;
    if (!dialog) return;
    if (mixerPanelOpen && !dialog.open) dialog.showModal();
    else if (!mixerPanelOpen && dialog.open) dialog.close();
  });

  function onVisibility(): void {
    if (document.visibilityState === 'visible' && isPrimary()) {
      void refreshStatus();
      // Coming back from the Shizuku app or the root manager: read the sources again.
      if (mixerPanelOpen || mixerSource.status) void readMixerSource();
    }
  }

  onMount(() => {
    const alone = liveInstances.size === 0;
    liveInstances.add(instance);
    // Another copy already read everything and keeps it current; a copy that is alone reads it afresh.
    if (alone || !s.loaded) void load();
    if (alone || !mixerSource.status) void readMixerSource();
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
    <h3>{tr("模式")}</h3>
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

<div class="settings-panel-header">
  <div>
    <h3>{tr("裝置")}</h3>
    <p>{androidOutputHeader(usbExclusive.enabled)}</p>
  </div>
</div>

{#if mixerRateEntryVisible(mixerSource.status)}
  <p class="mixer-source-line">
    {mixerRateSummary(mixerSource.status)}
    <button type="button" class="soft-button output-device-permission" onclick={openMixerPanel}>{tr('前往授權')}</button>
  </p>
{/if}

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
            <button type="button" class="soft-button output-device-permission" disabled={usbPermissionAsking} onclick={() => void requestUsbPermission()}>{usbPermissionAsking ? tr('授權中') : tr('前往授權')}</button>
          {/if}
        </p>
      {/if}
    </div>
  {/each}
</div>

<div class="settings-panel-header">
  <div>
    <h3>{tr("藍牙")}</h3>
  </div>
</div>

{#if btNotice}
  <p class="mixer-source-line">
    {btNotice}
    <button type="button" class="soft-button output-device-permission" onclick={openMixerPanel}>{tr('前往授權')}</button>
  </p>
{/if}

<label class="bluetooth-rate-follow">
  <input
    type="checkbox"
    checked={btRate.enabled}
    disabled={btRate.busy}
    onchange={(event) => void setBluetoothRateFollow(event.currentTarget.checked)}
  />
  <span>
    <strong>{tr('藍牙取樣率跟隨曲目')}</strong>
    <small>{tr('換歌時把藍牙編碼的取樣率改成接近這首歌的值（需要 root 或 Shizuku）。')}</small>
  </span>
</label>
{#if btRate.failure}<p class="source-error-message" role="alert">{btRate.failure}</p>{/if}

<label class="bluetooth-rate-follow">
    <input
      type="checkbox"
      checked={btCodec.custom}
      disabled={btCodec.busy}
      onchange={(event) => void setBluetoothCodecCustom(event.currentTarget.checked)}
    />
    <span>
      <strong>{tr('自訂藍牙編碼順序')}</strong>
      <small>{tr('連線與調整順序時，依下列順序使用耳機支援的第一個編碼；關閉時只記住順序，不會切換耳機的編碼。')}</small>
    </span>
  </label>
  {#if codecSection === 'connect'}
    <p class="mixer-source-line">{tr('連接藍牙耳機後可選擇編碼')}</p>
  {:else if codecSection === 'list'}
    <ol class="track-column-settings bluetooth-codec-list" use:longPressDragGuard={() => codecDrag !== null} aria-label={tr('藍牙編碼順序')}>
      {#each codecRows as row, index (row.key)}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
        <li
          class="track-column-setting-row"
          class:column-order-dragging={codecDrag?.fromIndex === index}
          class:column-order-drop-before={codecDrag !== null && codecDrag.toIndex === index && codecDrag.fromIndex > index}
          class:column-order-drop-after={codecDrag !== null && codecDrag.toIndex === index && codecDrag.fromIndex < index}
          data-codec-index={index}
          use:appTooltip={tr('長按 300 毫秒後拖曳排序')}
          onpointerdown={(event) => codecPointerDown(event, index)}
          onpointermove={codecPointerMove}
          onpointerup={codecPointerUp}
          onpointercancel={codecPointerCancel}
        >
          <span class="track-column-setting-label">
            <span>{row.name}</span>
            {#if row.active}<small class="bluetooth-codec-tag is-active">{tr('使用中')}</small>{/if}
            {#if row.unsupported}<small class="bluetooth-codec-tag">{tr('此耳機不支援')}</small>{/if}
          </span>
          <div class="track-column-order-actions">
            <span aria-label={tr('第 {0} 項', [index + 1])}>{index + 1}</span>
          </div>
        </li>
      {/each}
    </ol>
    {#if codecDevice?.error}<p class="source-error-message" role="alert">{tr('無法切換藍牙編碼')}</p>{/if}
  {/if}
  {#if btCodec.failure}<p class="source-error-message" role="alert">{btCodec.failure}</p>{/if}


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
  bind:this={mixerDialog}
  class="update-dialog floating-surface"
  aria-labelledby="mixer-source-title"
  oncancel={(event) => { event.preventDefault(); closeMixerPanel(); }}
>
  {#if mixerPanel}
    <div class="update-dialog-content">
      <h2 id="mixer-source-title">{mixerPanel.title}</h2>
      {#each mixerPanel.paragraphs as paragraph}<p>{paragraph}</p>{/each}
      {#if mixerFailure}<p class="error-text" role="alert">{mixerFailure}</p>{/if}
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" disabled={mixerBusy} onclick={closeMixerPanel}>{tr('關閉')}</button>
        {#each mixerPanel.buttons as item (item.action)}
          <button class="soft-button" type="button" disabled={mixerBusy} onclick={() => void runMixerAction(item.action)}>{mixerBusy ? tr('授權中') : item.label}</button>
        {/each}
      </div>
    </div>
  {/if}
</dialog>

<style>
  .mixer-source-line {
    margin: 0 0 10px;
    font-size: 0.9em;
    opacity: 0.8;
  }

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

  .bluetooth-rate-follow {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin: 0 0 10px;
    cursor: pointer;
  }

  .bluetooth-rate-follow span {
    display: grid;
    gap: 2px;
  }

  .bluetooth-rate-follow small {
    opacity: 0.75;
  }

  .bluetooth-codec-list {
    margin: 0 0 10px;
  }

  .bluetooth-codec-tag {
    margin-left: 8px;
    opacity: 0.7;
  }

  .bluetooth-codec-tag.is-active {
    color: var(--accent-text);
    opacity: 1;
  }
</style>
