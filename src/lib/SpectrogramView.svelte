<script lang="ts">
  // 頻譜圖：桌面為獨立 Tauri 視窗（label `spectrogram`），Android 以全螢幕頁面呈現（ADR 0013）。
  // 視窗內可調整配色、分析參數與聲道（混合／L／R／分開）；不在視窗內換曲。
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import {
    COLORMAPS,
    FFT_SIZES,
    WINDOW_FUNCTIONS,
    fractionToFrequency,
    frequencyTicks,
    frequencyToFraction,
    loadSpectrogramSettings,
    saveSpectrogramSettings,
    toRenderOptions,
    type SpectrogramSettings,
  } from './spectrogram-settings.js';

  type Spectrogram = {
    pngBase64: string; sampleRate: number; channels: number; channelLabels: string[]; panels: number; durationMs: number;
    columns: number; height: number; maxFrequencyHz: number; cutoffHz: number | null; fileName: string; path: string;
  };
  type TrackLite = { id: string; title: string | null; artist: string | null; album: string | null };

  let { trackId: initialTrackId, onClose }: { trackId: string; onClose?: () => void } = $props();

  let trackId = $state('');
  let trackLabel = $state('');
  let data = $state<Spectrogram | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let settings = $state<SpectrogramSettings>(loadSpectrogramSettings());
  let showSettings = $state(false);
  let hover = $state<{ x: number; y: number; time: string; freq: string; channel: string } | null>(null);
  let plot = $state<HTMLDivElement | null>(null);
  let unlisten: UnlistenFn | null = null;
  let renderTimer: ReturnType<typeof setTimeout> | null = null;
  let requestSerial = 0;
  let renderedKey = '';

  
  function renderSize() {
    const rect = plot?.getBoundingClientRect();
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    return { columns: (rect?.width ?? 1200) * dpr, height: (rect?.height ?? 500) * dpr };
  }

  async function render(force = false) {
    if (!trackId) return;
    const size = renderSize();
    const options = toRenderOptions(settings, size.columns, size.height);
    const key = JSON.stringify([trackId, options]);
    if (!force && key === renderedKey) return;
    renderedKey = key;
    const serial = ++requestSerial;
    loading = true; error = null;
    try {
      const result = await invoke<Spectrogram>('track_spectrogram', { trackId, options });
      if (serial !== requestSerial) return;
      data = result;
      document.title = `頻譜圖 — ${trackLabel || result.fileName}`;
    } catch (cause) {
      if (serial === requestSerial) { error = String(cause); data = null; }
    } finally {
      if (serial === requestSerial) loading = false;
    }
  }

  function scheduleRender(delay = 250) {
    if (renderTimer) clearTimeout(renderTimer);
    renderTimer = setTimeout(() => void render(), delay);
  }

  async function setTrack(id: string, label?: string) {
    if (!id) return;
    trackId = id;
    trackLabel = label ?? '';
    await render(true);
  }

  function updateSettings(patch: Partial<SpectrogramSettings>) {
    settings = { ...settings, ...patch };
    saveSpectrogramSettings(settings);
    scheduleRender();
  }

  // Axis geometry
  const panels = $derived(data?.panels ?? 1);
  const maxHz = $derived(data?.maxFrequencyHz ?? 22050);
  const ticks = $derived(frequencyTicks(maxHz, settings.logFrequency));
  const timeTicks = $derived.by(() => {
    if (!data) return [] as number[];
    const total = data.durationMs / 1000;
    const step = [1, 2, 5, 10, 15, 30, 60, 120, 300].find((s) => total / s <= 14) ?? 600;
    return Array.from({ length: Math.floor(total / step) + 1 }, (_, i) => i * step);
  });
  // Labels come from the decoded layout (L, R, C, LFE, SL, SR…), so 5.1/7.1 list every channel.
  const channelName = (i: number, total: number) => data?.channelLabels?.[i] ?? (total === 1 ? 'Mono' : `Ch ${i + 1}`);
  const fmtTime = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
  const fmtFreq = (f: number) => (f >= 1000 ? `${(f / 1000).toFixed(f >= 10000 || f % 1000 === 0 ? 0 : 1)}k` : `${Math.round(f)}`);

  function onMove(event: PointerEvent) {
    if (!data || !plot) return;
    const rect = plot.getBoundingClientRect();
    const fx = (event.clientX - rect.left) / rect.width;
    const fyAll = (event.clientY - rect.top) / rect.height;
    if (fx < 0 || fx > 1 || fyAll < 0 || fyAll > 1) { hover = null; return; }
    const panel = Math.min(panels - 1, Math.floor(fyAll * panels));
    const fy = fyAll * panels - panel;
    const hz = fractionToFrequency(1 - fy, maxHz, settings.logFrequency);
    hover = {
      x: event.clientX - rect.left, y: event.clientY - rect.top,
      time: fmtTime((fx * data.durationMs) / 1000),
      freq: `${hz >= 1000 ? (hz / 1000).toFixed(2) + ' kHz' : Math.round(hz) + ' Hz'}`,
      channel: panels > 1 ? channelName(panel, panels) : '',
    };
  }

  function channelValue(): string {
    const c = settings.channel;
    return c.mode === 'channel' ? `channel:${c.index}` : c.mode;
  }
  function setChannel(value: string) {
    if (value.startsWith('channel:')) updateSettings({ channel: { mode: 'channel', index: Number(value.slice(8)) } });
    else updateSettings({ channel: value === 'separate' ? { mode: 'separate' } : { mode: 'mix' } });
  }
  const channelCount = $derived(data?.channels ?? 2);
  const colormapGradient = $derived(COLORMAPS.find((c) => c.value === settings.colormap)?.gradient ?? '');

  let resizeObserver: ResizeObserver | null = null;
  onMount(async () => {
    await setTrack(initialTrackId);
    if (plot) {
      resizeObserver = new ResizeObserver(() => scheduleRender(400));
      resizeObserver.observe(plot);
    }
    try {
      unlisten = await listen<string>('spectrogram-track', (event) => {
        void setTrack(event.payload);
      });
    } catch { /* not in a Tauri window */ }
  });
  onDestroy(() => {
    unlisten?.();
    resizeObserver?.disconnect();
    if (renderTimer) clearTimeout(renderTimer);
  });
