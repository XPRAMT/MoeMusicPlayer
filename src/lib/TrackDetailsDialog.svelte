<script lang="ts">
  import { formatDetailsBitrate, formatDetailsDuration, formatDetailsSampleRate } from './track-details-format.js';
  import { tr } from './i18n.js';
  import { originalForPastedCover, pixelFingerprint, rememberCopiedCover } from './cover-clipboard.js';
  import { formatAudioFormat } from './track-columns.js';
  // 歌曲詳細資訊面板（ADR 0013 Phase 1）：單曲標籤檢視／暫存編輯，按「儲存」才寫檔。
  import { onDestroy, onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import type { PlaybackSnapshot } from './ipc';
  import { seekPlayback } from './ipc';
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
    tagFormat: string; readOnly: boolean; readOnlyReason: string | null; remote?: boolean; fields: TagFields;
    properties: { durationMs: number; overallBitrateKbps: number | null; audioBitrateKbps: number | null; sampleRate: number | null; bitDepth: number | null; channels: number | null };
    cover: { mime: string; width: number; height: number; bytes: number } | null;
  };

  // Android: 「在資料夾中開啟」 opens the containing folder (no app can select the file), and the
  // clipboard is read natively (WebView rejects navigator.clipboard.read() for images).
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
    { key: 'title', label: tr('標題') },
    { key: 'artist', label: tr('演出者') },
    { key: 'album', label: tr('專輯') },
    { key: 'albumArtist', label: tr('專輯演出者') },
    { key: 'composer', label: tr('作曲') },
    { key: 'genre', label: tr('類型') },
  ];
  const NUMBER_ROW: { key: keyof TagFields; label: string }[] = [
    { key: 'date', label: tr('錄音日期') },
    { key: 'trackNumber', label: tr('音軌號') },
    { key: 'discNumber', label: tr('碟號') },
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
  /** Overwrite confirmation shown before Save writes the file. */
  let confirmingSave = $state(false);
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

  let removeCover = $state(false);
  let onlineLyricsText = $state<string | null>(null);
  let coverOptions = $state<CoverEditOptions>(loadCoverEditOptions());

  const isDirty = (key: keyof TagFields) => !!draft && !!original && draft[key] !== original[key];
  // Cover facts shown next to the preview follow the staged state, not the loaded file.
  const shownCover = $derived(removeCover ? null : pendingCoverPath ? stagedCoverInfo : details?.cover ?? null);
  /** How Save will write the staged cover: Rust's player_tags::plan_cover, the same rule Save uses
   *  (JPG unless a pixel has alpha < 255, then PNG; unscaled JPG / transparent PNG keep their bytes). */
  type CoverPlan = { width: number; height: number; mime: string; reencode: boolean };
  let savedPlan = $state<CoverPlan | null>(null);
  let coverPlanRequest = 0;
  $effect(() => {
    const path = pendingCoverPath;
    const base64 = pendingCoverBase64;
    const options = { ...coverOptions };
    const request = ++coverPlanRequest;
    savedPlan = null;
    if (!path) return;
    const clipboard = path === CLIPBOARD_COVER;
    if (clipboard && !base64) return;
    invoke<CoverPlan>('track_details_cover_plan', { coverPath: clipboard ? null : path, coverBase64: clipboard ? base64 : null, coverOptions: options })
      .then((plan) => { if (request === coverPlanRequest) savedPlan = plan; })
      .catch(() => { /* undecodable: Save will report the error */ });
  });
  function coverPlanText(plan: CoverPlan): string {
    const kind = plan.mime === 'image/png' ? 'PNG' : 'JPG';
    if (!plan.reencode) return tr("儲存時：{0}×{1} {2}（原檔寫入，不重新編碼）", [plan.width, plan.height, kind]);
    if (kind === 'PNG') return tr("儲存時：{0}×{1} PNG（保留透明）", [plan.width, plan.height]);
    return tr("儲存時：{0}×{1} JPG（品質 {2}%）", [plan.width, plan.height, coverOptions.jpegQuality]);
  }
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

  /** Empties the lyrics field; like every other field it reaches the file only when 儲存 is pressed. */
  function clearLyrics() {
    if (draft) draft.lyrics = '';
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
  let clipboardHasImage = $state(false);

  async function readClipboardImage(): Promise<Blob | null> {
    if (isAndroid) {
      // Throws the native reason (not focused / no readable image) so 貼上 can show it.
      const bytes = await invoke<ArrayBuffer>('track_details_read_clipboard_image');
      return bytes && bytes.byteLength > 0 ? new Blob([bytes]) : null;
    }
    try {
      if (!navigator.clipboard?.read) return null;
      for (const item of await navigator.clipboard.read()) {
        const type = item.types.find((value) => value.startsWith('image/'));
        if (type) return await item.getType(type);
      }
    } catch { /* permission denied / not focused / unsupported */ }
    return null;
  }

  async function refreshClipboard() {
    if (isAndroid) {
      // Only the clip description is checked here (a hint: Android hides the clip while the app is
      // unfocused); 貼上 stays enabled on Android and reads the clipboard when pressed.
      try { clipboardHasImage = await invoke<boolean>('track_details_clipboard_has_image'); } catch { clipboardHasImage = false; }
      return;
    }
    clipboardImage = await readClipboardImage();
    clipboardHasImage = clipboardImage !== null;
  }
  function refreshClipboardWhenVisible() { if (document.visibilityState === 'visible') void refreshClipboard(); }

  async function blobToBase64(blob: Blob): Promise<string> {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = '';
    for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    return btoa(binary);
  }

  /** Fingerprint of an image's decoded pixels (see cover-clipboard.js); null when it cannot be decoded. */
  async function imageFingerprint(blob: Blob): Promise<string | null> {
    try {
      const bitmap = await createImageBitmap(blob);
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      const context = canvas.getContext('2d', { willReadFrequently: true })!;
      context.drawImage(bitmap, 0, 0);
      bitmap.close();
      return await pixelFingerprint(canvas.width, canvas.height, context.getImageData(0, 0, canvas.width, canvas.height).data);
    } catch {
      return null;
    }
  }

  async function pasteCover() {
    let blob: Blob | null;
    try { blob = clipboardImage ?? await readClipboardImage(); } catch (cause) { error = String(cause); return; }
    if (!blob) { error = tr('剪貼簿中沒有圖片'); return; }
    // A cover copied in this app comes back as its original bytes (no JPG re-encode on Save).
    blob = originalForPastedCover(await imageFingerprint(blob)) ?? blob;
    pendingCoverBase64 = await blobToBase64(blob);
    revoke(pendingCoverUrl);
    pendingCoverUrl = URL.createObjectURL(blob);
    void describeStaged(blob);
    pendingCoverPath = CLIPBOARD_COVER;
    removeCover = false;
  }

  /** Clipboard images must be PNG: re-encode the current cover through a canvas (the original bytes are remembered for 貼上). */
  async function copyCover() {
    if (isAndroid) {
      // WebView's navigator.clipboard.write() resolves for images without reaching the system
      // clipboard; the native side shares the cover as a content URI instead.
      try {
        const original = await invoke<ArrayBuffer>('track_details_cover', { trackId });
        await invoke('track_details_copy_cover', { trackId });
        if (original && original.byteLength > 0) {
          const blob = new Blob([original]);
          rememberCopiedCover(await imageFingerprint(blob), blob);
        }
        notice = tr('已複製封面');
        void refreshClipboard();
      } catch (cause) {
        error = tr("複製封面失敗：{0}", [String(cause)]);
      }
      return;
    }
    try {
      const bytes = await invoke<ArrayBuffer>('track_details_cover', { trackId });
      if (!bytes || bytes.byteLength === 0) { error = tr('此曲目沒有內嵌封面'); return; }
      const bitmap = await createImageBitmap(new Blob([bytes]));
      const canvas = document.createElement('canvas');
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      canvas.getContext('2d')!.drawImage(bitmap, 0, 0);
      bitmap.close();
      const original = new Blob([bytes]);
      rememberCopiedCover(await pixelFingerprint(canvas.width, canvas.height, canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data), original);
      const png = await new Promise<Blob>((resolve, reject) => canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error(tr('PNG 轉換失敗'))), 'image/png'));
      await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
      notice = tr('已複製封面');
      void refreshClipboard();
    } catch (cause) {
      error = tr("複製封面失敗：{0}", [String(cause)]);
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
      if (saved) notice = tr("已匯出：{0}", [saved]);
    } catch (cause) { error = String(cause); }
  }

  /** Save button / Ctrl+S: ask before overwriting the file. Cancel keeps all staged edits. */
  function requestSave() {
    if (!draft || saving || !anyDirty || details?.readOnly) return;
    confirmingSave = true;
  }

  function cancelSave() {
    confirmingSave = false;
  }

  function confirmSave() {
    confirmingSave = false;
    void save();
  }

  function focusOnMount(node: HTMLElement) {
    node.focus();
  }

  /**
   * A song on a server that is playing right now: its file is let go of for the upload (the server cannot replace a file
   * that is open), so playback is paused first and the song is loaded again at the same place afterwards, playing or paused
   * as it was. Returns the step that puts playback back, or `null` when there is nothing to put back.
   */
  async function holdPlaybackForRemoteSave(): Promise<(() => Promise<void>) | null> {
    if (!details?.remote) return null;
    try {
      const snapshot = await invoke<PlaybackSnapshot>('playback_get_snapshot');
      if (snapshot.currentTrack?.id !== trackId) return null;
      const position = snapshot.positionMs;
      const wasPlaying = snapshot.isPlaying;
      await invoke('playback_pause');
      return async () => {
        try {
          await invoke('playback_play', { trackId });
          await seekPlayback(position);
          if (!wasPlaying) await invoke('playback_pause');
        } catch { /* the song stays wherever playback stopped */ }
      };
    } catch {
      return null;
    }
  }

  async function save() {
    if (!draft || saving) return;
    saving = true; error = null; notice = null;
    const resumePlayback = await holdPlaybackForRemoteSave();
    try {
      const result = await invoke<TrackDetails>('track_details_save', {
        trackId,
        request: {
          fields: draft,
          replaceCoverPath: pendingCoverPath === CLIPBOARD_COVER ? null : pendingCoverPath,
          replaceCoverBase64: pendingCoverPath === CLIPBOARD_COVER ? pendingCoverBase64 : null,
          lyricsChanged: isDirty('lyrics'),
          removeCover,
          coverOptions,
        },
      });
      details = result;
      original = { ...result.fields };
      draft = { ...result.fields };
      pendingCoverPath = null; pendingCoverBase64 = null; revoke(pendingCoverUrl); pendingCoverUrl = null; removeCover = false;
      await loadCover();
      notice = tr('已儲存');
      onSaved?.();
    } catch (cause) {
      error = String(cause);
    } finally {
      saving = false;
      await resumePlayback?.();
    }
  }

  function requestClose() {
    // Esc / Android back / scrim first dismiss the overwrite confirmation (edits are kept).
    if (confirmingSave) { cancelSave(); return; }
    if (anyDirty && !confirm(tr('有尚未儲存的變更，確定要關閉嗎？'))) return;
    onClose();
  }

  function updateCoverOptions(patch: Partial<CoverEditOptions>) {
    coverOptions = { ...coverOptions, ...patch };
    saveCoverEditOptions(coverOptions);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') { event.preventDefault(); requestClose(); }
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); requestSave(); }
  }

  const formatBytes = (bytes: number) => bytes >= 1048576 ? `${(bytes / 1048576).toFixed(2)} MB` : `${Math.round(bytes / 1024)} KB`;
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
  onMount(() => { window.addEventListener('keydown', onKeydown); window.addEventListener('moe:track-details-request-close', requestClose); window.addEventListener('focus', refreshClipboard); document.addEventListener('visibilitychange', refreshClipboardWhenVisible); void refreshClipboard(); });
  onDestroy(() => { window.removeEventListener('keydown', onKeydown); window.removeEventListener('moe:track-details-request-close', requestClose); window.removeEventListener('focus', refreshClipboard); document.removeEventListener('visibilitychange', refreshClipboardWhenVisible); revoke(coverUrl); revoke(pendingCoverUrl); });
