<script lang="ts">
  import { tr } from './i18n.js';
  // 頻譜圖：桌面為獨立 Tauri 視窗（label `spectrogram`），Android 以全螢幕頁面呈現（ADR 0013）。
  // 可開啟任意檔案（對話框／拖放）並以 A／B 比較：單張、並排、疊圖（切換／透明度／差異）、分割滑桿。
  // A、B 共用設定、軸與縮放平移；滑鼠讀值同時顯示兩者。不在視窗內瀏覽曲庫換曲。
  import { onDestroy, onMount } from 'svelte';
  // Android: the page opens from the detail dialog, which is portaled into .app-shell at
  // z-index 1000+. Mount the page there too (above it and the dock) so it is not covered.
  function portalToShell(node: HTMLElement) {
    const shell = document.querySelector('.app-shell');
    if (shell) shell.appendChild(node);
    return { destroy() { if (shell) node.remove(); } };
  }
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { followLanguageChange } from './language-sync.js';
  import {
    COLORMAPS, FFT_SIZES, WINDOW_FUNCTIONS, fractionToFrequency, frequencyTicks, frequencyToFraction,
    loadSpectrogramSettings, saveSpectrogramSettings, toRenderOptions, type SpectrogramSettings,
  } from './spectrogram-settings.js';
  import {
    COMPARE_MODES, FULL_VIEW, differenceImage, dropAssignment, layerHeightFraction, levelToDb,
    loadComparePrefs, panView, pinchView, sampleLevel, saveComparePrefs, timeTickStep, timeTickValues, worldStyle, zoomView,
    type ComparePrefs, type View,
  } from './spectrogram-compare.js';

  type Side = 'A' | 'B';
  type Spectrogram = {
    pngBase64: string; levelsBase64: string; sampleRate: number; channels: number; channelLabels: string[];
    panels: number; durationMs: number; columns: number; height: number; maxFrequencyHz: number;
    cutoffHz: number | null; fileName: string; path: string;
  };
  type Source = { kind: 'track'; id: string } | { kind: 'file'; path: string };
  type Loaded = { data: Spectrogram; levels: Uint8Array | null };

  let { trackId: initialTrackId, onClose }: { trackId: string; onClose?: () => void } = $props();

  let sources = $state<Record<Side, Source | null>>({ A: null, B: null });
  let loaded = $state<Record<Side, Loaded | null>>({ A: null, B: null });
  let loading = $state<Record<Side, boolean>>({ A: false, B: false });
  let errors = $state<Record<Side, string | null>>({ A: null, B: null });
  let settings = $state<SpectrogramSettings>(loadSpectrogramSettings());
  let prefs = $state<ComparePrefs>(loadComparePrefs());
  let showSettings = $state(false);
  let showB = $state(true);
  let view = $state<View>({ ...FULL_VIEW });
  let hover = $state<{ x: number; y: number; wx: number; wy: number; host: HTMLElement } | null>(null);
  let dragOver = $state(false);
  let sizer = $state<HTMLDivElement | null>(null);
  let diffUrl = $state<string | null>(null);
  const serial: Record<Side, number> = { A: 0, B: 0 };
  const renderedKey: Record<Side, string> = { A: '', B: '' };
  const unlisteners: UnlistenFn[] = [];
  let renderTimer: ReturnType<typeof setTimeout> | null = null;
  let panStart: { x: number; y: number; view: View; rect: DOMRect } | null = null;
  /** The fingers (and the mouse) currently down on a plot; two fingers pinch-zoom and pan. */
  const pointers = new Map<number, { x: number; y: number }>();
  let pinchPrev: { x: number; y: number; dist: number } | null = null;

  function pinchState(host: HTMLElement) {
    const [a, b] = [...pointers.values()];
    const rect = host.getBoundingClientRect();
    return { x: (a.x + b.x) / 2 - rect.left, y: (a.y + b.y) / 2 - rect.top, dist: Math.hypot(a.x - b.x, a.y - b.y), width: rect.width, height: rect.height };
  }

  const compareActive = $derived(prefs.mode !== 'single' && !!loaded.B);
  const sides = $derived<Side[]>(compareActive ? ['A', 'B'] : ['A']);

  function renderSize() {
    const rect = sizer?.getBoundingClientRect();
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    return { columns: (rect?.width ?? 1200) * dpr, height: (rect?.height ?? 500) * dpr };
  }

  function decodeLevels(base64: string): Uint8Array {
    const binary = atob(base64);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) out[i] = binary.charCodeAt(i);
    return out;
  }

  async function render(side: Side, force = false) {
    const source = sources[side];
    if (!source) return;
    const size = renderSize();
    const options = toRenderOptions(settings, size.columns, size.height);
    const key = JSON.stringify([source, options]);
    if (!force && key === renderedKey[side]) return;
    renderedKey[side] = key;
    const mine = ++serial[side];
    loading[side] = true; errors[side] = null;
    try {
      const requested = performance.now();
      const data = source.kind === 'track'
        ? await invoke<Spectrogram>('track_spectrogram', { trackId: source.id, options })
        : await invoke<Spectrogram>('spectrogram_render_file', { path: source.path, options });
      const timing = data as unknown as { prepareMs?: number; renderMs?: number; encodeMs?: number };
      console.info(`[spectrogram] ${side} total=${Math.round(performance.now() - requested)}ms prepare=${timing.prepareMs}ms render=${timing.renderMs}ms encode=${timing.encodeMs}ms cols=${data.columns} h=${data.height}`);
      if (mine !== serial[side]) return;
      loaded[side] = { data, levels: decodeLevels(data.levelsBase64) };
      if (side === 'A') document.title = tr("頻譜圖 — {0}", [data.fileName]);
    } catch (cause) {
      if (mine === serial[side]) { errors[side] = String(cause); loaded[side] = null; }
    } finally {
      if (mine === serial[side]) loading[side] = false;
    }
  }

  const renderAll = (force = false) => { void render('A', force); void render('B', force); };
  function scheduleRender(delay = 250) {
    if (renderTimer) clearTimeout(renderTimer);
    renderTimer = setTimeout(() => renderAll(), delay);
  }

  function setSource(side: Side, source: Source) {
    sources[side] = source;
    view = { ...FULL_VIEW };
    if (side === 'B' && prefs.mode === 'single') updatePrefs({ mode: 'sideH' });
    void render(side, true);
  }

  async function pickFile(side: Side) {
    try {
      const path = await invoke<string | null>('spectrogram_pick_file');
      if (path) setSource(side, { kind: 'file', path });
    } catch (cause) { errors[side] = String(cause); }
  }

  function clearB() {
    sources.B = null; loaded.B = null; errors.B = null; serial.B += 1;
  }

  function swap() {
    [sources.A, sources.B] = [sources.B, sources.A];
    [loaded.A, loaded.B] = [loaded.B, loaded.A];
    [renderedKey.A, renderedKey.B] = [renderedKey.B, renderedKey.A];
  }

  function updateSettings(patch: Partial<SpectrogramSettings>) {
    settings = { ...settings, ...patch };
    saveSpectrogramSettings(settings);
    scheduleRender();
  }
  function updatePrefs(patch: Partial<ComparePrefs>) {
    prefs = { ...prefs, ...patch };
    saveComparePrefs(prefs);
  }

  // Shared axes
  const durationMax = $derived(Math.max(loaded.A?.data.durationMs ?? 0, compareActive ? loaded.B?.data.durationMs ?? 0 : 0) || 1);
  const sharedMaxHz = $derived(Math.max(loaded.A?.data.maxFrequencyHz ?? 0, compareActive ? loaded.B?.data.maxFrequencyHz ?? 0 : 0) || 22050);
  const panels = $derived(Math.max(loaded.A?.data.panels ?? 1, compareActive ? loaded.B?.data.panels ?? 1 : 1));
  const range = $derived(settings.dbCeiling - settings.dbFloor);

  function layerOf(side: Side) {
    const item = loaded[side];
    if (!item?.levels) return null;
    return {
      levels: item.levels, columns: item.data.columns, height: item.data.height,
      widthFraction: item.data.durationMs / durationMax,
      heightFraction: layerHeightFraction(item.data.maxFrequencyHz, sharedMaxHz, false),
    };
  }

  // Difference image (overlay mode)
  $effect(() => {
    const want = compareActive && prefs.mode === 'overlay' && prefs.difference;
    const a = want ? layerOf('A') : null;
    const b = want ? layerOf('B') : null;
    if (!a || !b) { diffUrl = null; return; }
    const width = Math.max(a.columns, b.columns);
    const height = Math.max(a.height, b.height);
    const canvas = document.createElement('canvas');
    canvas.width = width; canvas.height = height;
    const context = canvas.getContext('2d');
    if (!context) return;
    context.putImageData(new ImageData(differenceImage(a, b, width, height), width, height), 0, 0);
    diffUrl = canvas.toDataURL('image/png');
  });

  const ticks = $derived(frequencyTicks(sharedMaxHz, false));
  /** Width of one plot in px (a side-by-side layout halves it), for the time axis step. */
  let plotWidthPx = $state(1200);
  /** The window is taller than wide: the colour bar lies below the plot instead of at its right. */
  let portrait = $state(false);
  const timeTicks = $derived.by(() => {
    const total = durationMax / 1000;
    const visible = total * (view.x1 - view.x0);
    return timeTickValues(view.x0 * total, view.x1 * total, timeTickStep(visible, plotWidthPx));
  });
  /** A label at the very left or right edge of the axis stays inside it (centred there it would be cut off). */
  const timeLabelShift = (fraction: number) => (fraction < 0.03 ? '0 0' : fraction > 0.97 ? '-100% 0' : '-50% 0');
  const toViewX = (worldX: number) => (worldX - view.x0) / (view.x1 - view.x0);
  const toViewTop = (worldY: number) => 1 - (worldY - view.y0) / (view.y1 - view.y0);
  const panelWorldY = (p: number, axisFraction: number) => (panels - 1 - p + axisFraction) / panels;
  const channelName = (i: number) => loaded.A?.data.channelLabels?.[i] ?? loaded.B?.data.channelLabels?.[i] ?? `Ch ${i + 1}`;
  const fmtTime = (s: number) => {
    const m = Math.floor(s / 60); const sec = s - m * 60;
    return `${m}:${sec < 10 ? '0' : ''}${(view.x1 - view.x0) < 0.05 ? sec.toFixed(2) : Math.floor(sec)}`;
  };
  const fmtFreq = (f: number) => (f >= 1000 ? `${(f / 1000).toFixed(f >= 10000 || f % 1000 === 0 ? 0 : 1)}k` : `${Math.round(f)}`);
  const fmtHz = (hz: number) => (hz >= 1000 ? `${(hz / 1000).toFixed(2)} kHz` : `${Math.round(hz)} Hz`);
  const world = $derived(worldStyle(view));

  function readout() {
    if (!hover) return null;
    const { wx, wy } = hover;
    const p = Math.min(panels - 1, Math.max(0, panels - 1 - Math.floor(wy * panels)));
    const axisFraction = wy * panels - (panels - 1 - p);
    const values = (['A', 'B'] as Side[]).filter((s) => s === 'A' || compareActive).map((side) => {
      const layer = layerOf(side);
      const level = layer ? sampleLevel(layer, wx, wy) : null;
      return { side, db: level === null ? null : levelToDb(level, settings.dbFloor, settings.dbCeiling) };
    });
    const [a, b] = values;
    return {
      time: fmtTime((wx * durationMax) / 1000),
      freq: fmtHz(fractionToFrequency(axisFraction, sharedMaxHz, false)),
      channel: panels > 1 ? channelName(p) : '',
      values,
      delta: a?.db != null && b?.db != null ? b.db - a.db : null,
    };
  }

  function worldAt(event: { clientX: number; clientY: number }, host: HTMLElement) {
    const rect = host.getBoundingClientRect();
    const vx = (event.clientX - rect.left) / rect.width;
    const vy = 1 - (event.clientY - rect.top) / rect.height;
    return { vx, vy, wx: view.x0 + vx * (view.x1 - view.x0), wy: view.y0 + vy * (view.y1 - view.y0), rect };
  }

  function onPointerMove(event: PointerEvent) {
    const host = event.currentTarget as HTMLElement;
    if (pointers.has(event.pointerId)) pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 2) {
      const next = pinchState(host);
      if (pinchPrev) view = pinchView(view, pinchPrev, next, next.width, next.height);
      pinchPrev = { x: next.x, y: next.y, dist: next.dist };
      hover = null;
      return;
    }
    if (panStart) {
      const dx = -(event.clientX - panStart.x) / panStart.rect.width;
      const dy = (event.clientY - panStart.y) / panStart.rect.height;
      view = panView(panStart.view, dx, dy);
    }
    const { vx, vy, wx, wy, rect } = worldAt(event, host);
    if (vx < 0 || vx > 1 || vy < 0 || vy > 1) { hover = null; return; }
    hover = { x: event.clientX - rect.left, y: event.clientY - rect.top, wx, wy, host };
  }
  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    const host = event.currentTarget as HTMLElement;
    host.setPointerCapture(event.pointerId);
    pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
    if (pointers.size === 2) {
      // The second finger turns the drag into a pinch.
      panStart = null;
      const state = pinchState(host);
      pinchPrev = { x: state.x, y: state.y, dist: state.dist };
      return;
    }
    panStart = { x: event.clientX, y: event.clientY, view: { ...view }, rect: host.getBoundingClientRect() };
  }
  function onPointerUp(event: PointerEvent) {
    pointers.delete(event.pointerId);
    pinchPrev = null;
    // One finger left after a pinch: keep dragging from where it is, without a jump.
    const rest = pointers.size === 1 ? [...pointers.values()][0] : null;
    panStart = rest ? { x: rest.x, y: rest.y, view: { ...view }, rect: (event.currentTarget as HTMLElement).getBoundingClientRect() } : null;
  }
  function onWheel(event: WheelEvent) {
    event.preventDefault();
    const host = event.currentTarget as HTMLElement;
    const { vx, vy } = worldAt(event, host);
    const factor = Math.exp((event.deltaY || event.deltaX) * 0.0015);
    // The wheel zooms time and frequency together around the pointer; Shift keeps it to frequency, Ctrl to time.
    if (event.shiftKey) view = zoomView(view, 'y', vy, factor);
    else if (event.ctrlKey) view = zoomView(view, 'x', vx, factor);
    else view = zoomView(zoomView(view, 'x', vx, factor), 'y', vy, factor);
  }

  let wipeDragging = false;
  function onWipeDown(event: PointerEvent) {
    event.stopPropagation();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    wipeDragging = true;
  }
  function onWipeMove(event: PointerEvent) {
    if (!wipeDragging) return;
    event.stopPropagation();
    const host = (event.currentTarget as HTMLElement).parentElement!;
    const rect = host.getBoundingClientRect();
    updatePrefs({ wipe: Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)) });
  }

  function onKeydown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    if (target && /^(INPUT|SELECT|TEXTAREA)$/.test(target.tagName)) return;
    if (event.code === 'Space' && compareActive && prefs.mode === 'overlay') { event.preventDefault(); showB = !showB; }
    if (event.key === '0') view = { ...FULL_VIEW };
  }

  function channelValue(): string {
    const c = settings.channel;
    return c.mode === 'channel' ? `channel:${c.index}` : c.mode;
  }
  function setChannel(value: string) {
    if (value.startsWith('channel:')) updateSettings({ channel: { mode: 'channel', index: Number(value.slice(8)) } });
    else updateSettings({ channel: value === 'separate' ? { mode: 'separate' } : { mode: 'mix' } });
  }
  const channelCount = $derived(Math.max(loaded.A?.data.channels ?? 0, loaded.B?.data.channels ?? 0) || 2);
  const channelOptions = $derived(Array.from({ length: channelCount }, (_, i) => i));
  const colormapGradient = $derived(COLORMAPS.find((c) => c.value === settings.colormap)?.gradient ?? '');
  /** The colour bar stands at the right of the plot like Spek's: the loudest level at the top, the quietest at the bottom. */
  const colorBarGradient = $derived(
    portrait
      ? (diffUrl ? 'linear-gradient(90deg,#00f,#fff,#f00)' : colormapGradient)
      : (diffUrl ? 'linear-gradient(0deg,#00f,#fff,#f00)' : colormapGradient.replace('90deg', '0deg')),
  );
  /** Tick labels of the colour bar: every 10 dB (every 20 dB for a very wide range), or the difference scale in diff mode. */
  const colorBarTicks = $derived.by(() => {
    if (diffUrl) {
      const half = range / 2;
      return [
        { top: 0, label: `+${half.toFixed(0)} dB` },
        { top: 50, label: '0 dB' },
        { top: 100, label: `−${half.toFixed(0)} dB` },
      ];
    }
    // Below the plot the labels stand side by side: use the finest step that keeps them about 50 px apart.
    const step = portrait
      ? ([10, 20, 30, 50, 100].find((s) => (s / range) * plotWidthPx >= 50) ?? 100)
      : (range > 130 ? 20 : 10);
    const ticks = [];
    for (let db = Math.floor(settings.dbCeiling / step) * step; db >= settings.dbFloor; db -= step) {
      ticks.push({ top: ((settings.dbCeiling - db) / range) * 100, label: `${db} dB` });
    }
    return ticks;
  });

  // Drag and drop (required): two files → A and B; one file → hovered side.
  function handleDrop(paths: string[], clientX: number, clientY: number) {
    const element = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
    const hovered = (element?.closest('[data-drop-side]') as HTMLElement | null)?.dataset.dropSide as Side | undefined;
    const plotRect = sizer?.getBoundingClientRect();
    const xFraction = plotRect ? (clientX - plotRect.left) / plotRect.width : 0;
    const assignment = dropAssignment(paths, prefs.mode, hovered ?? null, xFraction);
    if (assignment.B && prefs.mode === 'single') updatePrefs({ mode: 'sideH' });
    if (assignment.A) setSource('A', { kind: 'file', path: assignment.A });
    if (assignment.B) setSource('B', { kind: 'file', path: assignment.B });
  }

  let resizeObserver: ResizeObserver | null = null;
  onMount(async () => {
    if (initialTrackId) setSource('A', { kind: 'track', id: initialTrackId });
    window.addEventListener('keydown', onKeydown);
    if (sizer) {
      const measure = () => {
        // The axis column is 46 px; a left-right layout gives each plot half of what is left.
        const cells = compareActive && prefs.mode === 'sideH' ? 2 : 1;
        plotWidthPx = Math.max(1, (sizer!.clientWidth - (cells - 1) * 10) / cells - 46);
        portrait = window.innerWidth < window.innerHeight;
      };
      measure();
      resizeObserver = new ResizeObserver(() => { measure(); scheduleRender(400); });
      resizeObserver.observe(sizer);
    }
    try {
      unlisteners.push(await listen<string>('app-language-changed', (event) => followLanguageChange(event.payload)));
      unlisteners.push(await listen<string>('spectrogram-track', (event) => setSource('A', { kind: 'track', id: event.payload })));
      const { getCurrentWebview } = await import('@tauri-apps/api/webview');
      unlisteners.push(await getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === 'enter' || payload.type === 'over') dragOver = true;
        else if (payload.type === 'leave') dragOver = false;
        else if (payload.type === 'drop') {
          dragOver = false;
          const dpr = window.devicePixelRatio || 1;
          handleDrop(payload.paths, payload.position.x / dpr, payload.position.y / dpr);
        }
      }));
    } catch { /* not in a Tauri window */ }
  });
  onDestroy(() => {
    unlisteners.forEach((u) => u());
    resizeObserver?.disconnect();
    window.removeEventListener('keydown', onKeydown);
    if (renderTimer) clearTimeout(renderTimer);
  });
