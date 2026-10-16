<script lang="ts">
  import { formatAudioFormat } from './track-columns.js';
  // 歌曲詳細資訊面板（ADR 0013 Phase 1）：單曲標籤檢視／暫存編輯，按「儲存」才寫檔。
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { hasLyricsInUse, lyricsToText } from './lyrics-import.js';
  import {
    loadCoverEditOptions,
    saveCoverEditOptions,
    type CoverEditOptions,
  } from './cover-edit-options.js';

  type TagFields = {
    title: string; artist: string; album: string; albumArtist: string; composer: string; genre: string;
    trackNumber: string; trackTotal: string; discNumber: string; discTotal: string; date: string; lyrics: string;
  };
  type TrackDetails = {
    path: string; fileName: string; fileSize: number; modifiedUnixMs: number | null; createdUnixMs: number | null; format: string;
    tagFormat: string; readOnly: boolean; readOnlyReason: string | null; fields: TagFields;
    properties: { durationMs: number; overallBitrateKbps: number | null; audioBitrateKbps: number | null; sampleRate: number | null; bitDepth: number | null; channels: number | null };
    cover: { mime: string; width: number; height: number; bytes: number } | null;
  };

  // Android: tracks are content:// documents; there is no file manager "reveal".
  const isAndroid = typeof navigator !== 'undefined' && /Android/i.test(navigator.userAgent);
  let { trackId, onClose, onSaved, onOpenSpectrogram }: {
    trackId: string;
    onClose: () => void;
    onSaved?: () => void;
    onOpenSpectrogram: (trackId: string) => void;
  } = $props();

  // trackTotal / discTotal are not shown; they stay in the draft unchanged, so Save keeps
  // whatever the file has.
  const WIDE_FIELDS: { key: keyof TagFields; label: string }[] = [
    { key: 'title', label: '標題' },
    { key: 'artist', label: '演出者' },
    { key: 'album', label: '專輯' },
    { key: 'albumArtist', label: '專輯演出者' },
    { key: 'composer', label: '作曲' },
    { key: 'genre', label: '類型' },
  ];
  const NUMBER_ROW: { key: keyof TagFields; label: string }[] = [
    { key: 'date', label: '錄音日期' },
    { key: 'trackNumber', label: '音軌號' },
    { key: 'discNumber', label: '碟號' },
  ];

  /** Move the dialog to <body> so no ancestor stacking context (the player shell) can put the
   *  dock above it. */
  function portal(node: HTMLElement) {
    // Mount inside .app-shell: it carries the live appearance vars (--quick-settings-opacity /
    // -blur, backdrop settings) that .floating-surface reads, so the panel matches the quick
    // settings drawer and follows setting changes. A direct child of .app-shell with a high
    // z-index still sits above the dock.
    (document.querySelector('.app-shell') ?? document.body).appendChild(node);
    return { destroy() { node.remove(); } };
  }

  let details = $state<TrackDetails | null>(null);
  let original = $state<TagFields | null>(null);
  let draft = $state<TagFields | null>(null);
  let error = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let saving = $state(false);
  let coverUrl = $state<string | null>(null);
  let pendingCoverPath = $state<string | null>(null);
  let pendingCoverUrl = $state<string | null>(null);
  type CoverInfo = { width: number; height: number; mime: string; bytes: number };
  /** Facts of the staged (picked / pasted, not yet saved) image. */
  let stagedCoverInfo = $state<CoverInfo | null>(null);

  function sniffImageMime(bytes: Uint8Array, fallback: string): string {
    if (bytes[0] === 0xff && bytes[1] === 0xd8) return 'image/jpeg';
    if (bytes[0] === 0x89 && bytes[1] === 0x50) return 'image/png';
    if (bytes[0] === 0x47 && bytes[1] === 0x49) return 'image/gif';
    if (bytes[0] === 0x42 && bytes[1] === 0x4d) return 'image/bmp';
    if (bytes[8] === 0x57 && bytes[9] === 0x45 && bytes[10] === 0x42 && bytes[11] === 0x50) return 'image/webp';
    return fallback || 'image/unknown';
  }

  async function describeStaged(blob: Blob) {
    stagedCoverInfo = null;
    try {
      const head = new Uint8Array(await blob.slice(0, 16).arrayBuffer());
      const bitmap = await createImageBitmap(blob);
      stagedCoverInfo = { width: bitmap.width, height: bitmap.height, mime: sniffImageMime(head, blob.type), bytes: blob.size };
      bitmap.close();
    } catch { /* undecodable: Save will report the error */ }
  }

  /** Mirrors player_tags::cover::prepare_cover: JPEG kept as-is unless resized; otherwise
   *  re-encoded to JPEG; auto-resize only shrinks to the short edge, keeping aspect ratio. */
  function savedCoverPlan(info: CoverInfo, options: CoverEditOptions): { width: number; height: number; reencode: boolean } {
    let { width, height } = info;
    const short = Math.min(width, height);
    if (options.autoResize && short > options.shortEdgePx) {
      const scale = options.shortEdgePx / short;
      width = Math.max(1, Math.round(width * scale));
      height = Math.max(1, Math.round(height * scale));
    }
    const resized = width !== info.width || height !== info.height;
    return { width, height, reencode: resized || info.mime !== 'image/jpeg' };
  }
  let removeCover = $state(false);
  let onlineLyricsText = $state<string | null>(null);
  let coverOptions = $state<CoverEditOptions>(loadCoverEditOptions());

  const isDirty = (key: keyof TagFields) => !!draft && !!original && draft[key] !== original[key];
  // Cover facts shown next to the preview follow the staged state, not the loaded file.
  const shownCover = $derived(removeCover ? null : pendingCoverPath ? stagedCoverInfo : details?.cover ?? null);
  const savedPlan = $derived(pendingCoverPath && stagedCoverInfo ? savedCoverPlan(stagedCoverInfo, coverOptions) : null);
  const coverDirty = $derived(pendingCoverPath !== null || removeCover);
  const anyDirty = $derived(!!draft && !!original && (Object.keys(original) as (keyof TagFields)[]).some((key) => draft![key] !== original![key]) || coverDirty);

  function revoke(url: string | null) { if (url) URL.revokeObjectURL(url); }

  async function loadCover() {
    revoke(coverUrl); coverUrl = null;
    const bytes = await invoke<ArrayBuffer>('track_details_cover', { trackId });
    if (bytes && bytes.byteLength > 0) coverUrl = URL.createObjectURL(new Blob([bytes]));
  }

  async function load() {
    error = null;
    try {
      const result = await invoke<TrackDetails>('track_details_get', { trackId });
      details = result;
      original = { ...result.fields };
      draft = { ...result.fields };
      pendingCoverPath = null; pendingCoverBase64 = null; revoke(pendingCoverUrl); pendingCoverUrl = null; removeCover = false;
      await loadCover();
    } catch (cause) {
      error = String(cause);
    }
    try {
      const lyrics = await invoke<{ lyrics: { provider?: string; source: string; synced: boolean; lines: { startMs: number | null; text: string; translation: string | null }[] } | null }>('lyrics_get_track', { trackId });
      onlineLyricsText = hasLyricsInUse(lyrics.lyrics) ? lyricsToText(lyrics.lyrics!) : null;
    } catch {
      onlineLyricsText = null;
    }
  }

  function importOnlineLyrics() {
    if (draft && onlineLyricsText !== null) draft.lyrics = onlineLyricsText;
  }

  function resetField(key: keyof TagFields) { if (draft && original) draft[key] = original[key]; }
  function resetAll() {
    if (original) draft = { ...original };
    pendingCoverPath = null; pendingCoverBase64 = null; revoke(pendingCoverUrl); pendingCoverUrl = null; removeCover = false;
  }

  // Clipboard image staged as the new cover (base64 of the original bytes; Save applies the
  // JPG quality / resize options like a picked file).
  const CLIPBOARD_COVER = '__clipboard__';
  let pendingCoverBase64 = $state<string | null>(null);
  let clipboardImage = $state<Blob | null>(null);

  async function readClipboardImage(): Promise<Blob | null> {
    try {
      if (!navigator.clipboard?.read) return null;
      for (const item of await navigator.clipboard.read()) {
        const type = item.types.find((value) => value.startsWith('image/'));
        if (type) return await item.getType(type);
      }
    } catch { /* permission denied / not focused / unsupported */ }
    return null;
  }

  async function refreshClipboard() { clipboardImage = await readClipboardImage(); }

  async function blobToBase64(blob: Blob): Promise<string> {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = '';
    for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    return btoa(binary);
  }

  async function pasteCover() {
    const blob = clipboardImage ?? await readClipboardImage();
    if (!blob) { error = '剪貼簿中沒有圖片'; return; }
    pendingCoverBase64 = await blobToBase64(blob);
    revoke(pendingCoverUrl);
    pendingCoverUrl = URL.createObjectURL(blob);
    void describeStaged(blob);
    pendingCoverPath = CLIPBOARD_COVER;
    removeCover = false;
  }

  /** Clipboard images must be PNG: re-encode the current cover through a canvas. */
  async function copyCover() {
    try {
      const bytes = await invoke<ArrayBuffer>('track_details_cover', { trackId });
      if (!bytes || bytes.byteLength === 0) { error = '此曲目沒有內嵌封面'; return; }
      const bitmap = await createImageBitmap(new Blob([bytes]));
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      canvas.getContext('2d')!.drawImage(bitmap, 0, 0);
      bitmap.close();
      const png = await new Promise<Blob>((resolve, reject) => canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('PNG 轉換失敗')), 'image/png'));
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
      notice = '已複製封面';
      void refreshClipboard();
    } catch (cause) {
      error = `複製封面失敗：${String(cause)}`;
    }
  }

  async function pickCover() {
    const path = await invoke<string | null>('track_details_pick_cover');
    if (!path) return;
    const bytes = await invoke<ArrayBuffer>('track_details_read_image', { path });
    revoke(pendingCoverUrl);
    const pickedBlob = new Blob([bytes]);
    pendingCoverUrl = URL.createObjectURL(pickedBlob);
    void describeStaged(pickedBlob);
    pendingCoverPath = path;
    pendingCoverBase64 = null;
    removeCover = false;
  }

  async function exportCover() {
    try {
      const saved = await invoke<string | null>('track_details_export_cover', { trackId });
      if (saved) notice = `已匯出：${saved}`;
    } catch (cause) { error = String(cause); }
  }

  async function save() {
    if (!draft || saving) return;
    saving = true; error = null; notice = null;
    try {
      const result = await invoke<TrackDetails>('track_details_save', {
        trackId,
        request: {
          fields: draft,
          replaceCoverPath: pendingCoverPath === CLIPBOARD_COVER ? null : pendingCoverPath,
          replaceCoverBase64: pendingCoverPath === CLIPBOARD_COVER ? pendingCoverBase64 : null,
          removeCover,
          coverOptions,
        },
      });
      details = result;
      original = { ...result.fields };
      draft = { ...result.fields };
      pendingCoverPath = null; pendingCoverBase64 = null; revoke(pendingCoverUrl); pendingCoverUrl = null; removeCover = false;
      await loadCover();
      notice = '已儲存';
      onSaved?.();
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
    }
  }

  function requestClose() {
    if (anyDirty && !confirm('有尚未儲存的變更，確定要關閉嗎？')) return;
    onClose();
  }

  function updateCoverOptions(patch: Partial<CoverEditOptions>) {
    coverOptions = { ...coverOptions, ...patch };
    saveCoverEditOptions(coverOptions);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); requestClose(); }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void save(); }
  }

  const formatBytes = (bytes: number) => bytes >= 1048576 ? `${(bytes / 1048576).toFixed(2)} MB` : `${Math.round(bytes / 1024)} KB`;
  const formatDuration = (ms: number) => { const s = Math.round(ms / 1000); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };
  const pad = (value: number) => String(value).padStart(2, '0');
  /** Local time with zone, e.g. 2026-10-16 23:54 (UTC+8). */
  const formatDate = (ms: number | null) => {
    if (ms == null) return '—';
    const date = new Date(ms);
    const offset = -date.getTimezoneOffset();
    const zone = `UTC${offset >= 0 ? '+' : '-'}${Math.floor(Math.abs(offset) / 60)}${Math.abs(offset) % 60 ? `:${pad(Math.abs(offset) % 60)}` : ''}`;
    return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())} (${zone})`;
  };

  let lastTrackId = '';
  $effect(() => { if (trackId !== lastTrackId) { lastTrackId = trackId; void load(); } });
  onMount(() => { window.addEventListener('keydown', onKeydown); window.addEventListener('moe:track-details-request-close', requestClose); window.addEventListener('focus', refreshClipboard); void refreshClipboard(); });
  onDestroy(() => { window.removeEventListener('keydown', onKeydown); window.removeEventListener('moe:track-details-request-close', requestClose); window.removeEventListener('focus', refreshClipboard); revoke(coverUrl); revoke(pendingCoverUrl); });
</script>

<div class="track-details-layer" use:portal>
<button class="track-details-scrim" type="button" tabindex="-1" aria-label="關閉歌曲資訊" onclick={requestClose}></button>
<div class="track-details floating-surface" role="dialog" aria-modal="true" aria-label="歌曲資訊">
  <header class="track-details-header">
    <h2>歌曲資訊</h2>
    {#if notice}<span class="track-details-saved">{notice}</span>{/if}
    <div class="track-details-actions">
      <button type="button" class="soft-button" disabled={!anyDirty || saving} onclick={resetAll}>還原</button>
      <button type="button" class="soft-button" onclick={requestClose}>關閉</button>
      <button type="button" class="soft-button is-primary" disabled={!anyDirty || saving || !!details?.readOnly} onclick={() => void save()}>{saving ? '儲存中…' : '儲存'}</button>
    </div>
  </header>

  <div class="track-details-body">
    {#if error}<p class="track-details-error" role="alert">{error}</p>{/if}
    {#if details?.readOnly}<p class="track-details-notice">{details.readOnlyReason ?? '此檔案為唯讀'}</p>{/if}

    {#if details && draft}
      <div class="track-details-path">
        <span class="track-details-path-label">路徑</span>
        <code title={details.path}>{details.path}</code>
        <button type="button" class="soft-button" onclick={() => onOpenSpectrogram(trackId)}>頻譜圖</button>
        <button type="button" class="soft-button" onclick={() => void navigator.clipboard.writeText(details!.path)}>複製路徑</button>
        {#if !isAndroid}
        <button type="button" class="soft-button" onclick={() => void invoke('track_details_reveal', { trackId }).catch((cause) => { error = String(cause); })}>在資料夾中顯示</button>
        {/if}
      </div>

      <dl class="track-details-props">
        <dt>格式</dt><dd>{formatAudioFormat({ codec: details.format })}</dd>
        <dt>標籤</dt><dd>{details.tagFormat || '—'}</dd>
        <dt>時長</dt><dd>{formatDuration(details.properties.durationMs)}</dd>
        <dt>位元率</dt><dd>{details.properties.audioBitrateKbps ?? details.properties.overallBitrateKbps ?? '—'} kbps</dd>
        <dt>取樣率</dt><dd>{details.properties.sampleRate ? `${details.properties.sampleRate} Hz` : '—'}</dd>
        <dt>位元深度</dt><dd>{details.properties.bitDepth ? `${details.properties.bitDepth} bit` : '—'}</dd>
        <dt>聲道</dt><dd>{details.properties.channels ?? '—'}</dd>
        <dt>修改時間</dt><dd>{formatDate(details.modifiedUnixMs)}</dd>
        <dt>檔案大小</dt><dd>{formatBytes(details.fileSize)}</dd>
        <dt>建立時間</dt><dd>{formatDate(details.createdUnixMs)}</dd>
      </dl>

      <section class="track-details-cover">
        <div class="track-details-cover-preview" class:is-dirty={coverDirty}>
          {#if removeCover}
            <span>（將移除封面）</span>
          {:else if pendingCoverUrl}
            <img src={pendingCoverUrl} alt="新封面預覽" />
          {:else if coverUrl}
            <img src={coverUrl} alt="目前封面" />
          {:else}
            <span>無封面</span>
          {/if}
        </div>
        <div class="track-details-cover-meta">
          <div class="track-details-row-buttons">
            <button type="button" class="soft-button" aria-label="選擇封面" title="選擇封面圖片（儲存後才寫入）" disabled={details.readOnly} onclick={() => void pickCover()}>選擇</button>
            <button type="button" class="soft-button" aria-label="匯出封面" title="匯出封面" disabled={!details.cover} onclick={() => void exportCover()}>匯出</button>
            <button type="button" class="soft-button" aria-label="複製封面" title="複製封面到剪貼簿" disabled={!details.cover} onclick={() => void copyCover()}>複製</button>
            <button type="button" class="soft-button" aria-label="貼上封面" title={clipboardImage ? '以剪貼簿圖片作為新封面（儲存後才寫入）' : '剪貼簿中沒有圖片'} disabled={details.readOnly || !clipboardImage} onclick={() => void pasteCover()}>貼上</button>
            <button type="button" class="soft-button" aria-label="移除封面" title="移除封面（儲存後才寫入）" disabled={details.readOnly || (!details.cover && !pendingCoverPath)} onclick={() => { removeCover = true; pendingCoverPath = null; pendingCoverBase64 = null; }}>移除</button>
          </div>
          <div class="track-details-cover-option-row">
            <label>JPG品質 <input type="number" min="1" max="100" value={coverOptions.jpegQuality} onchange={(event) => updateCoverOptions({ jpegQuality: Math.min(100, Math.max(1, Number(event.currentTarget.value) || 100)) })} />%</label>
            <label><input type="checkbox" checked={coverOptions.autoResize} onchange={(event) => updateCoverOptions({ autoResize: event.currentTarget.checked })} />自動縮放</label>
          </div>
          <label>短邊 <input type="number" min="64" max="10000" step="10" disabled={!coverOptions.autoResize} value={coverOptions.shortEdgePx} onchange={(event) => updateCoverOptions({ shortEdgePx: Math.max(64, Number(event.currentTarget.value) || 2160) })} /> px（保持長寬比）</label>
          {#if shownCover}
            <p>{shownCover.width}×{shownCover.height} · {shownCover.mime.replace('image/', '').toUpperCase()} · {formatBytes(shownCover.bytes)}{#if pendingCoverPath}（未儲存）{/if}</p>
            {#if pendingCoverPath && savedPlan?.reencode}<p class="track-details-cover-plan">儲存時：{savedPlan.width}×{savedPlan.height} JPG（品質 {coverOptions.jpegQuality}%）</p>{/if}
          {/if}
        </div>
      </section>



      <section class="track-details-fields">
        {#snippet fieldInput(field: { key: keyof TagFields; label: string }, wide: boolean)}
          <label class="track-details-field" class:is-wide={wide} class:is-dirty={isDirty(field.key)}>
            <span>{field.label}</span>
            <input type="text" bind:value={draft![field.key]} readonly={details!.readOnly} />
            {#if isDirty(field.key)}<button type="button" class="track-details-reset" aria-label={`還原${field.label}`} title="還原" onclick={() => resetField(field.key)}>↺</button>{/if}
          </label>
        {/snippet}
        {#each WIDE_FIELDS as field (field.key)}{@render fieldInput(field, true)}{/each}
        <div class="track-details-number-row">
          {#each NUMBER_ROW as field (field.key)}{@render fieldInput(field, false)}{/each}
        </div>
        <div class="track-details-field is-wide is-lyrics" class:is-dirty={isDirty('lyrics')}>
          <span class="track-details-lyrics-label">
            歌詞（內嵌）
            <button type="button" class="soft-button" disabled={details.readOnly || onlineLyricsText === null || onlineLyricsText === draft.lyrics} title={onlineLyricsText === null ? '此曲目目前沒有使用中的歌詞' : onlineLyricsText === draft.lyrics ? '內嵌歌詞已與使用中歌詞相同' : '以播放器目前使用的歌詞取代（線上、.lrc、手動選擇等；同步時間戳保留為 LRC，儲存後才寫入）'} onclick={importOnlineLyrics}>導入使用中歌詞</button>
            {#if isDirty('lyrics')}<button type="button" class="track-details-reset" aria-label="還原歌詞" title="還原" onclick={() => resetField('lyrics')}>↺</button>{/if}
          </span>
          <textarea rows="30" bind:value={draft.lyrics} readonly={details.readOnly} aria-label="歌詞"></textarea>
        </div>
      </section>
    {:else if !error}
      <p>讀取中…</p>
    {/if}
  </div>

</div>
</div>

<style>
  .track-details-scrim { position: fixed; inset: 0; z-index: 1000; border: 0; background: rgb(0 0 0 / 0.35); }
  .track-details {
    position: fixed; z-index: 1001; left: 50%; top: 50%; translate: -50% -50%;
    width: min(880px, calc(100vw - 32px - var(--android-safe-left, 0px) - var(--android-safe-right, 0px)));
    /* Android insets come from native (MainActivity -> --android-safe-*), env() is 0 there. */
    max-height: calc(100dvh - 64px - var(--android-safe-top, 0px) - var(--android-safe-bottom, 0px));
    display: flex; flex-direction: column; color: var(--text-primary, inherit);
  }
  .track-details-header { display: flex; align-items: center; gap: 8px; padding: 12px 18px 8px; min-width: 0; }
  .track-details-header h2 { margin: 0; font-size: 1.1rem; white-space: nowrap; flex: none; }
  .track-details-actions { display: flex; flex: none; gap: 6px; margin-left: auto; }
  .track-details-actions .soft-button { white-space: nowrap; }
  .track-details-path { display: flex; gap: 8px; align-items: center; min-width: 0; }
  .track-details-path-label { opacity: 0.7; font-size: 0.85rem; flex: none; }
  .track-details-path code { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; user-select: text; font-size: 0.85rem; opacity: 0.85; }
  .track-details-body { overflow: auto; padding: 8px 18px; display: grid; gap: 14px; }
  .track-details-error { color: #ff6b6b; margin: 0; }
  .track-details-notice { margin: 0; opacity: 0.8; }
  .track-details-cover { display: flex; gap: 14px; align-items: flex-start; flex-wrap: wrap; }
  .track-details-cover-preview { width: 160px; height: 160px; display: grid; place-items: center; border-radius: 8px; background: rgb(127 127 127 / 0.15); overflow: hidden; outline: 2px solid transparent; }
  .track-details-cover-preview.is-dirty { outline-color: var(--accent, #6aa7ff); }
  .track-details-cover-preview img { width: 100%; height: 100%; object-fit: contain; }
  .track-details-cover-meta { display: grid; gap: 6px; font-size: 0.9rem; }
  .track-details-cover-meta p { margin: 0; opacity: 0.8; }
  .track-details-cover-meta input[type='number'] { width: 80px; }
  .track-details-cover-option-row { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 18px; }
  .track-details-row-buttons { display: flex; gap: 6px; flex-wrap: wrap; }
  .track-details-layer { display: contents; }
  .track-details-fields { display: grid; grid-template-columns: minmax(0, 1fr); gap: 8px 12px; }
  /* Date / track / disc share one row; columns shrink (min 0) instead of overlapping. */
  .track-details-number-row { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px 12px; }
  .track-details-number-row input { min-width: 0; width: 100%; box-sizing: border-box; }
  .track-details-field { position: relative; display: grid; gap: 3px; font-size: 0.85rem; }
  .track-details-field.is-wide { grid-column: 1 / -1; }
  .track-details-field > span { opacity: 0.75; }
  .track-details-field input, .track-details-field textarea { font: inherit; font-size: 0.95rem; padding: 5px 8px; border-radius: 6px; border: 1px solid rgb(127 127 127 / 0.35); background: rgb(127 127 127 / 0.08); color: inherit; }
  .track-details-field textarea { resize: vertical; font-family: inherit; }
  /* app.css gives button/input a single 2px accent outline on :focus-visible; textarea was not
     covered, so the WebView's default focus ring was drawn against the field border (double
     outline). Give it the same ring as the inputs. */
  .track-details-field textarea:focus { outline: none; }
  .track-details-field textarea:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 3px; }
  .track-details-field.is-dirty input, .track-details-field.is-dirty textarea { border-color: var(--accent, #6aa7ff); }
  .track-details-reset { position: absolute; right: 6px; top: 22px; border: 0; background: none; color: inherit; cursor: pointer; opacity: 0.7; }
  .track-details-lyrics-label { display: flex; align-items: center; gap: 8px; opacity: 1 !important; }
  .track-details-lyrics-label .track-details-reset { position: static; }
  /* Two label/value pairs per row: 格式|標籤, 時長|位元率 … (rows split around the cover). */
  .track-details-props { display: grid; grid-template-columns: auto minmax(0, 1fr) auto minmax(0, 1fr); gap: 4px 12px; margin: 0; font-size: 0.85rem; align-items: center; }
  .track-details-props dt { opacity: 0.7; }
  .track-details-props dd { margin: 0; }
  .track-details-saved { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; opacity: 0.8; font-size: 0.85rem; }
  .is-primary { font-weight: 600; }
  @media (max-width: 820px) {
    .track-details { inset: 0; left: 0; top: 0; translate: none; width: 100vw; max-height: none; height: 100dvh; border-radius: 0; box-sizing: border-box; padding: var(--android-safe-top, 0px) var(--android-safe-right, 0px) var(--android-safe-bottom, 0px) var(--android-safe-left, 0px); }
  }
</style>