</script>

<div class="spectrogram-page">
  <header class="spectrogram-toolbar">
    {#if onClose}<button type="button" class="sg-button" onclick={onClose}>返回</button>{/if}
    <button type="button" class="sg-button" class:is-active={showSettings} aria-expanded={showSettings} onclick={() => (showSettings = !showSettings)}>設定</button>
    <button type="button" class="sg-button" disabled={loading} onclick={() => void render(true)}>重新繪製</button>
  </header>

  {#if showSettings}
    <section class="sg-settings" aria-label="頻譜圖設定">
      <label>配色
        <select value={settings.colormap} onchange={(e) => updateSettings({ colormap: e.currentTarget.value })}>
          {#each COLORMAPS as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
        </select>
      </label>
      <label>FFT 大小
        <select value={String(settings.fftSize)} onchange={(e) => updateSettings({ fftSize: Number(e.currentTarget.value) })}>
          {#each FFT_SIZES as size (size)}<option value={String(size)}>{size}</option>{/each}
        </select>
      </label>
      <label>窗函數
        <select value={settings.window} onchange={(e) => updateSettings({ window: e.currentTarget.value })}>
          {#each WINDOW_FUNCTIONS as w (w.value)}<option value={w.value}>{w.label}</option>{/each}
        </select>
      </label>
      <label>頻率軸
        <select value={settings.logFrequency ? 'log' : 'linear'} onchange={(e) => updateSettings({ logFrequency: e.currentTarget.value === 'log' })}>
          <option value="linear">線性</option><option value="log">對數</option>
        </select>
      </label>
      <label>聲道
        <select value={channelValue()} onchange={(e) => setChannel(e.currentTarget.value)}>
          <option value="mix">混合（全部聲道）</option>
          {#each Array.from({ length: Math.max(1, channelCount) }, (_, i) => i) as i (i)}<option value={`channel:${i}`}>{channelName(i, channelCount)}</option>{/each}
          <option value="separate">各聲道分開</option>
        </select>
      </label>
      <label>dB 下限 <input type="number" step="5" min="-200" max="-1" value={settings.dbFloor} onchange={(e) => updateSettings({ dbFloor: Number(e.currentTarget.value) })} /></label>
      <label>dB 上限 <input type="number" step="5" min="-199" max="20" value={settings.dbCeiling} onchange={(e) => updateSettings({ dbCeiling: Number(e.currentTarget.value) })} /></label>
      <label>最高頻率 (kHz)
        <input type="number" step="1" min="0.1" placeholder="自動" value={settings.maxFrequencyHz ? settings.maxFrequencyHz / 1000 : ''} onchange={(e) => { const v = Number(e.currentTarget.value); updateSettings({ maxFrequencyHz: e.currentTarget.value && v > 0 ? Math.round(v * 1000) : null }); }} />
      </label>
    </section>
  {/if}

  <div class="sg-info">
    <strong title={data?.path}>{trackLabel || data?.fileName || '頻譜圖'}</strong>
    {#if data}
      <span>{data.sampleRate} Hz · {data.channels} ch · {fmtTime(data.durationMs / 1000)}</span>
      {#if data.cutoffHz}<span class="sg-cutoff">約 {(data.cutoffHz / 1000).toFixed(1)} kHz 以上幾乎無能量（可能源自有損編碼，僅供參考）</span>{/if}
    {/if}
    {#if loading}<span class="sg-loading">計算中…</span>{/if}
  </div>
  {#if error}<p class="sg-error" role="alert">{error}</p>{/if}

  <div class="sg-plot-area">
    <div class="sg-yaxis">
      {#each Array.from({ length: panels }, (_, p) => p) as p (p)}
        {#each ticks as f (f)}
          <span style:top={`${((p + 1 - frequencyToFraction(f, maxHz, settings.logFrequency)) / panels) * 100}%`}>{fmtFreq(f)}</span>
        {/each}
        {#if panels > 1}<em style:top={`${((p + 0.5) / panels) * 100}%`}>{channelName(p, panels)}</em>{/if}
      {/each}
    </div>
    <div class="sg-plot" bind:this={plot} role="img" aria-label="頻譜圖" onpointermove={onMove} onpointerleave={() => (hover = null)} class:is-loading={loading}>
      {#if data}<img src={`data:image/png;base64,${data.pngBase64}`} alt="" draggable="false" />{/if}
      {#each Array.from({ length: panels }, (_, p) => p) as p (p)}
        {#each ticks as f (f)}<i class="sg-grid" style:top={`${((p + 1 - frequencyToFraction(f, maxHz, settings.logFrequency)) / panels) * 100}%`}></i>{/each}
        {#if p > 0}<i class="sg-divider" style:top={`${(p / panels) * 100}%`}></i>{/if}
      {/each}
      {#if hover}<div class="sg-hover" style:left={`${hover.x + 12}px`} style:top={`${hover.y + 12}px`}>{hover.channel ? `${hover.channel} · ` : ''}{hover.time} · {hover.freq}</div>{/if}
    </div>
    <div></div>
    <div class="sg-xaxis">
      {#if data}{#each timeTicks as t (t)}<span style:left={`${((t * 1000) / data.durationMs) * 100}%`}>{fmtTime(t)}</span>{/each}{/if}
    </div>
  </div>
  <div class="sg-legend"><span>{settings.dbFloor} dB</span><i style:background={colormapGradient}></i><span>{settings.dbCeiling} dB</span><span class="sg-hint">Hz 刻度：k = kHz</span></div>
</div>

<style>
  .spectrogram-page { position: fixed; inset: 0; display: flex; flex-direction: column; gap: 6px; padding: 10px 14px; background: #0b0b10; color: #e8e8ee; font-size: 13px; z-index: 80; }
  .spectrogram-toolbar { display: flex; justify-content: flex-end; align-items: center; gap: 6px; flex-wrap: wrap; }
  .sg-button { background: #22222c; color: inherit; border: 1px solid #3a3a48; border-radius: 6px; padding: 4px 10px; cursor: pointer; font: inherit; }
  .sg-button:hover { background: #2c2c38; }
  .sg-button.is-active { border-color: #8a7dff; }
  .sg-button:disabled { opacity: 0.5; cursor: default; }
  .sg-settings { display: flex; flex-wrap: wrap; gap: 8px 16px; padding: 8px 10px; background: #15151c; border: 1px solid #2c2c38; border-radius: 6px; }
  .sg-settings label { display: flex; align-items: center; gap: 6px; }
  .sg-settings select, .sg-settings input { background: #0f0f15; color: inherit; border: 1px solid #3a3a48; border-radius: 4px; padding: 2px 4px; font: inherit; }
  .sg-settings input[type='number'] { width: 70px; }
  .sg-info { display: flex; gap: 12px; align-items: baseline; flex-wrap: wrap; }
  .sg-info strong { font-size: 14px; max-width: 60vw; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sg-info span { opacity: 0.75; }
  .sg-cutoff { color: #ffcf6b; opacity: 1 !important; }
  .sg-error { color: #ff7a7a; margin: 0; }
  .sg-plot-area { flex: 1; min-height: 0; display: grid; grid-template-columns: 52px 1fr; grid-template-rows: 1fr 20px; }
  .sg-yaxis, .sg-xaxis { position: relative; font-size: 11px; opacity: 0.8; }
  .sg-yaxis span { position: absolute; right: 6px; translate: 0 -50%; white-space: nowrap; }
  .sg-yaxis em { position: absolute; left: 0; translate: 0 -50%; font-style: normal; font-weight: 600; color: #8a7dff; }
  .sg-xaxis span { position: absolute; top: 3px; translate: -50% 0; white-space: nowrap; }
  .sg-plot { position: relative; min-height: 0; overflow: hidden; cursor: crosshair; background: #000; }
  .sg-plot.is-loading img { opacity: 0.55; }
  .sg-plot img { position: absolute; inset: 0; width: 100%; height: 100%; display: block; user-select: none; }
  .sg-grid { position: absolute; left: 0; right: 0; height: 1px; background: rgb(255 255 255 / 0.1); pointer-events: none; }
  .sg-divider { position: absolute; left: 0; right: 0; height: 2px; background: #0b0b10; pointer-events: none; }
  .sg-hover { position: absolute; padding: 2px 6px; border-radius: 4px; background: rgb(0 0 0 / 0.78); pointer-events: none; white-space: nowrap; }
  .sg-legend { display: flex; align-items: center; gap: 8px; margin-left: 52px; font-size: 11px; opacity: 0.85; }
  .sg-legend i { width: 220px; height: 8px; border-radius: 2px; }
  .sg-hint { margin-left: auto; opacity: 0.6; }
</style>