</script>

<div class="track-details-layer" use:portal>
<button class="track-details-scrim" type="button" tabindex="-1" aria-label={tr("關閉歌曲資訊")} onclick={requestClose}></button>
<div class="track-details floating-surface" role="dialog" aria-modal="true" aria-label={tr("歌曲資訊")}>
  <header class="track-details-header">
    <h2>{tr("歌曲資訊")}</h2>
    {#if notice}<span class="track-details-saved">{notice}</span>{/if}
    <div class="track-details-actions">
      <button type="button" class="soft-button" onclick={() => onOpenSpectrogram(trackId)}>{tr("頻譜圖")}</button>
      <button type="button" class="soft-button" disabled={!anyDirty || saving} onclick={resetAll}>{tr("還原")}</button>
      <button type="button" class="soft-button is-primary" disabled={!anyDirty || saving || !!details?.readOnly} onclick={requestSave}>{saving ? tr('儲存中…') : tr('儲存')}</button>
    </div>
  </header>

  <div class="track-details-body">
    {#if error}<p class="track-details-error" role="alert">{error}</p>{/if}
    {#if details?.readOnly}<p class="track-details-notice">{details.readOnlyReason ?? tr('此檔案為唯讀')}</p>{/if}

    {#if details && draft}
      <div class="track-details-path">
        <span class="track-details-path-label">{tr("路徑")}</span>
        <code title={details.path}>{details.path}</code>
        <button type="button" class="soft-button" onclick={() => void navigator.clipboard.writeText(details!.path)}>{tr("複製路徑")}</button>
        <button type="button" class="soft-button" disabled={details.remote} onclick={() => void invoke('track_details_reveal', { trackId }).catch((cause) => { error = String(cause); })}>{tr("在資料夾中開啟")}</button>
      </div>

      <dl class="track-details-props">
        <dt>{tr("格式")}</dt><dd>{details.format}</dd>
        <dt>{tr("標籤")}</dt><dd>{details.tagFormat || '—'}</dd>
        <dt>{tr("時長")}</dt><dd>{formatDetailsDuration(details.properties.durationMs)}</dd>
        <dt>{tr("位元率")}</dt><dd>{formatDetailsBitrate(details.properties.audioBitrateKbps ?? details.properties.overallBitrateKbps)}</dd>
        <dt>{tr("取樣率")}</dt><dd>{formatDetailsSampleRate(details.properties.sampleRate)}</dd>
        <dt>{tr("位元深度")}</dt><dd>{details.properties.bitDepth ? `${details.properties.bitDepth} bit` : '—'}</dd>
        <dt>{tr("聲道")}</dt><dd>{details.properties.channels ?? '—'}</dd>
        <dt>{tr("修改時間")}</dt><dd>{formatDate(details.modifiedUnixMs)}</dd>
        <dt>{tr("檔案大小")}</dt><dd>{formatBytes(details.fileSize)}</dd>
        <dt>{tr("建立時間")}</dt><dd>{formatDate(details.createdUnixMs)}</dd>
      </dl>

      <section class="track-details-cover">
        <div class="track-details-cover-preview" class:is-dirty={coverDirty}>
          {#if removeCover}
            <span>{tr("（將移除封面）")}</span>
          {:else if pendingCoverUrl}
            <img src={pendingCoverUrl} alt={tr("新封面預覽")} />
          {:else if coverUrl}
            <img src={coverUrl} alt={tr("目前封面")} />
          {:else}
            <span>{tr("無封面")}</span>
          {/if}
        </div>
        <div class="track-details-cover-meta">
          <div class="track-details-row-buttons">
            <button type="button" class="soft-button" aria-label={tr("選擇封面")} title={tr("選擇封面圖片（儲存後才寫入）")} disabled={details.readOnly} onclick={() => void pickCover()}>{tr("選擇")}</button>
            <button type="button" class="soft-button" aria-label={tr("匯出封面")} title={tr("匯出封面")} disabled={!details.cover} onclick={() => void exportCover()}>{tr("匯出")}</button>
            <button type="button" class="soft-button" aria-label={tr("複製封面")} title={tr("複製封面到剪貼簿")} disabled={!details.cover} onclick={() => void copyCover()}>{tr("複製")}</button>
            <button type="button" class="soft-button" aria-label={tr("貼上封面")} title={clipboardHasImage ? tr('以剪貼簿圖片作為新封面（儲存後才寫入）') : isAndroid ? tr('讀取剪貼簿圖片作為新封面（儲存後才寫入）') : tr('剪貼簿中沒有圖片')} disabled={details.readOnly || (!isAndroid && !clipboardHasImage)} onclick={() => void pasteCover()}>{tr("貼上")}</button>
            <button type="button" class="soft-button" aria-label={tr("移除封面")} title={tr("移除封面（儲存後才寫入）")} disabled={details.readOnly || (!details.cover && !pendingCoverPath)} onclick={() => { removeCover = true; pendingCoverPath = null; pendingCoverBase64 = null; }}>{tr("移除")}</button>
          </div>
          <div class="track-details-cover-option-row">
            <label>{tr("JPG品質")} <input type="number" min="1" max="100" value={coverOptions.jpegQuality} onchange={(event) => updateCoverOptions({ jpegQuality: Math.min(100, Math.max(1, Number(event.currentTarget.value) || 100)) })} />%</label>
            <label><input type="checkbox" checked={coverOptions.autoResize} onchange={(event) => updateCoverOptions({ autoResize: event.currentTarget.checked })} />{tr("自動縮放")}</label>
          </div>
          <label>{tr("短邊")} <input type="number" min="64" max="10000" step="10" disabled={!coverOptions.autoResize} value={coverOptions.shortEdgePx} onchange={(event) => updateCoverOptions({ shortEdgePx: Math.max(64, Number(event.currentTarget.value) || 2160) })} /> {tr("px（保持長寬比）")}</label>
          {#if shownCover}
            <p>{shownCover.width}×{shownCover.height} · {shownCover.mime.replace('image/', '').toUpperCase()} · {formatBytes(shownCover.bytes)}{#if pendingCoverPath}{tr("（未儲存）")}{/if}</p>
            {#if pendingCoverPath && savedPlan}<p class="track-details-cover-plan">{coverPlanText(savedPlan)}</p>{/if}
          {/if}
        </div>
      </section>



      <section class="track-details-fields">
        {#snippet fieldInput(field: { key: keyof TagFields; label: string }, wide: boolean)}
          <label class="track-details-field" class:is-wide={wide} class:is-dirty={isDirty(field.key)}>
            <span>{field.label}</span>
            <input type="text" bind:value={draft![field.key]} readonly={details!.readOnly} />
            {#if isDirty(field.key)}<button type="button" class="track-details-reset" aria-label={tr("還原{0}", [field.label])} title={tr("還原")} onclick={() => resetField(field.key)}>↺</button>{/if}
          </label>
        {/snippet}
        {#each WIDE_FIELDS as field (field.key)}{@render fieldInput(field, true)}{/each}
        <div class="track-details-number-row">
          {#each NUMBER_ROW as field (field.key)}{@render fieldInput(field, false)}{/each}
        </div>
        <div class="track-details-field is-wide is-lyrics" class:is-dirty={isDirty('lyrics')}>
          <span class="track-details-lyrics-label">
            {tr("歌詞（內嵌）")}
            <button type="button" class="soft-button" disabled={details.readOnly || onlineLyricsText === null || onlineLyricsText === draft.lyrics} title={onlineLyricsText === null ? tr('此曲目目前沒有使用中的歌詞') : onlineLyricsText === draft.lyrics ? tr('內嵌歌詞已與使用中歌詞相同') : tr('以播放器目前使用的歌詞取代（線上、.lrc、手動選擇等；同步時間戳保留為 LRC，儲存後才寫入）')} onclick={importOnlineLyrics}>{tr("導入使用中歌詞")}</button>
            <button type="button" class="soft-button" disabled={details.readOnly || draft.lyrics === ''} title={tr('清空歌詞欄位（儲存後才會從檔案移除內嵌歌詞）')} onclick={clearLyrics}>{tr("清空")}</button>
            {#if isDirty('lyrics')}<button type="button" class="track-details-reset" aria-label={tr("還原歌詞")} title={tr("還原")} onclick={() => resetField('lyrics')}>↺</button>{/if}
          </span>
          <textarea rows="30" bind:value={draft.lyrics} readonly={details.readOnly} aria-label={tr("歌詞")}></textarea>
        </div>
      </section>
    {:else if !error}
      <p>{tr("讀取中…")}</p>
    {/if}
  </div>

</div>
{#if confirmingSave}
  <button class="track-details-confirm-scrim" type="button" tabindex="-1" aria-label={tr("取消儲存")} onclick={cancelSave}></button>
  <div class="track-details-confirm floating-surface" role="alertdialog" aria-modal="true" aria-labelledby="track-details-confirm-text" data-testid="track-details-save-confirm">
    <p id="track-details-confirm-text">{tr("將會覆寫檔案原有資訊，是否繼續？")}</p>
    <div class="track-details-confirm-actions">
      <button type="button" class="soft-button" onclick={cancelSave}>{tr("取消")}</button>
      <button type="button" class="soft-button is-primary" use:focusOnMount onclick={confirmSave}>{tr("確認")}</button>
    </div>
  </div>
{/if}
</div>

<style>
  .track-details-confirm-scrim { position: fixed; inset: 0; z-index: 1002; border: 0; background: rgb(0 0 0 / 0.2); }
  .track-details-confirm {
    position: fixed; z-index: 1003; left: 50%; top: 50%; translate: -50% -50%;
    width: min(360px, calc(100vw - 32px - var(--android-safe-left, 0px) - var(--android-safe-right, 0px)));
    padding: 18px 18px 14px; display: grid; gap: 14px;
  }
  .track-details-confirm p { margin: 0; line-height: 1.5; }
  .track-details-confirm-actions { display: flex; justify-content: flex-end; gap: 8px; }
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
