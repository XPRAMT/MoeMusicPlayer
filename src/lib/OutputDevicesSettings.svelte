<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this list, and the two must always be the same list:
  // everything the list shows or edits lives here, once, and each mounted copy only draws it. Work that must happen
  // once (reading devices, reacting to Windows) is done by the copy that was mounted first.
  import type {
    AudioOutputDevice,
    AudioOutputDeviceStatus,
    OutputDeviceEntry,
  } from './ipc';

  /** Where the volume-link switch used to be kept (now `settings.json`; read once to carry the choice over). */
  const LEGACY_SYNC_KEY = 'moe.outputVolumeSync';
  const PRESET_SLOTS = 5;

  const s = $state({
    /** "音量同步", kept and applied by the backend (it works with no settings UI open). */
    volumeSync: true,
    devices: [] as AudioOutputDevice[],
    entries: [] as OutputDeviceEntry[],
    statuses: [] as AudioOutputDeviceStatus[],
    presets: Array.from({ length: PRESET_SLOTS }, () => null) as (OutputDeviceEntry[] | null)[],
    selectedSlot: 0,
    /** The real Windows volume (0 to 100) and mute state of each device, by device id. */
    volumes: {} as Record<string, { percent: number; muted: boolean }>,
    /** WASAPI exclusive mode: one device, bit-perfect, no mixing with other programs. */
    exclusive: false,
    loading: true,
    loaded: false,
    failure: null as string | null,
  });
  const volumeEdited = new Map<string, number>();
  const liveInstances = new Set<symbol>();
  // Saving is one queue for both copies, so an older answer never overwrites a newer edit.
  let saveRevision = 0;
  let saveQueue: Promise<void> = Promise.resolve();
</script>

