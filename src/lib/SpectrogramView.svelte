<script lang="ts">
  // 頻譜圖：桌面為獨立 Tauri 視窗（label `spectrogram`），Android 以全螢幕頁面呈現（ADR 0013）。
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';

  type Spectrogram = { pngBase64: string; sampleRate: number; durationMs: number; columns: number; height: number; cutoffHz: number | null; fileName: string };

  let { trackId: initialTrackId, onClose }: { trackId: string; onClose?: () => void } = $props();

  let trackId = $state('');
  let data = $state<Spectrogram | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);
  let hover = $state<{ x: number; y: number; time: string; freq: string } | null>(null);
  let image = $state<HTMLImageElement | null>(null);
  let unlisten: UnlistenFn | null = null;
  let requestSerial = 0;

  async function load(id: string) {
    trackId = id;
    const serial = ++requestSerial;
    loading = true; error = null;
    try {
      const result = await invoke<Spectrogram>('track_spectrogram', { trackId: id });
      if (serial !== requestSerial) return;
      data = result;
      document.title = `頻譜圖 — ${result.fileName}`;
    } catch (cause) {
      if (serial === requestSerial) { error = String(cause); data = null; }
    } finally {
      if (serial === requestSerial) loading = false;
    }
  }

  const nyquist = $derived(data ? data.sampleRate / 2 : 0);
  const freqTicks = $derived(nyquist ? Array.from({ length: Math.floor(nyquist / 2000) + 1 }, (_, i) => i * 2000).filter((f) => f <= nyquist) : []);
  const timeTicks = $derived.by(() => {
    if (!data) return [] as number[];
    const total = data.durationMs / 1000;
    const step = [5, 10, 15, 30, 60, 120, 300].find((s) => total / s <= 12) ?? 600;
    return Array.from({ length: Math.floor(total / step) + 1 }, (_, i) => i * step);
  });
  const fmtTime = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
  const fmtFreq = (f: number) => (f >= 1000 ? `${(f / 1000).toFixed(f % 1000 ? 1 : 0)} kHz` : `${Math.round(f)} Hz`);

  function onMove(event: PointerEvent) {
    if (!data || !image) return;
    const rect = image.getBoundingClientRect();
    const fx = (event.clientX - rect.left) / rect.width;
    const fy = (event.clientY - rect.top) / rect.height;
    if (fx < 0 || fx > 1 || fy < 0 || fy > 1) { hover = null; return; }
    hover = { x: event.clientX - rect.left, y: event.clientY - rect.top, time: fmtTime((fx * data.durationMs) / 1000), freq: fmtFreq((1 - fy) * nyquist) };
  }

  onMount(async () => {
    void load(initialTrackId);
    try {
      unlisten = await listen<string>('spectrogram-track', (event) => void load(event.payload));
    } catch { /* not in a Tauri window */ }
  });
  onDestroy(() => unlisten?.());
</script>

<div class="spectrogram-page">
  <header class="spectrogram-header">
    {#if onClose}<button type="button" class="soft-button" onclick={onClose}>返回</button>{/if}
    <h1>{data?.fileName ?? '頻譜圖'}</h1>
    {#if data}
      <span class="spectrogram-meta">{data.sampleRate} Hz · {fmtTime(data.durationMs / 1000)}{#if data.cutoffHz} · 約 {fmtFreq(data.cutoffHz)} 以上無能量（可能源自有損編碼，僅供參考）{/if}</span>
    {/if}
  </header>
  {#if error}<p class="spectrogram-error" role="alert">{error}</p>{/if}
  {#if loading && !data}<p class="spectrogram-loading">計算中…</p>{/if}
  {#if data}
    <div class="spectrogram-plot" class:is-loading={loading}>
      <div class="spectrogram-yaxis">
        {#each freqTicks as f (f)}<span style:bottom={`${(f / nyquist) * 100}%`}>{fmtFreq(f)}</span>{/each}
      </div>
      <div class="spectrogram-image" role="img" aria-label="頻譜圖" onpointermove={onMove} onpointerleave={() => (hover = null)}>
        <img bind:this={image} src={`data:image/png;base64,${data.pngBase64}`} alt="" draggable="false" />
        {#each freqTicks as f (f)}<i class="grid-h" style:bottom={`${(f / nyquist) * 100}%`}></i>{/each}
        {#if hover}<div class="spectrogram-hover" style:left={`${hover.x + 12}px`} style:top={`${hover.y + 12}px`}>{hover.time} · {hover.freq}</div>{/if}
      </div>
      <div></div>
      <div class="spectrogram-xaxis">
        {#each timeTicks as t (t)}<span style:left={`${((t * 1000) / data.durationMs) * 100}%`}>{fmtTime(t)}</span>{/each}
      </div>
    </div>
    <div class="spectrogram-legend"><span>−120 dB</span><i></i><span>0 dB</span></div>
  {/if}
</div>

<style>
  .spectrogram-page { position: fixed; inset: 0; display: flex; flex-direction: column; gap: 8px; padding: 12px 16px; background: #0b0b10; color: #e8e8ee; font-size: 13px; z-index: 80; }
  .spectrogram-header { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; }
  .spectrogram-header h1 { font-size: 15px; margin: 0; }
  .spectrogram-meta { opacity: 0.75; }
  .spectrogram-error { color: #ff7a7a; }
  .spectrogram-plot { flex: 1; min-height: 0; display: grid; grid-template-columns: 64px 1fr; grid-template-rows: 1fr 22px; }
  .spectrogram-plot.is-loading { opacity: 0.6; }
  .spectrogram-yaxis, .spectrogram-xaxis { position: relative; font-size: 11px; opacity: 0.8; }
  .spectrogram-yaxis span { position: absolute; right: 6px; translate: 0 50%; white-space: nowrap; }
  .spectrogram-xaxis span { position: absolute; top: 4px; translate: -50% 0; white-space: nowrap; }
  .spectrogram-image { position: relative; min-height: 0; overflow: hidden; cursor: crosshair; }
  .spectrogram-image img { width: 100%; height: 100%; display: block; image-rendering: auto; user-select: none; }
  .grid-h { position: absolute; left: 0; right: 0; height: 1px; background: rgb(255 255 255 / 0.12); pointer-events: none; }
  .spectrogram-hover { position: absolute; padding: 2px 6px; border-radius: 4px; background: rgb(0 0 0 / 0.75); pointer-events: none; white-space: nowrap; }
  .spectrogram-legend { display: flex; align-items: center; gap: 8px; margin-left: 64px; font-size: 11px; opacity: 0.8; }
  .spectrogram-legend i { width: 200px; height: 8px; border-radius: 2px; background: linear-gradient(90deg, rgb(0 0 4), rgb(59 15 112), rgb(140 41 129), rgb(222 73 104), rgb(254 159 109), rgb(252 253 191)); }
</style>
