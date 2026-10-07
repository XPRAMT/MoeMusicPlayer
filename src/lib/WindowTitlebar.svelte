<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import type { UnlistenFn } from '@tauri-apps/api/event';

  type Props = {
    transparent?: boolean;
  };

  let { transparent = false }: Props = $props();

  const LABEL_CONTROLS = '\u8996\u7A97\u63A7\u5236';
  const LABEL_MINIMIZE = '\u6700\u5C0F\u5316';
  const LABEL_MAXIMIZE = '\u6700\u5927\u5316';
  const LABEL_RESTORE = '\u9084\u539F';
  const LABEL_CLOSE = '\u95DC\u9589';
  const LABEL_ENTER_FULLSCREEN = '\u5168\u87a2\u5e55';
  const LABEL_EXIT_FULLSCREEN = '\u7d50\u675f\u5168\u87a2\u5e55';

  let maximized = $state(false);
  let fullscreen = $state(false);
  let restoreMaximizedAfterFullscreen = false;
  let unlistenResize: UnlistenFn | undefined;

  const appWindow = (() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  })();

  async function refreshWindowChromeState(): Promise<void> {
    if (!appWindow) return;
    try {
      maximized = await appWindow.isMaximized();
    } catch {
      maximized = false;
    }
    try {
      fullscreen = await appWindow.isFullscreen();
    } catch {
      fullscreen = false;
    }
  }

  function onToggleFullscreenRequest(): void {
    void toggleFullscreen();
  }

  onMount(() => {
    window.addEventListener('moemusicplayer-toggle-fullscreen', onToggleFullscreenRequest);
    void refreshWindowChromeState();
    if (!appWindow) return;
    void appWindow
      .onResized(() => {
        void refreshWindowChromeState();
      })
      .then((unlisten) => {
        unlistenResize = unlisten;
      })
      .catch(() => {
        /* harness / non-desktop */
      });
  });

  onDestroy(() => {
    window.removeEventListener('moemusicplayer-toggle-fullscreen', onToggleFullscreenRequest);
    if (typeof unlistenResize === 'function') unlistenResize();
  });

  async function minimize(): Promise<void> {
    try {
      await appWindow?.minimize();
    } catch {
      /* ignore */
    }
  }

  async function toggleMaximize(): Promise<void> {
    try {
      await appWindow?.toggleMaximize();
      await refreshWindowChromeState();
    } catch {
      /* ignore */
    }
  }

  async function toggleFullscreen(): Promise<void> {
    if (!appWindow) return;
    try {
      const entering = !(await appWindow.isFullscreen());
      if (entering) {
        // A maximized borderless window keeps the maximized client clipped to
        // the work area, so fullscreen cannot cover the taskbar until that
        // state is cleared.
        restoreMaximizedAfterFullscreen = await appWindow.isMaximized();
        if (restoreMaximizedAfterFullscreen) {
          await appWindow.unmaximize();
        }
        await appWindow.setFullscreen(true);
      } else {
        await appWindow.setFullscreen(false);
        if (restoreMaximizedAfterFullscreen) {
          restoreMaximizedAfterFullscreen = false;
          await appWindow.maximize();
        }
      }
      await refreshWindowChromeState();
    } catch {
      /* ignore */
    }
  }

  async function close(): Promise<void> {
    try {
      await appWindow?.close();
    } catch {
      /* ignore */
    }
  }
</script>

<div
  class="window-titlebar"
  class:is-transparent={transparent}
  data-testid="window-titlebar"
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="window-titlebar-drag"
    data-tauri-drag-region
    title="MoeMusicPlayer"
    ondblclick={() => void toggleMaximize()}
  >
    <span class="window-titlebar-label" data-tauri-drag-region>MoeMusicPlayer</span>
  </div>
  <div class="window-titlebar-controls" role="group" aria-label={LABEL_CONTROLS}>
    <button
      type="button"
      class="window-titlebar-button"
      class:is-active={fullscreen}
      aria-label={fullscreen ? LABEL_EXIT_FULLSCREEN : LABEL_ENTER_FULLSCREEN}
      title={fullscreen ? LABEL_EXIT_FULLSCREEN : LABEL_ENTER_FULLSCREEN}
      aria-pressed={fullscreen}
      onclick={() => void toggleFullscreen()}
      data-testid="window-titlebar-fullscreen"
    >
      {#if fullscreen}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M1.2 3.2H3.2V1.2M6.8 1.2V3.2H8.8M8.8 6.8H6.8V8.8M3.2 8.8V6.8H1.2"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M1.2 3.2V1.2H3.2M6.8 1.2H8.8V3.2M8.8 6.8V8.8H6.8M3.2 8.8H1.2V6.8"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      {/if}
    </button>
    <button
      type="button"
      class="window-titlebar-button"
      aria-label={LABEL_MINIMIZE}
      title={LABEL_MINIMIZE}
      onclick={() => void minimize()}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <path d="M1 5h8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      </svg>
    </button>
    <button
      type="button"
      class="window-titlebar-button"
      aria-label={maximized ? LABEL_RESTORE : LABEL_MAXIMIZE}
      title={maximized ? LABEL_RESTORE : LABEL_MAXIMIZE}
      onclick={() => void toggleMaximize()}
    >
      {#if maximized}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path
            d="M3 2.2h4.8V7H3zM2.2 3.2H1.5V8.5H6.8V7.8"
            fill="none"
            stroke="currentColor"
            stroke-width="1.1"
          />
        </svg>
      {:else}
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <rect x="1.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.2" />
        </svg>
      {/if}
    </button>
    <button
      type="button"
      class="window-titlebar-button is-close"
      aria-label={LABEL_CLOSE}
      title={LABEL_CLOSE}
      onclick={() => void close()}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <path d="M2 2l6 6M8 2L2 8" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
      </svg>
    </button>
  </div>
</div>