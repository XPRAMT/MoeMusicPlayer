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
  import { describeMirrorPath, describeOutputFormat, formatSampleRate } from './output-format.js';
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
    setRowEnabled,
    sourceRowCount,
    standardRoutes,
    statusFor,
    toggleCell,
  } from './output-devices.js';

  interface Props {
    /** The player's current sample-rate path (the main stream's). */
    outputFormat?: PlaybackOutputFormat | null;
  }

  let { outputFormat = null }: Props = $props();

  const SHARED_EVENT = 'moe-output-devices-saved';
  const instance = Symbol('output-devices');
  const SAVE_DELAY_MS = 250;
  const STATUS_POLL_MS = 2000;
  const DELAY_STEP_MS = 10;
  const PRESET_SLOTS = 3;

  let devices = $state<AudioOutputDevice[]>([]);
  let entries = $state<OutputDeviceEntry[]>([]);
  let statuses = $state<AudioOutputDeviceStatus[]>([]);
  let presets = $state<(OutputDeviceEntry[] | null)[]>(Array.from({ length: PRESET_SLOTS }, () => null));
  let selectedSlot = $state(0);
  let matrixOpen = $state<Set<string>>(new Set());
  let allSources = $state<Set<string>>(new Set());
  let loading = $state(true);
  let refreshing = $state(false);
  let failure = $state<string | null>(null);

  let rows = $derived(buildDeviceRows(entries, devices));
  const sourceLabels = channelLabels(OUTPUT_MAX_SOURCE_CHANNELS);

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveRevision = 0;
  let saveQueue: Promise<void> = Promise.resolve();
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  async function refreshStatus(): Promise<void> {
    try {
      const next = await invokeCommand('audio_output_device_status', {});
      if (!disposed) statuses = next;
    } catch {
      // The status is a hint only.
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
      const [saved, listed, savedPresets] = await Promise.all([
        invokeCommand('settings_get_output_devices', {}),
        invokeCommand('audio_list_output_devices', {}),
        invokeCommand('settings_get_output_presets', {}),
      ]);
      if (disposed) return;
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

  function toggle(index: number, enabled: boolean): void {
    entries = entriesFromRows(setRowEnabled(rows, index, enabled));
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

  function currentMap(index: number) {
    return rows[index].entry?.channelMap ?? null;
  }

  function channelCount(index: number): number {
    return Math.max(1, rows[index].channels);
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

  function setMember(set: Set<string>, key: string, on: boolean): Set<string> {
    const next = new Set(set);
    if (on) next.add(key);
    else next.delete(key);
    return next;
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
  });

  onDestroy(() => {
    disposed = true;
    window.removeEventListener(SHARED_EVENT, adoptShared);
    if (saveTimer) {
      clearTimeout(saveTimer);
      saveTimer = null;
      save();
    }
    if (pollTimer) clearInterval(pollTimer);
  });
</script>

<div class="settings-panel-header">
  <div>
    <h3>{tr("播放裝置")}</h3>
    <p>{tr("勾選要輸出的裝置，可以同時輸出到多台，每台各自有音量、延遲與聲道設定。同步輸出使用 Windows 共享模式，不是 bit-perfect。")}</p>
  </div>
  <button type="button" class="soft-button" disabled={refreshing || loading} onclick={() => void refreshDevices()}>{tr("重新整理")}</button>
</div>

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
  <button type="button" class="soft-button" use:appTooltip={tr('把目前的設定存到選取的方案')} onclick={() => void savePreset()}>{tr('存入')}</button>
  <button type="button" class="soft-button" disabled={presets[selectedSlot] === null} use:appTooltip={tr('清除選取的方案')} onclick={() => void clearPreset()}>{tr('清除')}</button>
</div>

{#if failure}<p class="source-error-message" role="alert">{failure}</p>{/if}

<div class="output-device-list" role="list" aria-busy={loading}>
  {#each rows as row, index (row.key)}
    {@const playing = rowEnabled(rows, index)}
    {@const lastPlaying = isLastPlaying(rows, index)}
    {@const entry = row.entry}
    {@const path = pathInfo(index)}
    {@const channels = channelCount(index)}
    {@const map = currentMap(index)}
    {@const matrix = matrixOpen.has(row.key)}
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

      {#if playing}
        <div class="output-device-controls">
          <label class="output-volume">
            <span>{tr('音量')}</span>
            <input
              type="range"
              min="0"
              max="100"
              step="1"
              value={entry?.volumePercent ?? 100}
              aria-label={tr('音量')}
              oninput={(event) => update(index, { volumePercent: Number(event.currentTarget.value) })}
              ondblclick={() => update(index, { volumePercent: 100 })}
            />
            <output>{entry?.volumePercent ?? 100}%</output>
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
          <button
            type="button"
            class="soft-button"
            class:is-active={matrix}
            aria-expanded={matrix}
            onclick={() => (matrixOpen = setMember(matrixOpen, row.key, !matrix))}
          >{map ? tr('聲道：自訂') : tr('聲道：自動')}</button>
        </div>
      {/if}

      {#if path.text || path.note}
        <p class="output-device-path is-{path.tone}">
          {path.text}{#if path.note}<span class="output-device-note"> — {path.note}</span>{/if}
        </p>
      {/if}

      {#if playing && matrix}
        {@const shown = map ?? standardRoutes(channels)}
        {@const rowsShown = sourceRowCount(map, allSources.has(row.key))}
        {@const outputLabels = channelLabels(channels)}
        <div class="channel-matrix" class:is-auto={map === null} role="group" aria-label={tr('聲道映射')}>
          <div class="channel-matrix-grid" style="grid-template-columns: 44px repeat({channels}, 38px);">
            <span class="channel-matrix-corner" use:appTooltip={tr('縱向是來源聲道，橫向是這個裝置的輸出聲道')}>↓ →</span>
            {#each shown as _, output (output)}
              <span class="channel-matrix-head">{outputLabels[output] || output + 1}</span>
            {/each}
            {#each Array.from({ length: rowsShown }, (_, source) => source) as source (source)}
              <span class="channel-matrix-side">{sourceLabels[source]}</span>
              {#each shown as route, output (output)}
                {@const on = route.source === source}
                <button
                  type="button"
                  class="channel-cell"
                  class:is-on={on}
                  aria-pressed={on}
                  aria-label={tr('來源 {0} → 輸出 {1}', [sourceLabels[source], outputLabels[output] || output + 1])}
                  onclick={() => clickCell(index, source, output)}
                  onwheel={(event) => wheelCell(event, index, source, output)}
                  oncontextmenu={(event) => silenceOutput(event, index, output)}
                >{on ? (route.gainPercent === 100 ? '●' : route.gainPercent) : ''}</button>
              {/each}
            {/each}
          </div>
          <div class="channel-matrix-actions">
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
            <button type="button" class="soft-button" onclick={() => (allSources = setMember(allSources, row.key, !allSources.has(row.key)))}>
              {allSources.has(row.key) ? tr('只顯示前幾個來源聲道') : tr('顯示全部來源聲道')}
            </button>
          </div>
          <p class="channel-matrix-hint">{map === null
            ? tr('目前是自動：依裝置的聲道數混縮或補齊。點一格就改成自訂。')
            : tr('點格子選擇來源，滾輪調增益，右鍵讓該輸出靜音。')}</p>
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
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
    min-width: 40px;
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

  .channel-matrix {
    display: grid;
    gap: 8px;
    margin-left: 28px;
    padding-top: 8px;
    border-top: var(--divider-width, 1px) solid var(--divider-color, var(--line));
  }

  .channel-matrix-grid {
    display: grid;
    gap: 4px;
    align-items: center;
    justify-content: start;
    overflow-x: auto;
  }

  .channel-matrix-corner,
  .channel-matrix-head,
  .channel-matrix-side {
    color: var(--text-soft);
    font-size: var(--font-caption);
    text-align: center;
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

  .channel-matrix.is-auto .channel-cell.is-on {
    border-style: dashed;
    opacity: 0.7;
  }

  .channel-matrix-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .channel-matrix-hint {
    margin: 0;
    color: var(--text-soft);
    font-size: var(--font-caption);
  }

  @media (max-width: 520px) {
    .output-device-controls,
    .output-device-path,
    .channel-matrix {
      padding-left: 0;
      margin-left: 0;
    }
  }
</style>