<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { appTooltip } from './app-tooltip.js';
  import { dockMarquee } from './dock-marquee.js';
  import { scrollChain } from './scroll-chain.js';
  import { getErrorText, invokeCommand } from './ipc';
  import { tr } from './i18n.js';
  import type { DeviceVolumeChanged, PlaybackOutputFormat } from './ipc';
  import {
    describeExclusive,
    describeExclusiveFallback,
    describeMirrorPath,
    describeOutputFormat,
    formatSupportedFormats,
  } from './output-format.js';
  import { chooseCardOnClick, chooseCardOnEnter } from './output-device-card.js';
  import {
    OUTPUT_DELAY_MAX_MS,
    OUTPUT_MAX_SOURCE_CHANNELS,
    buildDeviceRows,
    channelLabels,
    disconnectedPlayingRows,
    entriesFromRows,
    formatDelay,
    isLastPlaying,
    isMainStream,
    nudgeCell,
    patchRow,
    rowEnabled,
    sameDeviceList,
    sameDevices,
    selectOnly,
    setRowEnabled,
    shortDeviceName,
    sourceRowCount,
    standardRoutes,
    statusFor,
    takeoverRow,
    toggleCell,
    volumeDeviceId,
  } from './output-devices.js';

  interface Props {
    /** The player's current sample-rate path (the main stream's). */
    outputFormat?: PlaybackOutputFormat | null;
    /** Told when the exclusive-mode setting is read or changed (other parts of the settings page depend on it). */
    onExclusiveChange?: (exclusive: boolean) => void;
  }

  let { outputFormat = null, onExclusiveChange = () => {} }: Props = $props();

  const instance = Symbol('output-devices');
  const SAVE_DELAY_MS = 250;
  const STATUS_POLL_MS = 2000;
  /** Windows tells us about volume changes (an event per change); this slow read only catches what that missed. */
  const VOLUME_POLL_MS = 5000;
  /** How often the list of playback devices is read again, so a device that disconnects shows up. */
  /** Windows reports device changes as an event; this slow list read only covers a missed one. */
  const DEVICE_POLL_MS = 5000;
  /** Notifications come in bursts (a device appearing fires several); read the list once per burst. */
  const DEVICE_EVENT_DEBOUNCE_MS = 150;
  const VOLUME_SEND_MS = 60;
  /** A volume the user just changed is not overwritten by a poll for this long. */
  const VOLUME_HOLD_MS = 1500;
  const DELAY_STEP_MS = 10;
  /** Matrix cells are this wide; the first column of each device is this much wider so the gap sits inside the
   *  column (a margin on the cell would narrow it). */
  const MATRIX_CELL_PX = 38;
  const MATRIX_GROUP_GAP_PX = 10;
  /** The column with the source-channel names (FL, FR, ...). */
  const MATRIX_LABEL_PX = 32;


  let rows = $derived(buildDeviceRows(s.entries, s.devices));
  /** The delay lines devices up with each other, so it only applies (and shows) with two or more chosen devices. */
  let delayApplies = $derived(rows.filter((_, index) => rowEnabled(rows, index)).length > 1);
  let playingRows = $derived(rows.map((row, index) => ({ row, index })).filter(({ index }) => rowEnabled(rows, index) && rows[index].connected));
  /** The row that carries on playing because a chosen device disconnected (the new system default). */
  let takeover = $derived(takeoverRow(rows));
  let defaultDeviceName = $derived(s.devices.find((device) => device.isDefault)?.name ?? null);
  const sourceLabels = channelLabels(OUTPUT_MAX_SOURCE_CHANNELS);
  /** How many source-channel rows the shared matrix shows: as many as the widest playing device has output channels
   *  (at least the stereo pair), or more when a map already uses a higher source. */
  let matrixRows = $derived(
    Math.min(
      OUTPUT_MAX_SOURCE_CHANNELS,
      Math.max(
        2,
        ...playingRows.map(({ index }) => channelCount(index)),
        sourceRowCount(playingRows.flatMap(({ index }) => rows[index].entry?.channelMap ?? []), false),
      ),
    ),
  );

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let volumeTimer: ReturnType<typeof setInterval> | null = null;
  let deviceTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;
  let unlistenVolume: (() => void) | null = null;
  let unlistenDevices: (() => void) | null = null;
  let deviceEventTimer: ReturnType<typeof setTimeout> | null = null;

  const volumePending = new Map<string, number>();
  const volumeSendTimers = new Map<string, ReturnType<typeof setTimeout>>();

  async function setVolumeSync(on: boolean): Promise<void> {
    s.volumeSync = on;
    if (!isTauri()) return;
    try {
      s.volumeSync = await invokeCommand('settings_set_output_volume_sync', { enabled: on });
    } catch (error) {
      s.volumeSync = !on;
      s.failure = getErrorText(error);
    }
  }

  /** The switch used to live in the browser's storage; carry a choice made there over, once. */
  async function adoptLegacySyncChoice(saved: boolean): Promise<boolean> {
    let legacy: string | null = null;
    try {
      legacy = localStorage.getItem(LEGACY_SYNC_KEY);
      localStorage.removeItem(LEGACY_SYNC_KEY);
    } catch {
      return saved;
    }
    if (legacy === '1' && !saved) return invokeCommand('settings_set_output_volume_sync', { enabled: true });
    return saved;
  }

  async function refreshStatus(): Promise<void> {
    try {
      const next = await invokeCommand('audio_output_device_status', {});
      if (!disposed) s.statuses = next;
    } catch {
      // The status is a hint only.
    }
  }

  /** Windows reported a volume or mute change of a device. */
  function onVolumeChanged(change: DeviceVolumeChanged): void {
    if (!isPrimary()) return;
    const percent = Math.round(change.volumePercent);
    // Our own writes (and the slider the user is dragging) come back as events too; they are already shown.
    if (Date.now() - (volumeEdited.get(change.id) ?? 0) < VOLUME_HOLD_MS) return;
    const previous = s.volumes[change.id];
    if (previous && previous.percent === percent && previous.muted === change.muted) return;
    // Only shows it: when the volume link is on, the backend moves the other devices (and their changes arrive here
    // the same way), whether or not this list is open.
    s.volumes = { ...s.volumes, [change.id]: { percent, muted: change.muted } };
  }

  async function refreshVolumes(): Promise<void> {
    if (!isTauri()) return;
    try {
      const listed = await invokeCommand('audio_get_device_volumes', {});
      if (disposed) return;
      const now = Date.now();
      const next = { ...s.volumes };
      for (const item of listed) {
        // Keep what the user is dragging; take everything else (the real volume moves from outside too).
        if (now - (volumeEdited.get(item.id) ?? 0) < VOLUME_HOLD_MS) continue;
        next[item.id] = { percent: Math.round(item.volumePercent), muted: item.muted };
      }
      s.volumes = next;
    } catch {
      // Devices come and go; the next poll tries again.
    }
  }

  /** Read the device list again without any visible busy state; redraws only when something changed. */
  async function pollDevices(): Promise<void> {
    if (!isTauri() || s.loading) return;
    try {
      const next = await invokeCommand('audio_list_output_devices', {});
      if (!disposed && !sameDeviceList(next, s.devices)) {
        s.devices = next;
        void refreshVolumes();
        void refreshStatus();
      }
    } catch {
      // Keep the list that is shown.
    }
  }

  function onDevicesChanged(): void {
    if (!isPrimary()) return;
    if (deviceEventTimer) clearTimeout(deviceEventTimer);
    deviceEventTimer = setTimeout(() => {
      deviceEventTimer = null;
      void pollDevices();
      void refreshStatus();
    }, DEVICE_EVENT_DEBOUNCE_MS);
  }

  $effect(() => {
    onExclusiveChange(s.exclusive);
  });

  async function load(): Promise<void> {
    if (!s.loaded) s.loading = true;
    if (!isTauri()) {
      s.loading = false;
      s.failure = tr('瀏覽器預覽不會列出播放裝置。');
      return;
    }
    try {
      const [saved, listed, savedPresets, savedExclusive, savedSync] = await Promise.all([
        invokeCommand('settings_get_output_devices', {}),
        invokeCommand('audio_list_output_devices', {}),
        invokeCommand('settings_get_output_presets', {}),
        invokeCommand('settings_get_output_exclusive', {}),
        invokeCommand('settings_get_output_volume_sync', {}),
      ]);
      const volumeSync = await adoptLegacySyncChoice(savedSync);
      if (disposed) return;
      s.exclusive = savedExclusive;
      s.volumeSync = volumeSync;
      s.entries = saved;
      s.devices = listed;
      s.presets = savedPresets;
      s.selectedSlot = Math.max(0, savedPresets.findIndex((slot) => slot && sameDevices(slot, saved)));
      s.failure = null;
      s.loaded = true;
    } catch (error) {
      if (!disposed) s.failure = tr('無法讀取播放裝置設定：{0}', [getErrorText(error)]);
    } finally {
      s.loading = false;
    }
    void refreshStatus();
    void refreshVolumes();
  }

  /** The copy that mounted first does the work that must happen once. */
  function isPrimary(): boolean {
    return liveInstances.values().next().value === instance;
  }

  function save(): void {
    if (!isTauri()) return;
    const revision = ++saveRevision;
    const toSave = s.entries;
    saveQueue = saveQueue.catch(() => undefined).then(async () => {
      if (disposed || revision !== saveRevision) return;
      try {
        const saved = await invokeCommand('settings_set_output_devices', { devices: toSave });
        if (disposed || revision !== saveRevision) return;
        s.entries = saved;
        s.failure = null;
      } catch (error) {
        if (!disposed && revision === saveRevision) s.failure = getErrorText(error);
      }
      // Mirrors open in the background; look again once they had a chance to.
      setTimeout(() => void refreshStatus(), 800);
    });
  }

  function scheduleSave(immediate: boolean): void {
    if (saveTimer) clearTimeout(saveTimer);
    if (immediate) {
      saveTimer = null;
      save();
      return;
    }
    saveTimer = setTimeout(() => {
      saveTimer = null;
      save();
    }, SAVE_DELAY_MS);
  }

  function update(index: number, patch: Partial<OutputDeviceEntry>, immediate = false): void {
    s.entries = entriesFromRows(patchRow(rows, index, patch));
    scheduleSave(immediate);
  }

  async function setExclusive(value: boolean): Promise<void> {
    if (!isTauri() || value === s.exclusive) return;
    try {
      // Settle pending edits first; turning exclusive on keeps only the first chosen device.
      if (saveTimer) {
        clearTimeout(saveTimer);
        saveTimer = null;
        save();
      }
      await saveQueue;
      s.exclusive = await invokeCommand('settings_set_output_exclusive', { exclusive: value });
      s.entries = await invokeCommand('settings_get_output_devices', {});
      s.failure = null;
      setTimeout(() => void refreshStatus(), 800);
    } catch (error) {
      s.failure = getErrorText(error);
    }
  }

  function toggle(index: number, enabled: boolean): void {
    if (s.exclusive && enabled) {
      s.entries = entriesFromRows(selectOnly(rows, index));
      scheduleSave(true);
      return;
    }
    s.entries = entriesFromRows(setRowEnabled(rows, index, enabled));
    scheduleSave(true);
  }

  function removeSaved(index: number): void {
    const id = rows[index].id;
    s.entries = s.entries.filter((entry) => entry.id !== id);
    scheduleSave(true);
  }

  function setDelay(index: number, value: number): void {
    if (!Number.isFinite(value)) return;
    update(index, { delayMs: Math.min(OUTPUT_DELAY_MAX_MS, Math.max(0, Math.round(value))) });
  }

  function wheelDelta(event: WheelEvent, step: number): number {
    return (event.deltaY < 0 ? 1 : -1) * (event.shiftKey ? 1 : step);
  }

  // ---- real device volume -------------------------------------------------------------------------------------

  function volumeOf(index: number): { percent: number; muted: boolean } | null {
    const id = volumeDeviceId(rows[index], s.devices);
    return id ? (s.volumes[id] ?? null) : null;
  }

  function sendVolume(id: string): void {
    const percent = volumePending.get(id);
    volumeSendTimers.delete(id);
    if (percent === undefined) return;
    volumePending.delete(id);
    void invokeCommand('audio_set_device_volume', { id, volumePercent: percent }).catch((error) => {
      s.failure = getErrorText(error);
    });
  }

  function queueVolume(id: string, percent: number): void {
    const muted = percent <= 0;
    s.volumes = { ...s.volumes, [id]: { percent, muted } };
    volumeEdited.set(id, Date.now());
    volumePending.set(id, percent);
    if (!volumeSendTimers.has(id)) volumeSendTimers.set(id, setTimeout(() => sendVolume(id), VOLUME_SEND_MS));
  }

  function setVolume(index: number, percent: number): void {
    const id = volumeDeviceId(rows[index], s.devices);
    if (!id) return;
    // With the volume link on, the backend moves the other devices once this one is set.
    queueVolume(id, percent);
  }

  // ---- channel map (one matrix for every playing device) -----------------------------------------------------------

  function currentMap(index: number) {
    return rows[index].entry?.channelMap ?? null;
  }

  function channelCount(index: number): number {
    return Math.max(1, rows[index].channels);
  }

  function deviceLabel(index: number): string {
    const row = rows[index];
    return row.id === null ? (row.name || tr('系統預設')) : row.name || tr('未知的裝置');
  }

  function clickCell(index: number, source: number, output: number): void {
    update(index, { channelMap: toggleCell(currentMap(index), channelCount(index), source, output) }, true);
  }

  function wheelCell(event: WheelEvent, index: number, source: number, output: number): void {
    const next = nudgeCell(currentMap(index), channelCount(index), source, output, wheelDelta(event, 5));
    if (!next) return;
    event.preventDefault();
    update(index, { channelMap: next });
  }

  function silenceOutput(event: MouseEvent, index: number, output: number): void {
    event.preventDefault();
    const map = (currentMap(index) ?? standardRoutes(channelCount(index))).map((route) => ({ ...route }));
    map[output] = { ...map[output], source: null };
    update(index, { channelMap: map }, true);
  }

  /** The grid columns: the source names, then each device's channels (a gap before every device but the first). */
  function matrixColumns(): string {
    const groups = playingRows.map(({ index }, position) =>
      Array.from({ length: channelCount(index) }, (_, channel) =>
        channel === 0 && position > 0 ? `${MATRIX_CELL_PX + MATRIX_GROUP_GAP_PX}px` : `${MATRIX_CELL_PX}px`,
      ).join(' '),
    );
    return `${MATRIX_LABEL_PX}px ${groups.join(' ')}`;
  }

  interface PathInfo {
    text: string;
    note: string | null;
    tone: 'ok' | 'warn' | 'error' | 'idle';
  }

  /** What a card says about its device's current output path. */
  function pathInfo(index: number): PathInfo {
    const row = rows[index];
    if (disconnectedPlayingRows(rows).includes(index)) {
      // The device is gone: Windows moved on to the next one and the player followed it.
      const reason = s.exclusive ? outputFormat?.exclusiveFallback : null;
      return {
        text: defaultDeviceName
          ? tr('已中斷連線，目前改由 {0} 播放', [defaultDeviceName])
          : tr('已中斷連線'),
        note: reason ? tr('獨佔模式無法播放這首：{0}', [reason]) : null,
        tone: 'error',
      };
    }
    if (!rowEnabled(rows, index)) {
      // Every rate the device can be opened at; only its mix rate when it could not be asked (ADR 0020).
      const rates = row.supportedRates.length ? row.supportedRates : [row.sampleRate];
      const format = row.connected && row.sampleRate
        ? formatSupportedFormats(rates, [row.channels])
        : row.connected ? '' : tr('裝置未連線');
      return { text: format, note: null, tone: 'idle' };
    }
    if (s.exclusive) {
      // Exclusive mode opens the device when a track plays; what it opened is in the playback format.
      const fallback = describeExclusiveFallback(outputFormat, statusFor(s.statuses, row.id)?.channels ?? null);
      if (fallback) return { text: fallback.text, note: fallback.notice, tone: 'error' };
      const described = describeExclusive(outputFormat);
      if (!described) return { text: tr('獨佔模式：播放時才會開啟這個裝置'), note: null, tone: 'idle' };
      return { text: described.text, note: described.notice, tone: described.notice ? 'warn' : 'ok' };
    }
    const status = statusFor(s.statuses, row.id);
    if (!status || status.state === 'opening') return { text: tr('正在開啟…'), note: null, tone: 'warn' };
    if (status.state === 'lost') {
      return { text: tr('已中斷，每 5 秒重試'), note: status.message, tone: 'error' };
    }
    if (isMainStream(s.statuses, row.id)) {
      const described = describeOutputFormat(outputFormat, status.channels);
      return {
        text: described.text,
        note: status.message ?? described.notice,
        tone: status.message || described.notice ? 'warn' : 'ok',
      };
    }
    return {
      text: describeMirrorPath(outputFormat, status.deviceRateHz, status.converted, status.channels),
      note: null,
      tone: 'ok',
    };
  }

  async function applyPreset(slot: number): Promise<void> {
    s.selectedSlot = slot;
    if (!s.presets[slot] || !isTauri()) return;
    try {
      const saved = await invokeCommand('settings_apply_output_preset', { slot });
      s.entries = saved;
      s.failure = null;
      setTimeout(() => void refreshStatus(), 800);
    } catch (error) {
      s.failure = getErrorText(error);
    }
  }

  async function savePreset(): Promise<void> {
    if (!isTauri()) return;
    // Settle pending edits first, so the slot gets what is on screen.
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
      save();
    }
    await saveQueue;
    try {
      s.presets = await invokeCommand('settings_save_output_preset', { slot: s.selectedSlot });
      s.failure = null;
    } catch (error) {
      s.failure = getErrorText(error);
    }
  }

  async function clearPreset(): Promise<void> {
    if (!isTauri()) return;
    try {
      s.presets = await invokeCommand('settings_clear_output_preset', { slot: s.selectedSlot });
      s.failure = null;
    } catch (error) {
      s.failure = getErrorText(error);
    }
  }

  onMount(() => {
    const alone = liveInstances.size === 0;
    liveInstances.add(instance);
    if (isTauri()) {
      void listen<DeviceVolumeChanged>('device-volume-changed', (event) => onVolumeChanged(event.payload)).then((unlisten) => {
        if (disposed) unlisten();
        else unlistenVolume = unlisten;
      });
    }
    if (isTauri()) {
      void listen('audio-devices-changed', onDevicesChanged).then((unlisten) => {
        if (disposed) unlisten();
        else unlistenDevices = unlisten;
      });
    }
    // Another copy already read everything and keeps it current; a copy that is alone reads it afresh.
    if (alone || !s.loaded) void load();
    pollTimer = setInterval(() => isPrimary() && void refreshStatus(), STATUS_POLL_MS);
    deviceTimer = setInterval(() => isPrimary() && void pollDevices(), DEVICE_POLL_MS);
    volumeTimer = setInterval(() => isPrimary() && void refreshVolumes(), VOLUME_POLL_MS);
  });

  onDestroy(() => {
    disposed = true;
    liveInstances.delete(instance);
    unlistenVolume?.();
    unlistenDevices?.();
    if (deviceEventTimer) clearTimeout(deviceEventTimer);
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
      save();
    }
    for (const [id, timer] of volumeSendTimers) {
      clearTimeout(timer);
      sendVolume(id);
    }
    if (pollTimer) clearInterval(pollTimer);
    if (volumeTimer) clearInterval(volumeTimer);
    if (deviceTimer) clearInterval(deviceTimer);
  });
