<script lang="ts">
  // 浮動歌詞視窗（ADR 0032）：和工作列歌詞同一個繪製元件（TaskbarLyricsView variant="floating"），外加鎖定／解鎖按鈕。
  // 移動模式：整個視窗可拖曳，滑鼠移上去顯示鎖頭按鈕。固定模式：Rust 讓視窗穿透滑鼠，並在游標位於視窗上時送
  // `floating-lyrics-hover`，只有解鎖按鈕的矩形會接收點擊。
  import { onDestroy, onMount } from 'svelte';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import TaskbarLyricsView from './TaskbarLyricsView.svelte';
  import { invokeCommand } from './ipc';
  import { tr } from './i18n.js';
  import {
    FLOATING_LYRICS_AREA_SHARE,
    floatingLyricsLockPx,
    floatingLyricsMarginPx,
    normalizeFloatingLyricsSettings,
  } from './floating-lyrics-settings.js';

  let locked = $state(false);
  let fontSizePx = $state(32);
  let hover = $state(false);
  let fixedHover = $state(false);
  let busy = false;
  const unlisteners: UnlistenFn[] = [];

  async function toggleLock(event: MouseEvent): Promise<void> {
    event.stopPropagation();
    if (busy) return;
    busy = true;
    try {
      const saved = await invokeCommand('floating_lyrics_set_locked', { locked: !locked });
      locked = normalizeFloatingLyricsSettings(saved).locked;
      fixedHover = false;
    } finally {
      busy = false;
    }
  }

  function startDrag(event: PointerEvent): void {
    if (locked || event.button !== 0) return;
    if ((event.target as HTMLElement | null)?.closest('button')) return;
    void getCurrentWindow().startDragging();
  }

  onMount(() => {
    void invokeCommand('settings_get_floating_lyrics', {}).then((stored) => {
      ({ locked, fontSizePx } = normalizeFloatingLyricsSettings(stored));
    });
    void listen<unknown>('floating-lyrics-settings', (event) => {
      ({ locked, fontSizePx } = normalizeFloatingLyricsSettings(event.payload));
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<boolean>('floating-lyrics-hover', (event) => {
      fixedHover = event.payload === true;
    }).then((unlisten) => unlisteners.push(unlisten));
  });

  onDestroy(() => {
    for (const unlisten of unlisteners) unlisten();
  });

  let showButton = $derived(locked ? fixedHover : hover);
  // The lock gutter on the right (floating_lyrics.rs `lock_css`); only the lyric area left of it gets a background.
  let lockPx = $derived(floatingLyricsLockPx(fontSizePx));
  // Transparent room for the shadow / glow: a side margin, and (1 - 1.1 / 1.5) / 2 of the height above and below.
  let marginPx = $derived(floatingLyricsMarginPx(fontSizePx));
  const areaInsetPercent = ((1 - FLOATING_LYRICS_AREA_SHARE) / 2) * 100;
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="floating-lyrics"
  class:movable={!locked}
  class:highlight={showButton}
  onpointerdown={startDrag}
  onpointerenter={() => { hover = true; }}
  onpointerleave={() => { hover = false; }}
  style:--floating-button-size={`${lockPx}px`}
  style:--floating-margin={`${marginPx}px`}
  style:--floating-area-inset={`${areaInsetPercent}%`}
>
  <div class="floating-lyrics-area"></div>
  <TaskbarLyricsView variant="floating" />
  {#if showButton}
    <button
      class="floating-lyrics-lock"
      type="button"
      title={locked ? tr('解除鎖定（可移動）') : tr('鎖定位置（滑鼠穿透）')}
      aria-label={locked ? tr('解除鎖定（可移動）') : tr('鎖定位置（滑鼠穿透）')}
      onclick={toggleLock}
    >
      <svg viewBox="0 0 24 24" width="62%" height="62%" aria-hidden="true">
        <rect x="5" y="11" width="14" height="10" rx="2" fill="currentColor" />
        {#if locked}
          <path d="M8 11V8a4 4 0 0 1 7.6-1.7" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
        {:else}
          <path d="M8 11V8a4 4 0 0 1 8 0v3" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" />
        {/if}
      </svg>
    </button>
  {/if}
</div>

<style>
  .floating-lyrics {
    position: fixed;
    inset: 0;
  }

  .floating-lyrics.movable {
    cursor: move;
  }

  /* The lyric area only: the lock gutter on the right stays transparent. */
  .floating-lyrics-area {
    position: absolute;
    inset: var(--floating-area-inset) calc(var(--floating-button-size) + var(--floating-margin)) var(--floating-area-inset)
      var(--floating-margin);
    border-radius: 10px;
    transition: background-color 120ms ease;
  }

  .floating-lyrics.movable.highlight .floating-lyrics-area {
    background: rgba(0, 0, 0, 0.28);
  }

  .floating-lyrics-lock {
    position: absolute;
    top: 50%;
    right: var(--floating-margin);
    box-sizing: border-box;
    width: var(--floating-button-size);
    height: var(--floating-button-size);
    margin-top: calc(var(--floating-button-size) / -2);
    display: grid;
    place-items: center;
    padding: 0;
    border: 1px solid rgba(255, 255, 255, 0.35);
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    cursor: pointer;
  }

  .floating-lyrics-lock:hover {
    background: rgba(0, 0, 0, 0.75);
  }
</style>