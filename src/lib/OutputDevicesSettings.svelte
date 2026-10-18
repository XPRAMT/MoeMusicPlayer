<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { isTauri } from '@tauri-apps/api/core';
  import { appTooltip } from './app-tooltip.js';
  import { getErrorText, invokeCommand } from './ipc';
  import { tr } from './i18n.js';
  import type {
    AudioOutputDevice,
    AudioOutputDeviceStatus,
    OutputDeviceEntry,
    PlaybackOutputFormat,
  } from './ipc';
  import {
    describeExclusive,
    describeExclusiveFallback,
    describeMirrorPath,
    describeOutputFormat,
    formatSampleRate,
  } from './output-format.js';
  import {
    OUTPUT_DELAY_MAX_MS,
    OUTPUT_MAX_SOURCE_CHANNELS,
    activePreset,
    buildDeviceRows,
    channelLabels,
    entriesFromRows,
    formatDelay,
    isLastPlaying,
    isMainStream,
    nudgeCell,
    patchRow,
    rowEnabled,
    routePresets,
    sameDevices,
    selectOnly,
    setRowEnabled,
    shortDeviceName,
    sourceRowCount,
    standardRoutes,
    statusFor,
    toggleCell,
    volumeDeviceId,
  } from './output-devices.js';
  import { pruneScales, syncVolumes } from './volume-sync.js';

  interface Props {
    /** The player's current sample-rate path (the main stream's). */
    outputFormat?: PlaybackOutputFormat | null;
    /** Told when the exclusive-mode setting is read or changed (other parts of the settings page depend on it). */
    onExclusiveChange?: (exclusive: boolean) => void;
  }

  let { outputFormat = null, onExclusiveChange = () => {} }: Props = $props();

  const SHARED_EVENT = 'moe-output-devices-saved';
  const instance = Symbol('output-devices');
  const SAVE_DELAY_MS = 250;
  const STATUS_POLL_MS = 2000;
  const VOLUME_POLL_MS = 1000;
  const VOLUME_SEND_MS = 60;
  /** A volume the user just changed is not overwritten by a poll for this long. */
  const VOLUME_HOLD_MS = 1500;
  const DELAY_STEP_MS = 10;
  const PRESET_SLOTS = 3;
  const SYNC_KEY = 'moe.outputVolumeSync';

  let devices = $state<AudioOutputDevice[]>([]);
  let entries = $state<OutputDeviceEntry[]>([]);
  let statuses = $state<AudioOutputDeviceStatus[]>([]);
  let presets = $state<(OutputDeviceEntry[] | null)[]>(Array.from({ length: PRESET_SLOTS }, () => null));
  let selectedSlot = $state(0);
  /** The real Windows volume (0–100) and mute state of each device, by device id. */
  let volumes = $state<Record<string, { percent: number; muted: boolean }>>({});
  let volumeSync = $state(readSyncPreference());
  let allSources = $state(false);
  /** WASAPI exclusive mode: one device, bit-perfect, no mixing with other programs. */
  let exclusive = $state(false);
  let loading = $state(true);
  let refreshing = $state(false);
  let failure = $state<string | null>(null);

  let rows = $derived(buildDeviceRows(entries, devices));
  let playingRows = $derived(rows.map((row, index) => ({ row, index })).filter(({ index }) => rowEnabled(rows, index)));
  const sourceLabels = channelLabels(OUTPUT_MAX_SOURCE_CHANNELS);
  /** How many source-channel rows the shared matrix shows: enough for every playing device's map. */
  let matrixRows = $derived(
    sourceRowCount(
      playingRows.flatMap(({ index }) => rows[index].entry?.channelMap ?? []),
      allSources,
    ),
  );

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveRevision = 0;
  let saveQueue: Promise<void> = Promise.resolve();
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let volumeTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  const volumeEdited = new Map<string, number>();
  const volumePending = new Map<string, number>();
  const volumeSendTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const syncScales = new Map<string, number>();

  function readSyncPreference(): boolean {
    try {
      return localStorage.getItem(SYNC_KEY) === '1';
    } catch {
      return false;
    }
  }

  function setVolumeSync(on: boolean): void {
    volumeSync = on;
    syncScales.clear();
    try {
      localStorage.setItem(SYNC_KEY, on ? '1' : '0');
    } catch {
      // The preference just is not remembered.
    }
  }

  async function refreshStatus(): Promise<void> {
    try {
      const next = await invokeCommand('audio_output_device_status', {});
      if (!disposed) statuses = next;
    } catch {
      // The status is a hint only.
    }
  }

  async function refreshVolumes(): Promise<void> {
    if (!isTauri()) return;
    try {
      const listed = await invokeCommand('audio_get_device_volumes', {});
      if (disposed) return;
      const now = Date.now();
      const next = { ...volumes };
      for (const item of listed) {
        // Keep what the user is dragging; take everything else (the real volume moves from outside too).
        if (now - (volumeEdited.get(item.id) ?? 0) < VOLUME_HOLD_MS) continue;
        next[item.id] = { percent: Math.round(item.volumePercent), muted: item.muted };
      }
      volumes = next;
    } catch {
      // Devices come and go; the next poll tries again.
    }
  }

  async function refreshDevices(): Promise<void> {
    refreshing = true;
    try {
      const next = await invokeCommand('audio_list_output_devices', {});
      if (!disposed) {
        devices = next;
        failure = null;
      }
      void refreshVolumes();
    } catch (error) {
      if (!disposed) failure = tr('無法列出播放裝置：{0}', [getErrorText(error)]);
    } finally {
      refreshing = false;
    }
  }

  async function load(): Promise<void> {
    if (!isTauri()) {
      loading = false;
      failure = tr('瀏覽器預覽不會列出播放裝置。');
      return;
    }
    try {
      const [saved, listed, savedPresets, savedExclusive] = await Promise.all([
        invokeCommand('settings_get_output_devices', {}),
        invokeCommand('audio_list_output_devices', {}),
        invokeCommand('settings_get_output_presets', {}),
        invokeCommand('settings_get_output_exclusive', {}),
      ]);
      if (disposed) return;
      exclusive = savedExclusive;
      onExclusiveChange(savedExclusive);
      entries = saved;
      devices = listed;
      presets = savedPresets;
      selectedSlot = Math.max(0, savedPresets.findIndex((slot) => slot && sameDevices(slot, saved)));
      failure = null;
    } catch (error) {
      if (!disposed) failure = tr('無法讀取播放裝置設定：{0}', [getErrorText(error)]);
    } finally {
      loading = false;
    }
    void refreshStatus();
    void refreshVolumes();
  }

  function broadcast(saved: OutputDeviceEntry[]): void {
    // The quick-settings drawer and the settings page each mount this list; keep them in step.
    window.dispatchEvent(new CustomEvent(SHARED_EVENT, { detail: { source: instance, entries: saved } }));
  }

  function save(): void {
    if (!isTauri()) return;
    const revision = ++saveRevision;
    const toSave = entries;
    saveQueue = saveQueue.catch(() => undefined).then(async () => {
      if (disposed || revision !== saveRevision) return;
      try {
        const saved = await invokeCommand('settings_set_output_devices', { devices: toSave });
        if (disposed || revision !== saveRevision) return;
        entries = saved;
        failure = null;
        broadcast(saved);
      } catch (error) {
        if (!disposed && revision === saveRevision) failure = getErrorText(error);
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
    entries = entriesFromRows(patchRow(rows, index, patch));
    scheduleSave(immediate);
  }

  async function setExclusive(value: boolean): Promise<void> {
    if (!isTauri() || value === exclusive) return;
    try {
      // Settle pending edits first; turning exclusive on keeps only the first chosen device.
      if (saveTimer) {
        clearTimeout(saveTimer);
        saveTimer = null;
        save();
      }
      await saveQueue;
      exclusive = await invokeCommand('settings_set_output_exclusive', { exclusive: value });
      onExclusiveChange(exclusive);
      entries = await invokeCommand('settings_get_output_devices', {});
      failure = null;
      broadcast(entries);
      setTimeout(() => void refreshStatus(), 800);
    } catch (error) {
      failure = getErrorText(error);
    }
  }

  function toggle(index: number, enabled: boolean): void {
    if (exclusive && enabled) {
      entries = entriesFromRows(selectOnly(rows, index));
      scheduleSave(true);
      return;
    }
    entries = entriesFromRows(setRowEnabled(rows, index, enabled));
    syncScales.clear();
    scheduleSave(true);
  }

  function removeSaved(index: number): void {
    const id = rows[index].id;
    entries = entries.filter((entry) => entry.id !== id);
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
    const id = volumeDeviceId(rows[index], devices);
    return id ? (volumes[id] ?? null) : null;
  }

  function sendVolume(id: string): void {
    const percent = volumePending.get(id);
    volumeSendTimers.delete(id);
    if (percent === undefined) return;
    volumePending.delete(id);
    void invokeCommand('audio_set_device_volume', { id, volumePercent: percent }).catch((error) => {
      failure = getErrorText(error);
    });
  }

  function queueVolume(id: string, percent: number): void {
    const muted = percent <= 0;
    volumes = { ...volumes, [id]: { percent, muted } };
    volumeEdited.set(id, Date.now());
    volumePending.set(id, percent);
    if (!volumeSendTimers.has(id)) volumeSendTimers.set(id, setTimeout(() => sendVolume(id), VOLUME_SEND_MS));
  }

  function setVolume(index: number, percent: number): void {
    const id = volumeDeviceId(rows[index], devices);
    if (!id) return;
    const before = new Map(Object.entries(volumes).map(([key, value]) => [key, value.percent]));
    queueVolume(id, percent);
    if (!volumeSync) return;
    const ids = playingRows.map(({ row }) => volumeDeviceId(row, devices)).filter((value): value is string => value !== null);
    pruneScales(syncScales, ids);
    for (const [other, value] of syncVolumes({ changedId: id, next: percent, volumes: before, ids, scales: syncScales })) {
      queueVolume(other, value);
    }
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

  function presetLabel(id: string): string {
    switch (id) {
      case 'swap': return tr('左右對調');
      case 'left': return tr('只用左聲道');
      case 'right': return tr('只用右聲道');
      default: return tr('標準');
    }
  }

  interface PathInfo {
    text: string;
    note: string | null;
    tone: 'ok' | 'warn' | 'error' | 'idle';
  }

  /** What a card says about its device's current output path. */
  function pathInfo(index: number): PathInfo {
    const row = rows[index];
    if (!rowEnabled(rows, index)) {
      const format = row.connected && row.sampleRate
        ? tr('預設格式 {0}．{1} 聲道', [formatSampleRate(row.sampleRate), row.channels])
        : row.connected ? '' : tr('裝置未連線');
      return { text: format, note: null, tone: 'idle' };
    }
    if (exclusive) {
      // Exclusive mode opens the device when a track plays; what it opened is in the playback format.
      const fallback = describeExclusiveFallback(outputFormat);
      if (fallback) return { text: fallback.text, note: fallback.notice, tone: 'error' };
      const described = describeExclusive(outputFormat);
      if (!described) return { text: tr('獨佔模式：播放時才會開啟這個裝置'), note: null, tone: 'idle' };
      return { text: described.text, note: described.notice, tone: described.notice ? 'warn' : 'ok' };
    }
    const status = statusFor(statuses, row.id);
    if (!status || status.state === 'opening') return { text: tr('正在開啟…'), note: null, tone: 'warn' };
    if (status.state === 'lost') {
      return { text: tr('已中斷，每 5 秒重試'), note: status.message, tone: 'error' };
    }
    const channels = status.channels ? tr('{0} 聲道', [status.channels]) : '';
    if (isMainStream(statuses, row.id)) {
      const described = describeOutputFormat(outputFormat);
      return {
        text: [described.text, channels].filter(Boolean).join('．'),
        note: status.message ?? described.notice,
        tone: status.message || described.notice ? 'warn' : 'ok',
      };
    }
    return {
      text: [describeMirrorPath(outputFormat, status.deviceRateHz, status.converted), channels].filter(Boolean).join('．'),
      note: null,
      tone: 'ok',
    };
  }

  async function applyPreset(slot: number): Promise<void> {
    selectedSlot = slot;
    if (!presets[slot] || !isTauri()) return;
    try {
      const saved = await invokeCommand('settings_apply_output_preset', { slot });
      entries = saved;
      failure = null;
      broadcast(saved);
      setTimeout(() => void refreshStatus(), 800);
    } catch (error) {
      failure = getErrorText(error);
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
      presets = await invokeCommand('settings_save_output_preset', { slot: selectedSlot });
      failure = null;
    } catch (error) {
      failure = getErrorText(error);
    }
  }

  async function clearPreset(): Promise<void> {
    if (!isTauri()) return;
    try {
      presets = await invokeCommand('settings_clear_output_preset', { slot: selectedSlot });
      failure = null;
    } catch (error) {
      failure = getErrorText(error);
    }
  }

  function adoptShared(event: Event): void {
    const detail = (event as CustomEvent<{ source: symbol; entries: OutputDeviceEntry[] }>).detail;
    if (detail.source !== instance && !saveTimer) entries = detail.entries;
  }

  onMount(() => {
    window.addEventListener(SHARED_EVENT, adoptShared);
    void load();
    pollTimer = setInterval(() => void refreshStatus(), STATUS_POLL_MS);
    volumeTimer = setInterval(() => void refreshVolumes(), VOLUME_POLL_MS);
  });

  onDestroy(() => {
    disposed = true;
    window.removeEventListener(SHARED_EVENT, adoptShared);
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
  });
</script>

<div class="settings-panel-header">
  <div>
    <h3>{tr("播放裝置")}</h3>
    <p>{exclusive
      ? tr("獨佔模式只能選一台裝置。選好之後播放時，這台裝置會被播放器獨佔。")
      : tr("勾選要輸出的裝置，可以同時輸出到多台。音量是 Windows 裝置本身的音量，延遲與聲道映射由播放器處理。同步輸出使用 Windows 共享模式，不是 bit-perfect。")}</p>
  </div>
  <button type="button" class="soft-button" disabled={refreshing || loading} onclick={() => void refreshDevices()}>{tr("重新整理")}</button>
</div>

<div class="output-mode" role="group" aria-label={tr('輸出模式')}>
  <button
    type="button"
    class="layout-choice-button"
    class:chosen={!exclusive}
    aria-pressed={!exclusive}
    onclick={() => void setExclusive(false)}
  >
    <strong>{tr('共享')}</strong>
    <span>{tr('與其他程式一起播放，可以同時輸出到多台裝置，由 Windows 混音。')}</span>
  </button>
  <button
    type="button"
    class="layout-choice-button"
    class:chosen={exclusive}
    aria-pressed={exclusive}
    onclick={() => void setExclusive(true)}
  >
    <strong>{tr('獨佔')}</strong>
    <span>{tr('獨佔一台裝置，音訊不經 Windows 混音，取樣率與位元深度照檔案原樣送出（位元完美）。其他程式在這台裝置上暫時沒有聲音，也不能同時輸出到多台；藍牙裝置通常不支援。')}</span>
  </button>
</div>

{#if !exclusive}
<div class="output-presets" role="group" aria-label={tr('方案')}>
  <span class="output-presets-label">{tr('方案')}</span>
  {#each presets as slot, index (index)}
    <button
      type="button"
      class="soft-button output-preset"
      class:is-active={slot !== null && sameDevices(slot, entries)}
      class:is-selected={selectedSlot === index}
      class:is-empty={slot === null}
      aria-pressed={selectedSlot === index}
      use:appTooltip={slot === null ? tr('空的方案') : tr('{0} 台裝置', [slot.filter((entry) => entry.enabled).length])}
      onclick={() => void applyPreset(index)}
    >{index + 1}</button>
  {/each}
  <button type="button" class="soft-button" use:appTooltip={tr('把目前的設定存到選取的方案（不含音量）')} onclick={() => void savePreset()}>{tr('存入')}</button>
  <button type="button" class="soft-button" disabled={presets[selectedSlot] === null} use:appTooltip={tr('清除選取的方案')} onclick={() => void clearPreset()}>{tr('清除')}</button>
  {#if playingRows.length > 1}
    <label class="output-sync">
      <input type="checkbox" checked={volumeSync} onchange={(event) => setVolumeSync(event.currentTarget.checked)} />
      <span use:appTooltip={tr('調整其中一台的音量時，其他裝置依原本的比例跟著調整')}>{tr('音量同步')}</span>
    </label>
  {/if}
</div>

{/if}

{#if failure}<p class="source-error-message" role="alert">{failure}</p>{/if}

<div class="output-device-list" role="list" aria-busy={loading}>
  {#each rows as row, index (row.key)}
    {@const playing = rowEnabled(rows, index)}
    {@const lastPlaying = isLastPlaying(rows, index)}
    {@const entry = row.entry}
    {@const path = pathInfo(index)}
    {@const volume = volumeOf(index)}
    {@const label = row.id === null ? tr('系統預設') : row.name || tr('未知的裝置')}
    <div class="output-device" class:is-playing={playing} class:is-missing={!row.connected} role="listitem">
      <div class="output-device-head">
        <label class="output-device-toggle">
          <input
            type="checkbox"
            checked={playing}
            disabled={lastPlaying}
            aria-label={label}
            onchange={(event) => toggle(index, event.currentTarget.checked)}
          />
          <span class="output-device-dot is-{path.tone}" aria-hidden="true"></span>
          <span class="output-device-name" use:appTooltip={label}>
            <strong>{label}</strong>
            {#if row.id === null}
              <small>{row.name ? tr('目前是：{0}', [row.name]) : tr('跟隨 Windows 的預設播放裝置')}</small>
            {:else if row.isDefault}
              <small>{tr('目前的系統預設')}</small>
            {/if}
          </span>
        </label>
        {#if !row.connected}
          <button type="button" class="soft-button" onclick={() => removeSaved(index)}>{tr('移除')}</button>
        {/if}
      </div>

      {#if playing && !exclusive}
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

{#if playingRows.length > 0 && !exclusive}
  {@const rowsShown = matrixRows}
  <section class="channel-matrix-panel" aria-label={tr('聲道映射')}>
    <div class="settings-panel-header">
      <div>
        <h3>{tr('聲道映射')}</h3>
        <p>{tr('所有裝置共用一張表：縱向是曲目的來源聲道，橫向是各裝置的輸出聲道。點格子選擇輸出聲道的來源，滾輪調增益，右鍵讓該輸出靜音；自動的裝置依聲道數混縮或補齊。')}</p>
      </div>
    </div>
    <div class="channel-matrix-scroll">
      <div
        class="channel-matrix-grid"
        style="grid-template-columns: 44px {playingRows.map(({ index }) => `repeat(${channelCount(index)}, 38px)`).join(' ')};"
      >
        <span class="channel-matrix-corner"></span>
        {#each playingRows as { index } (index)}
          <span class="channel-matrix-device" style="grid-column: span {channelCount(index)};" use:appTooltip={deviceLabel(index)}>{shortDeviceName(deviceLabel(index))}</span>
        {/each}
        <span class="channel-matrix-corner"></span>
        {#each playingRows as { index } (index)}
          {@const outputLabels = channelLabels(channelCount(index))}
          {#each Array.from({ length: channelCount(index) }, (_, output) => output) as output (output)}
            <span class="channel-matrix-head" class:group-start={output === 0}>{outputLabels[output] || output + 1}</span>
          {/each}
        {/each}
        {#each Array.from({ length: rowsShown }, (_, source) => source) as source (source)}
          <span class="channel-matrix-side">{sourceLabels[source]}</span>
          {#each playingRows as { index } (index)}
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
                class:group-start={output === 0}
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
    <div class="channel-matrix-actions">
      {#each playingRows as { index } (index)}
        {@const map = currentMap(index)}
        {@const channels = channelCount(index)}
        <div class="channel-matrix-device-actions">
          <span class="channel-matrix-device-name" use:appTooltip={deviceLabel(index)}>{shortDeviceName(deviceLabel(index))}</span>
          <button type="button" class="soft-button" class:is-active={map === null} aria-pressed={map === null} onclick={() => update(index, { channelMap: null }, true)}>{tr('自動')}</button>
          {#each routePresets(channels) as preset (preset.id)}
            <button
              type="button"
              class="soft-button"
              class:is-active={activePreset(channels, map) === preset.id}
              aria-pressed={activePreset(channels, map) === preset.id}
              onclick={() => update(index, { channelMap: preset.routes.map((route) => ({ ...route })) }, true)}
            >{presetLabel(preset.id)}</button>
          {/each}
        </div>
      {/each}
      <button type="button" class="soft-button" onclick={() => (allSources = !allSources)}>
        {allSources ? tr('只顯示前幾個來源聲道') : tr('顯示全部來源聲道')}
      </button>
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
    outline: 1px solid color-mix(in srgb, var(--accent-text) 55%, transparent);
  }

  .output-sync {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-left: auto;
    font-size: var(--font-caption);
    cursor: pointer;
  }

  .output-device-list {
    display: grid;
    gap: 8px;
  }

  .output-device {
    display: grid;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 10px;
    background: color-mix(in srgb, var(--panel) 70%, transparent);
  }

  .output-device.is-playing {
    border-color: color-mix(in srgb, var(--accent-text) 45%, var(--line));
  }

  .output-device.is-missing .output-device-name {
    opacity: 0.6;
  }

  .output-device-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .output-device-toggle {
    display: flex;
    flex: 1 1 auto;
    align-items: center;
    gap: 10px;
    min-width: 0;
    cursor: pointer;
  }

  .output-device-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--line);
  }

  .output-device-dot.is-ok { background: var(--status-success); }
  .output-device-dot.is-warn { background: var(--status-warning); }
  .output-device-dot.is-error { background: var(--status-danger); }

  .output-device-name {
    display: grid;
    min-width: 0;
  }

  .output-device-name strong,
  .output-device-name small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .output-device-name small {
    color: var(--text-soft);
    font-size: var(--font-caption);
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

  .output-device-path {
    margin: 0;
    padding-left: 28px;
    color: var(--text-soft);
    font-size: var(--font-caption);
    overflow-wrap: anywhere;
  }

  .output-device-path.is-warn { color: var(--status-warning); }
  .output-device-path.is-error { color: var(--status-danger); }

  .output-device-note {
    opacity: 0.9;
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
    border-bottom: var(--divider-width, 1px) solid var(--divider-color, var(--line));
    font-size: var(--font-caption);
    font-weight: 600;
    text-align: center;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    background: transparent;
    font-size: var(--font-caption);
    cursor: pointer;
  }

  .channel-cell:hover {
    border-color: color-mix(in srgb, var(--accent-text) 55%, var(--line));
  }

  .channel-cell.is-on {
    border-color: color-mix(in srgb, var(--accent-text) 60%, var(--line));
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent-text) 14%, transparent);
  }

  .channel-cell.is-auto.is-on {
    border-style: dashed;
    opacity: 0.7;
  }

  .channel-matrix-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 18px;
  }

  .channel-matrix-device-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .channel-matrix-device-name {
    max-width: 160px;
    overflow: hidden;
    color: var(--text-soft);
    font-size: var(--font-caption);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 520px) {
    .output-device-controls,
    .output-device-path {
      padding-left: 0;
    }
  }
</style>