</script>

<div class="settings-panel-header">
  <div>
    <h3>{tr("模式")}</h3>
  </div>
</div>

<div class="output-mode" role="group" aria-label={tr('輸出模式')}>
  <button
    type="button"
    class="layout-choice-button"
    class:chosen={!s.exclusive}
    aria-pressed={!s.exclusive}
    onclick={() => void setExclusive(false)}
  >
    <strong>{tr('共享')}</strong>
    <span>{tr('與其他程式一起播放，可以同時輸出到多台裝置，由 Windows 混音。取樣率與裝置不同時，由播放器以 rubato 的 FFT 轉換。')}</span>
  </button>
  <button
    type="button"
    class="layout-choice-button"
    class:chosen={s.exclusive}
    aria-pressed={s.exclusive}
    onclick={() => void setExclusive(true)}
  >
    <strong>{tr('獨佔優先')}</strong>
    <span>{tr('獨佔一台裝置，音訊不經 Windows 混音，取樣率與位元深度照檔案原樣送出（位元完美）。其他程式在這台裝置上暫時沒有聲音，也不能同時輸出到多台；藍牙裝置通常不支援。')}</span>
  </button>
</div>

<div class="settings-panel-header">
  <div>
    <h3>{tr("裝置")}</h3>
    <p>{s.exclusive
      ? tr("獨佔模式只能選一台裝置。選好之後播放時，這台裝置會被播放器獨佔。")
      : tr("勾選要輸出的裝置，可以同時輸出到多台。音量是 Windows 裝置本身的音量，延遲與聲道映射由播放器處理。同步輸出使用 Windows 共享模式，不是 bit-perfect。")}</p>
  </div>