</script>

{#snippet layer(side: Side, opacity = 1, clipLeft: number | null = null)}
  {@const item = loaded[side]}
  {#if item}
    <div class="sg-clip" style:clip-path={clipLeft === null ? undefined : side === 'B' ? `inset(0 0 0 ${clipLeft * 100}%)` : `inset(0 ${(1 - clipLeft) * 100}% 0 0)`}>
      <div class="sg-world" style:left={`${world.left}%`} style:width={`${world.width}%`} style:bottom={`${world.bottom}%`} style:height={`${world.height}%`}>
        <img src={`data:image/png;base64,${item.data.pngBase64}`} alt="" draggable="false" style:opacity={opacity}
          style:width={`${(item.data.durationMs / durationMax) * 100}%`}
          style:height={`${layerHeightFraction(item.data.maxFrequencyHz, sharedMaxHz, false) * 100}%`} />
      </div>
    </div>
  {/if}
{/snippet}

{#snippet plotBox(content: 'A' | 'B' | 'overlay' | 'wipe', dropSide: Side | null)}
  {@const info = readout()}
  <div class="sg-plot-area">
    <div class="sg-yaxis">
      {#each Array.from({ length: panels }, (_, p) => p) as p (p)}
        {#each ticks as f (f)}
          {@const top = toViewTop(panelWorldY(p, frequencyToFraction(f, sharedMaxHz, false)))}
          {#if top >= -0.001 && top <= 1.001}<span style:top={`${top * 100}%`}>{fmtFreq(f)}</span>{/if}
        {/each}
        {#if panels > 1}
          {@const mid = toViewTop(panelWorldY(p, 0.5))}
          {#if mid >= 0 && mid <= 1}<em style:top={`${mid * 100}%`}>{channelName(p)}</em>{/if}
        {/if}
      {/each}
    </div>
    <div class="sg-plot" data-drop-side={dropSide} role="img" aria-label={tr("頻譜圖")}
      onpointermove={onPointerMove} onpointerdown={onPointerDown} onpointerup={onPointerUp} onpointercancel={onPointerUp}
      onpointerleave={() => { hover = null; }} onwheel={onWheel} ondblclick={() => { view = { ...FULL_VIEW }; }}>
      {#if content === 'A' || content === 'B'}
        {@render layer(content)}
      {:else if content === 'overlay'}
        {#if diffUrl}
          <div class="sg-clip"><div class="sg-world" style:left={`${world.left}%`} style:width={`${world.width}%`} style:bottom={`${world.bottom}%`} style:height={`${world.height}%`}><img src={diffUrl} alt="" draggable="false" style:width="100%" style:height="100%" /></div></div>
        {:else}
          {@render layer('A')}
          {#if showB}{@render layer('B', prefs.opacityB)}{/if}
        {/if}
      {:else}
        {@render layer('A', 1, prefs.wipe)}
        {@render layer('B', 1, prefs.wipe)}
        <div class="sg-wipe" style:left={`${prefs.wipe * 100}%`} role="slider" aria-label={tr("分割位置")} aria-valuenow={Math.round(prefs.wipe * 100)} tabindex="0"
          onpointerdown={onWipeDown} onpointermove={onWipeMove} onpointerup={() => { wipeDragging = false; }}><span>A｜B</span></div>
      {/if}
      {#each Array.from({ length: panels }, (_, p) => p) as p (p)}
        {#each ticks as f (f)}
          {@const top = toViewTop(panelWorldY(p, frequencyToFraction(f, sharedMaxHz, false)))}
          {#if top > 0 && top < 1}<i class="sg-grid" style:top={`${top * 100}%`}></i>{/if}
        {/each}
        {#if p > 0}{@const top = toViewTop(panelWorldY(p, 1))}{#if top > 0 && top < 1}<i class="sg-divider" style:top={`${top * 100}%`}></i>{/if}{/if}
      {/each}
      {#if dropSide ? loading[dropSide] : (loading.A || loading.B)}<span class="sg-badge">{tr("計算中…")}</span>{/if}
      {#if dropSide && !loaded[dropSide] && !loading[dropSide]}<span class="sg-empty">{tr("拖放檔案或按「開啟 {0}」", [dropSide])}</span>{/if}
      {#if content === 'overlay' && !diffUrl}<span class="sg-badge">{showB ? tr("B 透明度 {0}%", [Math.round(prefs.opacityB * 100)]) : tr('只顯示 A')}{tr("（Space 切換）")}</span>{/if}
      {#if info}
        <div class="sg-crosshair" style:left={`${toViewX(hover!.wx) * 100}%`}></div>
          <div class="sg-hover" style:left={`min(${hover!.x + 14}px, calc(100% - 190px))`} style:top={`${hover!.y + 14}px`}>
            <div>{info.channel ? `${info.channel} · ` : ''}{info.time} · {info.freq}</div>
            {#each info.values as v (v.side)}<div><b>{v.side}</b> {v.db === null ? '—' : `${v.db.toFixed(1)} dB`}</div>{/each}
            {#if info.delta !== null}<div>B − A {info.delta >= 0 ? '+' : ''}{info.delta.toFixed(1)} dB</div>{/if}
          </div>
      {/if}
    </div>
    <div></div>
    <div class="sg-xaxis">
      {#each timeTicks as t (t)}
        {@const at = toViewX((t * 1000) / durationMax)}
        <span style:left={`${at * 100}%`} style:translate={timeLabelShift(at)}>{fmtTime(t)}</span>
      {/each}
    </div>
  </div>
{/snippet}

<div class="spectrogram-page" class:is-drag-over={dragOver} use:portalToShell>
  <header class="spectrogram-toolbar">
    <button type="button" class="sg-button" onclick={() => void pickFile('A')}>{tr("開啟 A")}</button>
    <button type="button" class="sg-button" onclick={() => void pickFile('B')}>{tr("開啟 B")}</button>
    {#if loaded.B}
      <button type="button" class="sg-button" onclick={swap} title={tr("交換 A／B")}>⇄</button>
      <button type="button" class="sg-button" onclick={clearB}>{tr("關閉 B")}</button>
    {/if}
    <label class="sg-inline">{tr("比較")}
      <select value={prefs.mode} onchange={(e) => updatePrefs({ mode: e.currentTarget.value })}>
        {#each COMPARE_MODES as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
      </select>
    </label>
    {#if compareActive && prefs.mode === 'overlay'}
      <button type="button" class="sg-button" class:is-active={!showB} disabled={prefs.difference} onclick={() => (showB = !showB)}>{showB ? 'A+B' : tr('只看 A')}</button>
      <label class="sg-inline">{tr("B 透明度")} <input type="range" min="0" max="1" step="0.05" value={prefs.opacityB} disabled={prefs.difference} oninput={(e) => updatePrefs({ opacityB: Number(e.currentTarget.value) })} /></label>
      <label class="sg-inline"><input type="checkbox" checked={prefs.difference} onchange={(e) => updatePrefs({ difference: e.currentTarget.checked })} /> {tr("差異圖（B − A）")}</label>
    {/if}
    <span class="sg-spacer"></span>
    <button type="button" class="sg-button" onclick={() => { view = { ...FULL_VIEW }; }} title={tr("重置縮放（雙擊圖面或按 0）")}>{tr("重置縮放")}</button>
    <button type="button" class="sg-button" class:is-active={showSettings} aria-expanded={showSettings} onclick={() => (showSettings = !showSettings)}>{tr("設定")}</button>
  </header>

  {#if showSettings}
    <section class="sg-settings" aria-label={tr("頻譜圖設定")}>
      <label>{tr("配色")}
        <select value={settings.colormap} onchange={(e) => updateSettings({ colormap: e.currentTarget.value })}>
          {#each COLORMAPS as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
        </select>
      </label>
      <label>{tr("FFT 大小")}
        <select value={String(settings.fftSize)} onchange={(e) => updateSettings({ fftSize: Number(e.currentTarget.value) })}>
          {#each FFT_SIZES as size (size)}<option value={String(size)}>{size}</option>{/each}
        </select>
      </label>
      <label>{tr("窗函數")}
        <select value={settings.window} onchange={(e) => updateSettings({ window: e.currentTarget.value })}>
          {#each WINDOW_FUNCTIONS as w (w.value)}<option value={w.value}>{w.label}</option>{/each}
        </select>
      </label>
      <label>{tr("聲道")}
        <select value={channelValue()} onchange={(e) => setChannel(e.currentTarget.value)}>
          <option value="mix">{tr("混合（全部聲道）")}</option>
          {#each channelOptions as i (i)}<option value={`channel:${i}`}>{channelName(i)}</option>{/each}
          <option value="separate">{tr("各聲道分開")}</option>
        </select>
      </label>
      <label>{tr("dB 下限")} <input type="number" step="5" min="-200" max="-1" value={settings.dbFloor} onchange={(e) => updateSettings({ dbFloor: Number(e.currentTarget.value) })} /></label>
      <label>{tr("dB 上限")} <input type="number" step="5" min="-199" max="20" value={settings.dbCeiling} onchange={(e) => updateSettings({ dbCeiling: Number(e.currentTarget.value) })} /></label>
      <label>{tr("最高頻率 (kHz)")}
        <input type="number" step="1" min="0.1" placeholder={tr("自動")} value={settings.maxFrequencyHz ? settings.maxFrequencyHz / 1000 : ''} onchange={(e) => { const v = Number(e.currentTarget.value); updateSettings({ maxFrequencyHz: e.currentTarget.value && v > 0 ? Math.round(v * 1000) : null }); }} />
      </label>
    </section>
  {/if}

  <div class="sg-info-row">
    {#each (['A', 'B'] as Side[]).filter((s) => s === 'A' || sources.B) as side (side)}
      {@const item = loaded[side]}
      <div class="sg-info">
        <b class="sg-tag">{side}</b>
        <strong title={item?.data.path}>{item?.data.fileName ?? (sources[side] ? '…' : tr('（未開啟）'))}</strong>
        {#if item}
          <span>{item.data.sampleRate} Hz · {item.data.channels} ch · {Math.floor(item.data.durationMs / 60000)}:{String(Math.floor((item.data.durationMs / 1000) % 60)).padStart(2, '0')}</span>
          {#if item.data.cutoffHz}<span class="sg-cutoff">{tr("約 {0} kHz 以上幾乎無能量（僅供參考）", [(item.data.cutoffHz / 1000).toFixed(1)])}</span>{/if}
        {/if}
        {#if errors[side]}<span class="sg-error">{errors[side]}</span>{/if}
      </div>
    {/each}
  </div>

  <div class="sg-main">
  <div class="sg-plots" class:is-vertical={compareActive && prefs.mode === 'sideV'} bind:this={sizer}>
    {#if compareActive && (prefs.mode === 'sideH' || prefs.mode === 'sideV')}
      <div class="sg-plot-cell">{@render plotBox('A', 'A')}</div>
      <div class="sg-plot-cell">{@render plotBox('B', 'B')}</div>
    {:else if compareActive && prefs.mode === 'overlay'}
      <div class="sg-plot-cell">{@render plotBox('overlay', null)}</div>
    {:else if compareActive && prefs.mode === 'wipe'}
      <div class="sg-plot-cell">{@render plotBox('wipe', null)}</div>
    {:else}
      <div class="sg-plot-cell">{@render plotBox('A', 'A')}</div>
    {/if}
  </div>
  <div class="sg-colorbar" class:is-horizontal={portrait} aria-hidden="true">
    <div class="sg-colorbar-strip" style:background={colorBarGradient}></div>
    <div class="sg-colorbar-ticks">
      {#each colorBarTicks as tick (tick.top)}
        {#if portrait}
          {@const at = 100 - tick.top}
          <span style:left={`${at}%`} style:translate={at < 3 ? '0 0' : at > 97 ? '-100% 0' : '-50% 0'}>{tick.label}</span>
        {:else}
          <span style:top={`${tick.top}%`}>{tick.label}</span>
        {/if}
      {/each}
    </div>
  </div>
  </div>
  {#if diffUrl}
    <div class="sg-legend">
      <span>{tr("B 較弱 −")}{(range / 2).toFixed(0)} dB</span><span>{tr("B 較強 +")}{(range / 2).toFixed(0)} dB</span>
    </div>
  {/if}
  {#if dragOver}<div class="sg-drop-hint">{tr("放開以開啟（兩個檔案＝A 與 B）")}</div>{/if}
</div>

<style>
  .spectrogram-page { color-scheme: var(--color-scheme, dark); --sg-surface: color-mix(in srgb, var(--text, #e8e8ee) 10%, transparent); --sg-field: color-mix(in srgb, var(--text, #e8e8ee) 6%, transparent); position: fixed; inset: 0; display: flex; flex-direction: column; gap: 6px; padding: calc(10px + var(--android-safe-top, 0px)) calc(14px + var(--android-safe-right, 0px)) calc(10px + var(--android-safe-bottom, 0px)) calc(14px + var(--android-safe-left, 0px)); background: var(--page, #0b0b10); color: var(--text, #e8e8ee); font-size: 13px; z-index: 1100; }
  .spectrogram-page.is-drag-over { outline: 3px dashed #8a7dff; outline-offset: -6px; }
  .spectrogram-toolbar { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .sg-spacer { flex: 1; }
  .sg-button { background: var(--sg-surface); color: inherit; border: 1px solid rgba(var(--text-rgb, 232, 232, 238), 0.22); border-radius: 6px; padding: 4px 10px; cursor: pointer; font: inherit; }
  .sg-button:hover { background: rgba(var(--accent-rgb, 138, 125, 255), 0.18); }
  .sg-button.is-active { border-color: #8a7dff; }
  .sg-button:disabled { opacity: 0.5; cursor: default; }
  .sg-inline { display: flex; align-items: center; gap: 5px; }
  .sg-inline select, .sg-settings select, .sg-settings input { background: var(--sg-field); color: inherit; border: 1px solid rgba(var(--text-rgb, 232, 232, 238), 0.22); border-radius: 4px; padding: 2px 4px; font: inherit; }
  /* The opened list of a drop-down is drawn by the system with the page's light text; give it an opaque background of its own
     (the field colour is translucent, which leaves the list white) so the entries can be read. */
  .sg-inline select option, .sg-settings select option { background-color: var(--page, #0b0b10); color: var(--text, #e8e8ee); }
  .sg-settings { display: flex; flex-wrap: wrap; gap: 8px 16px; padding: 8px 10px; background: var(--sg-field); border: 1px solid rgba(var(--text-rgb, 232, 232, 238), 0.16); border-radius: 6px; }
  .sg-settings label { display: flex; align-items: center; gap: 6px; }
  .sg-settings input[type='number'] { width: 70px; }
  .sg-info-row { display: flex; gap: 18px; flex-wrap: wrap; }
  .sg-info { display: flex; gap: 10px; align-items: baseline; flex-wrap: wrap; min-width: 0; }
  .sg-info strong { max-width: 40vw; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sg-info span { opacity: 0.75; }
  .sg-tag { background: var(--accent, #8a7dff); color: var(--accent-foreground, #0b0b10); border-radius: 3px; padding: 0 5px; }
  .sg-cutoff { color: #ffcf6b; opacity: 1 !important; }
  .sg-error { color: #ff7a7a; opacity: 1 !important; }
  /* A little room above the plot: the top frequency label (centred on the top edge) would sit on the file info row. */
  .sg-main { flex: 1; min-height: 0; display: flex; gap: 6px; padding-top: 9px; }
  .sg-main:has(.sg-colorbar.is-horizontal) { flex-direction: column; }
  /* The colour bar at the right, as tall as the plot (the bottom padding is the time axis row below it). */
  .sg-colorbar { flex: 0 0 auto; display: flex; gap: 5px; padding-bottom: 18px; font-size: 11px; opacity: 0.9; }
  .sg-colorbar-strip { width: 14px; border-radius: 2px; }
  .sg-colorbar-ticks { position: relative; width: 50px; }
  .sg-colorbar-ticks span { position: absolute; left: 0; translate: 0 -50%; white-space: nowrap; }
  /* A tall window: the bar lies under the plot, as wide as the plot, the quietest level at the left. */
  .sg-colorbar.is-horizontal { flex-direction: column; padding-bottom: 0; margin-left: 46px; }
  .sg-colorbar.is-horizontal .sg-colorbar-strip { width: auto; height: 14px; }
  .sg-colorbar.is-horizontal .sg-colorbar-ticks { width: auto; height: 16px; }
  .sg-colorbar.is-horizontal .sg-colorbar-ticks span { top: 0; translate: -50% 0; }
  .sg-plots { flex: 1; min-width: 0; min-height: 0; display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; gap: 10px; }
  .sg-plots.is-vertical { grid-auto-flow: row; grid-auto-rows: 1fr; grid-auto-columns: auto; }
  .sg-plot-cell { min-width: 0; min-height: 0; display: flex; }
  .sg-plot-area { flex: 1; min-height: 0; min-width: 0; display: grid; grid-template-columns: 46px 1fr; grid-template-rows: 1fr 18px; }
  .sg-yaxis, .sg-xaxis { position: relative; font-size: 11px; opacity: 0.8; overflow: visible; }
  .sg-yaxis span { position: absolute; right: 5px; translate: 0 -50%; white-space: nowrap; }
  .sg-yaxis em { position: absolute; left: 0; translate: 0 -50%; font-style: normal; font-weight: 600; color: #8a7dff; }
  .sg-xaxis span { position: absolute; top: 2px; translate: -50% 0; white-space: nowrap; }
  .sg-plot { position: relative; min-height: 0; overflow: hidden; cursor: crosshair; background: #000; touch-action: none; }
  .sg-clip { position: absolute; inset: 0; pointer-events: none; }
  .sg-world { position: absolute; pointer-events: none; }
  .sg-world img { position: absolute; left: 0; bottom: 0; display: block; user-select: none; }
  .sg-grid { position: absolute; left: 0; right: 0; height: 1px; background: rgb(255 255 255 / 0.1); pointer-events: none; }
  .sg-divider { position: absolute; left: 0; right: 0; height: 2px; background: var(--page, #0b0b10); pointer-events: none; }
  .sg-crosshair { position: absolute; top: 0; bottom: 0; width: 1px; background: rgb(255 255 255 / 0.35); pointer-events: none; }
  .sg-hover { position: absolute; padding: 4px 7px; border-radius: 4px; background: rgb(0 0 0 / 0.8); pointer-events: none; white-space: nowrap; line-height: 1.4; z-index: 3; }
  .sg-badge { position: absolute; top: 6px; right: 8px; padding: 2px 6px; border-radius: 4px; background: rgb(0 0 0 / 0.6); font-size: 12px; pointer-events: none; }
  .sg-empty { position: absolute; inset: 0; display: grid; place-items: center; opacity: 0.55; pointer-events: none; }
  .sg-wipe { position: absolute; top: 0; bottom: 0; width: 14px; translate: -50% 0; cursor: ew-resize; z-index: 2; display: flex; justify-content: center; }
  .sg-wipe::before { content: ''; width: 2px; height: 100%; background: #fff; box-shadow: 0 0 4px #000; }
  .sg-wipe span { position: absolute; top: 50%; translate: 0 -50%; background: #fff; color: #000; font-size: 11px; padding: 1px 4px; border-radius: 3px; white-space: nowrap; }
  .sg-legend { display: flex; align-items: center; gap: 8px; margin-left: 46px; font-size: 11px; opacity: 0.85; flex-wrap: wrap; }
  .sg-drop-hint { position: absolute; inset: 0; display: grid; place-items: center; font-size: 18px; background: rgb(20 16 40 / 0.55); pointer-events: none; }
</style>
