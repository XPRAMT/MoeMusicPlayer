<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { isTauri } from '@tauri-apps/api/core';
  import { getErrorText, invokeCommand } from './ipc';
  import { tr } from './i18n.js';
  import type { AudioOutputDevice, OutputDeviceEntry } from './ipc';
  import {
    OUTPUT_DELAY_MAX_MS,
    OUTPUT_DELAY_STEP_MS,
    OUTPUT_MAX_SOURCE_CHANNELS,
    activePreset,
    buildDeviceRows,
    channelLabels,
    entriesFromRows,
    formatDelay,
    isLastPlaying,
    mainRowIndex,
    patchRow,
    setRowEnabled,
    rowEnabled,
    routePresets,
    standardRoutes,
  } from './output-devices.js';

  const SHARED_EVENT = 'moe-output-devices-saved';
  const instance = Symbol('output-devices');
  const SAVE_DELAY_MS = 250;
  const ERROR_POLL_MS = 3000;

  let devices = $state<AudioOutputDevice[]>([]);
  let entries = $state<OutputDeviceEntry[]>([]);
  let errors = $state<{ id: string | null; message: string }[]>([]);
  let expandedKey = $state<string | null>(null);
  let loading = $state(true);
  let refreshing = $state(false);
  let failure = $state<string | null>(null);

  let rows = $derived(buildDeviceRows(entries, devices));
  let mainIndex = $derived(mainRowIndex(rows));
  const sourceLabels = channelLabels(OUTPUT_MAX_SOURCE_CHANNELS);

  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveRevision = 0;
  let saveQueue: Promise<void> = Promise.resolve();
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  async function refreshErrors(): Promise<void> {
    try {
      const next = await invokeCommand('audio_output_device_errors', {});
      if (!disposed) errors = next;
    } catch {
      // The list of problems is a hint only.
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
      const [saved, listed] = await Promise.all([
        invokeCommand('settings_get_output_devices', {}),
        invokeCommand('audio_list_output_devices', {}),
      ]);
      if (disposed) return;
      entries = saved;
      devices = listed;
      failure = null;
    } catch (error) {
      if (!disposed) failure = tr('無法讀取播放裝置設定：{0}', [getErrorText(error)]);
    } finally {
      loading = false;
    }
    void refreshErrors();
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
        // The quick-settings drawer and the settings page each mount this list; keep them in step.
        window.dispatchEvent(new CustomEvent(SHARED_EVENT, { detail: { source: instance, entries: saved } }));
      } catch (error) {
        if (!disposed && revision === saveRevision) failure = getErrorText(error);
      }
      // Mirrors open in the background; look again once they had a chance to.
      setTimeout(() => void refreshErrors(), 800);
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
    if (expandedKey === rows[index].key) expandedKey = null;
    scheduleSave(true);
  }

  function channelCount(index: number): number {
    return Math.max(1, rows[index].channels);
  }

  /** The map being edited: the saved one, or the standard map when the device was on automatic. */
  function currentMap(index: number) {
    return rows[index].entry?.channelMap ?? standardRoutes(channelCount(index));
  }

  function setRoute(index: number, channel: number, patch: { source?: number | null; gainPercent?: number }): void {
    const map = currentMap(index).map((route) => ({ ...route }));
    while (map.length < channelCount(index)) map.push({ source: null, gainPercent: 100 });
    map[channel] = { ...map[channel], ...patch };
    update(index, { channelMap: map });
  }

  function errorFor(index: number): string | null {
    const id = rows[index].id;
    return errors.find((error) => error.id === id)?.message ?? null;
  }

  function presetLabel(id: string): string {
    switch (id) {
      case 'swap': return tr('左右對調');
      case 'left': return tr('只用左聲道');
      case 'right': return tr('只用右聲道');
      default: return tr('標準');
    }
  }

  function sourceOption(channel: number): string {
    const label = sourceLabels[channel];
    return label ? tr('聲道 {0}（{1}）', [channel + 1, label]) : tr('聲道 {0}', [channel + 1]);
  }

  function adoptShared(event: Event): void {
    const detail = (event as CustomEvent<{ source: symbol; entries: OutputDeviceEntry[] }>).detail;
    if (detail.source !== instance && !saveTimer) entries = detail.entries;
  }

  onMount(() => {
    window.addEventListener(SHARED_EVENT, adoptShared);
    void load();
    pollTimer = setInterval(() => void refreshErrors(), ERROR_POLL_MS);
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
    <p>{tr("勾選要輸出的裝置，可以同時輸出到多台。第一台勾選的裝置是主輸出，其餘裝置同步播放同一首歌，各自有音量、延遲與聲道設定。同步輸出使用 Windows 共享模式，不是 bit-perfect。")}</p>
  </div>
  <button type="button" class="soft-button" disabled={refreshing || loading} onclick={() => void refreshDevices()}>{tr("重新整理")}</button>
</div>

{#if failure}<p class="source-error-message" role="alert">{failure}</p>{/if}

<div class="output-device-list" role="list" aria-busy={loading}>
  {#each rows as row, index (row.key)}
    {@const playing = rowEnabled(rows, index)}
    {@const lastPlaying = isLastPlaying(rows, index)}
    {@const problem = errorFor(index)}
    {@const expanded = expandedKey === row.key}
    {@const channels = channelCount(index)}
    {@const entry = row.entry}
    {@const mapMode = entry?.channelMap ? 'custom' : 'auto'}
    <div class="output-device" class:is-playing={playing} class:is-missing={!row.connected} role="listitem">
      <div class="output-device-head">
        <label class="output-device-toggle">
          <input
            type="checkbox"
            checked={playing}
            disabled={lastPlaying}
            aria-label={row.id === null ? tr('系統預設') : row.name}
            onchange={(event) => toggle(index, event.currentTarget.checked)}
          />
          <span class="output-device-name">
            <strong>{row.id === null ? tr('系統預設') : row.name || tr('未知的裝置')}</strong>
            {#if row.id === null}
              <small>{row.name ? tr('目前是：{0}', [row.name]) : tr('跟隨 Windows 的預設播放裝置')}</small>
            {:else if row.isDefault}
              <small>{tr('目前的系統預設')}</small>
            {:else if !row.connected}
              <small>{tr('未連線')}</small>
            {/if}
          </span>
        </label>
        {#if playing}
          <span class="output-device-role" class:is-main={index === mainIndex}>{index === mainIndex ? tr('主輸出') : tr('同步輸出')}</span>
        {/if}
        {#if !row.connected}
          <button type="button" class="soft-button" onclick={() => removeSaved(index)}>{tr('移除')}</button>
        {/if}
        <button
          type="button"
          class="soft-button"
          aria-expanded={expanded}
          onclick={() => (expandedKey = expanded ? null : row.key)}
        >{tr('詳細設定')}</button>
      </div>
      {#if problem}<p class="source-error-message" role="alert">{problem}</p>{/if}
      {#if expanded}
        <div class="output-device-details" role="group" aria-label={tr('{0} 的詳細設定', [row.id === null ? tr('系統預設') : row.name])}>
          <div class="dsd-setting-row">
            <span class="dsd-setting-label"><strong>{tr('音量')}</strong><output>{entry?.volumePercent ?? 100}%</output></span>
            <input
              class="dsd-gain-slider"
              type="range"
              min="0"
              max="100"
              step="1"
              value={entry?.volumePercent ?? 100}
              aria-label={tr('音量')}
              oninput={(event) => update(index, { volumePercent: Number(event.currentTarget.value) })}
            />
          </div>
          <div class="dsd-setting-row">
            <span class="dsd-setting-label"><strong>{tr('延遲')}</strong><output>{formatDelay(entry?.delayMs ?? 0)}</output></span>
            <div class="output-delay-controls">
              <input
                class="dsd-gain-slider"
                type="range"
                min="0"
                max={OUTPUT_DELAY_MAX_MS}
                step={OUTPUT_DELAY_STEP_MS}
                value={entry?.delayMs ?? 0}
                aria-label={tr('延遲')}
                aria-valuetext={formatDelay(entry?.delayMs ?? 0)}
                oninput={(event) => update(index, { delayMs: Number(event.currentTarget.value) })}
              />
              <input
                class="output-delay-number"
                type="number"
                min="0"
                max={OUTPUT_DELAY_MAX_MS}
                step="1"
                value={entry?.delayMs ?? 0}
                aria-label={tr('延遲（毫秒）')}
                onchange={(event) => update(index, { delayMs: Number(event.currentTarget.value) })}
              />
            </div>
            <p class="dsd-setting-hint">{tr('讓這個裝置晚一點發聲。有線喇叭比藍牙耳機早出聲時，把有線裝置的延遲加大來對齊。')}</p>
          </div>
          <div class="dsd-setting-row" role="group" aria-label={tr('聲道映射')}>
            <span class="dsd-setting-label"><strong>{tr('聲道映射')}</strong><output>{tr('{0} 個聲道', [channels])}</output></span>
            <div class="dsd-choice-row">
              <button
                type="button"
                class="soft-button"
                class:is-active={mapMode === 'auto'}
                aria-pressed={mapMode === 'auto'}
                onclick={() => update(index, { channelMap: null }, true)}
              >{tr('自動')}</button>
              {#each routePresets(channels) as preset (preset.id)}
                <button
                  type="button"
                  class="soft-button"
                  class:is-active={activePreset(channels, entry?.channelMap ?? null) === preset.id}
                  aria-pressed={activePreset(channels, entry?.channelMap ?? null) === preset.id}
                  onclick={() => update(index, { channelMap: preset.routes.map((route) => ({ ...route })) }, true)}
                >{presetLabel(preset.id)}</button>
              {/each}
            </div>
            {#if mapMode === 'auto'}
              <p class="dsd-setting-hint">{tr('自動：依裝置的聲道數混縮或補齊，例如 5.1 曲目在雙聲道裝置上依標準規則混成立體聲。')}</p>
            {:else}
              <div class="channel-map" role="table" aria-label={tr('聲道映射')}>
                {#each Array.from({ length: channels }, (_, channel) => channel) as channel (channel)}
                  {@const route = currentMap(index)[channel] ?? { source: null, gainPercent: 100 }}
                  {@const label = channelLabels(channels)[channel]}
                  <div class="channel-map-row" role="row">
                    <span class="channel-map-output" role="cell">{label || tr('輸出 {0}', [channel + 1])}</span>
                    <select
                      role="cell"
                      aria-label={tr('輸出 {0} 的來源聲道', [label || channel + 1])}
                      value={route.source === null ? '' : String(route.source)}
                      onchange={(event) => {
                        const value = event.currentTarget.value;
                        setRoute(index, channel, { source: value === '' ? null : Number(value) });
                      }}
                    >
                      <option value="">{tr('靜音')}</option>
                      {#each Array.from({ length: OUTPUT_MAX_SOURCE_CHANNELS }, (_, source) => source) as source (source)}
                        <option value={String(source)}>{sourceOption(source)}</option>
                      {/each}
                    </select>
                    <input
                      type="range"
                      min="0"
                      max="100"
                      step="1"
                      value={route.gainPercent}
                      disabled={route.source === null}
                      aria-label={tr('輸出 {0} 的增益', [label || channel + 1])}
                      oninput={(event) => setRoute(index, channel, { gainPercent: Number(event.currentTarget.value) })}
                    />
                    <output class="channel-map-gain">{route.gainPercent}%</output>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
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

  .output-device-role {
    flex: 0 0 auto;
    padding: 2px 8px;
    border: 1px solid var(--line);
    border-radius: 999px;
    color: var(--text-soft);
    font-size: var(--font-caption);
    white-space: nowrap;
  }

  .output-device-role.is-main {
    border-color: color-mix(in srgb, var(--accent-text) 55%, var(--line));
    color: var(--accent-text);
  }

  .output-device-details {
    display: grid;
    gap: 14px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }

  .output-delay-controls {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .output-delay-controls .dsd-gain-slider {
    flex: 1 1 auto;
  }

  .output-delay-number {
    flex: 0 0 auto;
    width: 76px;
  }

  .channel-map {
    display: grid;
    gap: 6px;
  }

  .channel-map-row {
    display: grid;
    grid-template-columns: 56px minmax(120px, 190px) minmax(80px, 1fr) 44px;
    align-items: center;
    gap: 10px;
  }

  .channel-map-output {
    font-weight: 600;
  }

  .channel-map-gain {
    color: var(--text-soft);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  @media (max-width: 520px) {
    .output-device-head {
      flex-wrap: wrap;
    }

    .channel-map-row {
      grid-template-columns: 48px 1fr 40px;
    }

    .channel-map-row input[type='range'] {
      grid-column: 1 / -1;
      grid-row: 2;
    }
  }
</style>