</div>

{#if !s.exclusive}
<div class="output-presets" role="group" aria-label={tr('方案')}>
  <span class="output-presets-label">{tr('方案')}</span>
  {#each s.presets as slot, index (index)}
    <button
      type="button"
      class="soft-button output-preset"
      class:is-active={slot !== null && sameDevices(slot, s.entries)}
      class:is-selected={s.selectedSlot === index}
      class:is-empty={slot === null}
      aria-pressed={s.selectedSlot === index}
      use:appTooltip={slot === null ? tr('空的方案') : tr('{0} 台裝置', [slot.filter((entry) => entry.enabled).length])}
      onclick={() => void applyPreset(index)}
    >{index + 1}</button>
  {/each}
  <button type="button" class="soft-button" use:appTooltip={tr('把目前的設定存到選取的方案（不含音量）')} onclick={() => void savePreset()}>{tr('存入')}</button>
  <button type="button" class="soft-button" disabled={s.presets[s.selectedSlot] === null} use:appTooltip={tr('清除選取的方案')} onclick={() => void clearPreset()}>{tr('清除')}</button>
  {#if playingRows.length > 1}
    <label class="output-sync">
      <input type="checkbox" checked={s.volumeSync} onchange={(event) => void setVolumeSync(event.currentTarget.checked)} />
      <span use:appTooltip={tr('調整其中一台的音量時，其他裝置依原本的比例跟著調整')}>{tr('音量同步')}</span>
    </label>
  {/if}
</div>

{/if}

{#if s.failure}<p class="source-error-message" role="alert">{s.failure}</p>{/if}

<div class="output-device-list" role="list" aria-busy={s.loading} use:scrollChain>
  {#each rows as row, index (row.key)}
    {@const playing = rowEnabled(rows, index)}
    {@const lastPlaying = isLastPlaying(rows, index)}
    {@const entry = row.entry}
    {@const path = pathInfo(index)}
    {@const volume = volumeOf(index)}
    {@const label = row.id === null ? tr('系統預設') : row.name || tr('未知的裝置')}
    <!-- The card is the control: its checkbox is out of sight and keeps the keyboard (Tab, Space, Enter). -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <div
      class="output-device"
      class:is-playing={playing}
      class:is-missing={!row.connected}
      class:is-locked={lastPlaying}
      role="listitem"
      onclick={chooseCardOnClick}
    >
      <div class="output-device-head">
        <label class="output-device-toggle">
          <input
            class="output-device-input"
            type="checkbox"
            checked={playing}
            disabled={lastPlaying}
            aria-label={label}
            onchange={(event) => toggle(index, event.currentTarget.checked)}
            onkeydown={chooseCardOnEnter}
          />
          <span class="output-device-dot is-{path.tone}" aria-hidden="true"></span>
          <span class="output-device-name" use:appTooltip={label}>
            <strong>{label}</strong>
            {#if row.id === null}
              <small>{row.name ? tr('目前是：{0}', [row.name]) : tr('跟隨 Windows 的預設播放裝置')}</small>
            {:else if index === takeover}
              <small class="output-device-takeover">{tr('目前由這台播放（原裝置已中斷）')}</small>
            {:else if row.isDefault}
              <small>{tr('目前的系統預設')}</small>
            {/if}
          </span>
        </label>
        {#if !row.connected}
          <button type="button" class="soft-button" onclick={() => removeSaved(index)}>{tr('移除')}</button>
        {/if}
      </div>

      {#if playing && row.connected}
        <div class="output-device-controls">
          <label class="output-volume">
            <span>{tr('音量')}</span>
            <input
              type="range"
              min="0"
              max="100"
              step="1"
              value={volume?.percent ?? 0}
              disabled={volume === null}
              aria-label={tr('裝置音量')}
              oninput={(event) => setVolume(index, Number(event.currentTarget.value))}
            />
            <output>{volume ? `${volume.percent}%` : '—'}{#if volume?.muted && volume.percent > 0}<small> {tr('靜音')}</small>{/if}</output>
          </label>
          {#if !s.exclusive && delayApplies}
          <div class="output-delay" role="group" aria-label={tr('延遲')}>
            <span>{tr('延遲')}</span>
            <button type="button" class="soft-button icon-step" aria-label={tr('減少 {0} 毫秒', [DELAY_STEP_MS])} onclick={() => setDelay(index, (entry?.delayMs ?? 0) - DELAY_STEP_MS)}>−</button>
            <input
              type="number"
              min="0"
              max={OUTPUT_DELAY_MAX_MS}
              step="1"
              value={entry?.delayMs ?? 0}
              aria-label={tr('延遲（毫秒）')}
              aria-valuetext={formatDelay(entry?.delayMs ?? 0)}
              onchange={(event) => setDelay(index, Number(event.currentTarget.value))}
              onwheel={(event) => { event.preventDefault(); setDelay(index, (entry?.delayMs ?? 0) + wheelDelta(event, DELAY_STEP_MS)); }}
              ondblclick={() => setDelay(index, 0)}
            />
            <span class="output-delay-unit">ms</span>
            <button type="button" class="soft-button icon-step" aria-label={tr('增加 {0} 毫秒', [DELAY_STEP_MS])} onclick={() => setDelay(index, (entry?.delayMs ?? 0) + DELAY_STEP_MS)}>＋</button>
          </div>
          {/if}
        </div>
      {/if}

      {#if path.text || path.note}
        <p class="output-device-path is-{path.tone}">
          {path.text}{#if path.note}<span class="output-device-note"> — {path.note}</span>{/if}
        </p>
      {/if}
    </div>
  {/each}
</div>

{#if playingRows.length > 0 && !s.exclusive}
  {@const rowsShown = matrixRows}
  <section class="channel-matrix-panel" aria-label={tr('聲道映射')}>
    <div class="settings-panel-header">
      <div>
        <h3>{tr('聲道映射')}</h3>
        <p>{tr('所有裝置共用一張表：縱向是曲目的來源聲道，橫向是各裝置的輸出聲道。點格子選擇輸出聲道的來源，滾輪調增益，右鍵讓該輸出靜音；點表頭的裝置名稱恢復自動（依聲道數混縮或補齊）。')}</p>
      </div>
    </div>
    <div class="channel-matrix-scroll">
      <div
        class="channel-matrix-grid"
        style="grid-template-columns: {matrixColumns()};"
      >
        <span class="channel-matrix-corner"></span>
        {#each playingRows as { index }, position (index)}
          <button
            type="button"
            class="channel-matrix-device"
            class:group-start={position > 0}
            style="grid-column: span {channelCount(index)};"
            use:appTooltip={currentMap(index) ? tr('{0}：點一下恢復自動（依聲道數混縮或補齊）', [deviceLabel(index)]) : deviceLabel(index)}
            onclick={() => update(index, { channelMap: null }, true)}
          ><span class="dock-marquee"><span class="dock-marquee-text" use:dockMarquee>{shortDeviceName(deviceLabel(index))}</span></span></button>
        {/each}
        <span class="channel-matrix-corner" use:appTooltip={tr('縱向是來源聲道（input），橫向是這個裝置的輸出聲道（output）')}>i＼o</span>
        {#each playingRows as { index }, position (index)}
          {@const outputLabels = channelLabels(channelCount(index))}
          {#each Array.from({ length: channelCount(index) }, (_, output) => output) as output (output)}
            <span class="channel-matrix-head" class:group-start={output === 0 && position > 0}>{outputLabels[output] || output + 1}</span>
          {/each}
        {/each}
        {#each Array.from({ length: rowsShown }, (_, source) => source) as source (source)}
          <span class="channel-matrix-side">{sourceLabels[source]}</span>
          {#each playingRows as { index }, position (index)}
            {@const map = currentMap(index)}
            {@const shown = map ?? standardRoutes(channelCount(index))}
            {@const outputLabels = channelLabels(channelCount(index))}
            {#each shown as route, output (output)}
              {@const on = route.source === source}
              <button
                type="button"
                class="channel-cell"
                class:is-on={on}
                class:is-auto={map === null}
                class:group-start={output === 0 && position > 0}
                aria-pressed={on}
                aria-label={tr('來源 {0} → {1} 的 {2}', [sourceLabels[source], shortDeviceName(deviceLabel(index)), outputLabels[output] || output + 1])}
                onclick={() => clickCell(index, source, output)}
                onwheel={(event) => wheelCell(event, index, source, output)}
                oncontextmenu={(event) => silenceOutput(event, index, output)}
              >{on ? (route.gainPercent === 100 ? '●' : route.gainPercent) : ''}</button>
            {/each}
          {/each}
        {/each}
      </div>
    </div>
  </section>
{/if}

<style>
  .output-mode {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 8px;
    margin: 0 0 14px;
  }

  .output-presets {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: 0 0 12px;
  }

  .output-presets-label {
    color: var(--text-soft);
    font-size: var(--font-caption);
  }

  .output-preset {
    min-width: 36px;
  }

  .output-preset.is-empty {
    border-style: dashed;
    color: var(--text-soft);
  }

  .output-preset.is-selected {
    border-color: rgba(var(--accent-rgb), 0.64);
  }

  .output-sync {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    font-size: var(--font-caption);
    cursor: pointer;
  }

  .output-device-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 18px;
    padding-left: 28px;
  }

  .output-volume {
    display: flex;
    flex: 1 1 220px;
    align-items: center;
    gap: 8px;
    min-width: 180px;
  }

  .output-volume input[type='range'] {
    flex: 1 1 auto;
    min-width: 0;
  }

  .output-volume output {
    min-width: 48px;
    color: var(--text-soft);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  .output-delay {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .output-delay input[type='number'] {
    width: 64px;
    text-align: right;
  }

  .output-delay-unit {
    color: var(--text-soft);
    font-size: var(--font-caption);
  }

  .icon-step {
    min-width: 30px;
    padding-inline: 0;
  }

  .channel-matrix-panel {
    display: grid;
    gap: 10px;
    margin-top: 18px;
  }

  .channel-matrix-scroll {
    overflow-x: auto;
  }

  .channel-matrix-grid {
    display: grid;
    gap: 4px;
    align-items: center;
    justify-content: start;
  }

  .channel-matrix-corner,
  .channel-matrix-head,
  .channel-matrix-side {
    color: var(--text-soft);
    font-size: var(--font-caption);
    text-align: center;
  }

  .channel-matrix-device {
    overflow: hidden;
    padding: 2px 4px;
    border: 0;
    border-bottom: var(--divider-width, 1px) solid var(--divider-color, var(--line));
    color: inherit;
    background: transparent;
    font: inherit;
    font-size: var(--font-caption);
    font-weight: 600;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .channel-matrix-device:hover {
    color: var(--accent-text);
  }

  .group-start {
    margin-left: 10px;
  }

  .channel-cell {
    height: 28px;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: 6px;
    color: var(--text-soft);
    background: rgba(var(--text-rgb), 0.025);
    font-size: var(--font-caption);
    cursor: pointer;
  }

  .channel-cell:hover {
    border-color: rgba(var(--accent-rgb), 0.64);
  }

  .channel-cell.is-on {
    border-color: rgba(var(--accent-rgb), 0.64);
    color: var(--accent-text);
    background: rgba(var(--accent-rgb), 0.09);
  }

  .channel-cell.is-auto.is-on {
    border-style: dashed;
    opacity: 0.7;
  }

  @media (max-width: 520px) {
    .output-device-controls {
      padding-left: 0;
    }
  }
</style>
