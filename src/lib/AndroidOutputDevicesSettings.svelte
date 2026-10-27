<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this list, and the two must always be the same list:
  // everything the list shows or edits lives here, once, and each mounted copy only draws it. Reading the devices
  // again is done by the copy that was mounted first (ADR 0044).
  import type { AndroidAudioOutputStatus, AndroidOutputDevice } from './ipc';

  const s = $state({
    /** The choice in settings.json (null: the system default). */
    saved: null as AndroidOutputDevice | null,
    status: null as AndroidAudioOutputStatus | null,
    loading: true,
    loaded: false,
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

  const instance = Symbol('android-output-devices');
  /** Android tells the playback service about devices at once (AudioDeviceCallback); the list reads the service's view
   *  this often while it is shown. */
  const STATUS_POLL_MS = 2000;
  /** The service pins the device and Media3 opens a new AudioTrack shortly after a choice; read again then. */
  const AFTER_CHOICE_MS = 400;

  let rows = $derived(buildAndroidRows(s.status, s.saved));
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
      const [saved, status] = await Promise.all([
        invokeCommand('settings_get_android_output_device', {}),
        invokeCommand('android_audio_output_status', {}),
      ]);
      if (disposed) return;
      s.saved = saved;
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
    <p>{tr("選擇要輸出的裝置；拔除後改由系統預設播放，重新接上時自動切回。音訊經 Android 系統混音器輸出，系統可能再轉換取樣率，不是 bit-perfect。")}</p>
  </div>
</div>

{#if s.failure}<p class="source-error-message" role="alert">{s.failure}</p>{/if}

<div class="output-device-list" role="radiogroup" aria-label={tr('播放裝置')} aria-busy={s.loading} use:scrollChain>
  {#each rows as row (row.key)}
    <div class="output-device" class:is-playing={row.chosen} class:is-missing={!row.connected}>
      <div class="output-device-head">
        <label class="output-device-toggle">
          <input
            type="radio"
            name="android-output-device"
            checked={row.chosen}
            disabled={!row.connected || (row.key !== 'system' && row.identity === null)}
            aria-label={row.label}
            onchange={() => choose(row.identity)}
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
      {#if row.path?.text}
        <p class="output-device-path is-{row.path.tone}">{row.path.text}</p>
      {/if}
    </div>
  {/each}
</div>
