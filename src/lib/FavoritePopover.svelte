<script lang="ts">
  import { tr } from './i18n.js';
  import { tick } from 'svelte';
  import { IconHeart, IconHeartCheck, IconPlus } from '@tabler/icons-svelte-runes';
  import { invokeCommand, type FavoritePlaylist, type FavoritePlaylists } from './ipc';
  import { getPopoverHorizontalShiftPx } from './volume-popover-position.js';
  import { readSafeInsets } from './safe-viewport.js';
  import { appTooltip } from './app-tooltip.js';

  interface Props {
    trackId: string | null;
    platform: 'windows' | 'android' | 'unsupported';
    onChange?: () => void | Promise<void>;
  }

  let { trackId, platform, onChange }: Props = $props();
  let open = $state(false);
  let playlists = $state<FavoritePlaylist[]>([]);
  let isFavorited = $state(false);
  let setupRequired = $state(false);
  let loading = $state(false);
  let busySourceId = $state<string | null>(null);
  let error = $state<string | null>(null);
  let requestRevision = 0;
  let notifiedWindowsSources = false;
  let lastObservedTrackId: string | null = null;
  let favoriteButtonElement = $state<HTMLButtonElement | null>(null);
  let favoriteControlElement = $state<HTMLDivElement | null>(null);
  let favoritePopoverElement = $state<HTMLDivElement | null>(null);
  let favoritePopoverShiftX = $state(0);

  $effect(() => {
    const currentTrackId = trackId;
    const isOpen = open;
    if (!currentTrackId) {
      if (lastObservedTrackId !== null) {
        lastObservedTrackId = null;
        requestRevision += 1;
        playlists = [];
        isFavorited = false;
        setupRequired = false;
        loading = false;
        error = null;
      }
      return;
    }

    if (currentTrackId !== lastObservedTrackId) {
      lastObservedTrackId = currentTrackId;
      playlists = [];
      setupRequired = false;
      error = null;
      void refresh(currentTrackId);
    } else if (isOpen) {
      void refresh(currentTrackId);
    }
  });

  $effect(() => {
    if (!open) {
      favoritePopoverShiftX = 0;
      return;
    }

    let active = true;
    const updateAlignment = (): void => {
      if (!active || !favoriteButtonElement || !favoriteControlElement || !favoritePopoverElement) return;
      if (getComputedStyle(favoriteButtonElement).display === 'none') {
        open = false;
        return;
      }
      const anchor = favoriteControlElement.getBoundingClientRect();
      favoritePopoverShiftX = getPopoverHorizontalShiftPx(
        anchor.left + anchor.width / 2,
        favoritePopoverElement.getBoundingClientRect().width,
        window.innerWidth,
        undefined,
        readSafeInsets(),
      );
    };

    window.addEventListener('resize', updateAlignment);
    void tick().then(updateAlignment);
    return () => {
      active = false;
      window.removeEventListener('resize', updateAlignment);
    };
  });

  async function refresh(currentTrackId: string | null = trackId): Promise<void> {
    if (!currentTrackId) return;
    const revision = ++requestRevision;
    loading = true;
    error = null;
    try {
      const result: FavoritePlaylists = await invokeCommand('favorites_get_playlists', { trackId: currentTrackId });
      if (revision !== requestRevision) return;
      playlists = result.playlists;
      isFavorited = result.playlists.some((playlist) => playlist.selected);
      setupRequired = result.myLoveSetupRequired;
      if (platform === 'windows' && !notifiedWindowsSources && !result.myLoveSetupRequired) {
        notifiedWindowsSources = true;
        await onChange?.();
      }
    } catch (cause) {
      if (revision === requestRevision) {
        error = cause instanceof Error ? cause.message : String(cause);
        playlists = [];
        isFavorited = false;
      }
    } finally {
      if (revision === requestRevision) loading = false;
    }
  }

  function toggle(event: MouseEvent): void {
    event.stopPropagation();
    if (!trackId || platform === 'unsupported') return;
    open = !open;
  }

  async function changeMembership(playlist: FavoritePlaylist, selected: boolean): Promise<void> {
    if (!trackId || busySourceId) return;
    busySourceId = playlist.sourceId;
    error = null;
    try {
      await invokeCommand('favorites_set_membership', {
        sourceId: playlist.sourceId,
        trackId,
        selected,
      });
      await onChange?.();
      await refresh();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      await refresh();
    } finally {
      busySourceId = null;
    }
  }

  async function createMyLove(): Promise<void> {
    if (loading) return;
    loading = true;
    error = null;
    try {
      await invokeCommand('favorites_create_my_love', {});
      await onChange?.();
      await refresh();
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  }

  async function createPlaylist(): Promise<void> {
    if (loading) return;
    loading = true;
    error = null;
    try {
      const name = await invokeCommand('favorites_create_playlist', {});
      if (name) {
        await onChange?.();
        await refresh();
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open || !favoriteControlElement) return;
    const target = event.target;
    if (target instanceof Node && favoriteControlElement.contains(target)) return;
    open = false;
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div class="favorite-control" bind:this={favoriteControlElement}>
  <button
    class="dock-favorite control-button"
    class:has-favorites={isFavorited}
    type="button"
    bind:this={favoriteButtonElement}
    aria-label={tr("收藏曲目")}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-pressed={isFavorited}
    use:appTooltip={tr('加入或移出播放清單')}
    disabled={!trackId || platform === 'unsupported'}
    onclick={toggle}
  >
    {#if isFavorited}
      <IconHeartCheck size={21} stroke={1.6} aria-hidden="true" />
    {:else}
      <IconHeart size={21} stroke={1.6} aria-hidden="true" />
    {/if}
  </button>
  {#if open}
    <div
      class="favorite-popover floating-surface"
      role="dialog"
      tabindex="-1"
      aria-label={tr("收藏到播放清單")}
      data-testid="favorite-popover"
      bind:this={favoritePopoverElement}
      style:--favorite-popover-shift-x={`${favoritePopoverShiftX}px`}
    >
      <header class="favorite-popover-heading">
        <strong>{tr("加入播放清單")}</strong>
        <button
          class="favorite-close soft-button icon-button"
          type="button"
          aria-label={tr("建立播放清單")}
          use:appTooltip={tr('建立播放清單')}
          disabled={loading || busySourceId !== null}
          onclick={() => void createPlaylist()}
        ><IconPlus size={18} stroke={1.8} aria-hidden="true" /></button>
      </header>
      {#if setupRequired}
        <div class="favorite-setup">
          <p>{tr("建立「我的最愛」播放清單。建議選擇公用 Music 資料夾，也可以在資料夾選擇器中選擇其他位置。")}</p>
          <button class="soft-button" type="button" disabled={loading} onclick={() => void createMyLove()}>
            {loading ? tr('處理中…') : platform === 'android' ? tr('選擇資料夾並建立') : tr('建立我的最愛')}
          </button>
        </div>
      {/if}
      {#if loading && playlists.length === 0}
        <p class="favorite-message" aria-hidden="true"></p>
      {:else if playlists.length === 0 && !setupRequired}
        <p class="favorite-message">{tr("目前沒有可用的播放清單來源。")}</p>
      {:else if playlists.length > 0}
        <ul class="favorite-playlist-list">
          {#each playlists as playlist (playlist.sourceId)}
            <li>
              <label class="favorite-playlist-option">
                <input
                  type="checkbox"
                  checked={playlist.selected}
                  disabled={!playlist.writable || loading || busySourceId !== null}
                  aria-label={tr("加入{0}", [playlist.displayName])}
                  onchange={(event) => void changeMembership(playlist, event.currentTarget.checked)}
                />
                <span>{playlist.displayName}</span>
                {#if !playlist.writable}
                  <small>
                    {#if playlist.reason === 'notOnServer'}
                      {tr("這首歌不在這台伺服器上")}
                    {:else if playlist.reason === 'noAccount'}
                      {tr("找不到這台伺服器的帳號設定")}
                    {:else}
                      {tr("請到「設定 → 音樂來源」重新加入，以授予寫入權限")}
                    {/if}
                  </small>
                {/if}
              </label>
            </li>
          {/each}
        </ul>
      {/if}
      {#if error}
        <p class="favorite-error" role="alert">{error}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .favorite-control {
    position: relative;
    display: flex;
    flex: 0 0 auto;
  }

  .dock-favorite.has-favorites :global(svg) {
    fill: none;
  }

  .favorite-popover {
    position: absolute;
    z-index: 55;
    left: 50%;
    bottom: calc(100% + 14px);
    width: min(310px, calc(100vw - 24px));
    max-height: min(60vh, 420px);
    overflow-y: auto;
    padding: 12px;
    transform: translateX(calc(-50% + var(--favorite-popover-shift-x, 0px)));
  }

  .favorite-popover-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 8px;
  }

  .favorite-popover-heading strong,
  .favorite-playlist-option span {
    min-width: 0;
    overflow-wrap: anywhere;
    font-size: var(--font-body);
  }

  .favorite-close {
    width: 30px;
    min-width: 30px;
    height: 30px;
    padding: 0;
    font-size: var(--font-body);
  }

  .favorite-playlist-list {
    display: grid;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .favorite-playlist-option {
    display: grid;
    grid-template-columns: 18px minmax(0, 1fr);
    align-items: center;
    gap: 9px;
    min-height: 38px;
    padding: 5px 4px;
    border-radius: 8px;
    cursor: pointer;
  }

  .favorite-playlist-option:hover {
    background: color-mix(in srgb, var(--text) 7%, transparent);
  }

  .favorite-playlist-option input {
    width: 16px;
    height: 16px;
    accent-color: var(--accent);
  }

  .favorite-playlist-option small {
    grid-column: 2;
    color: var(--text-muted);
    font-size: var(--font-caption);
  }

  .favorite-message,
  .favorite-setup p,
  .favorite-error {
    margin: 8px 0;
    color: var(--text-soft);
    font-size: var(--font-caption);
    line-height: 1.5;
  }

  .favorite-setup .soft-button {
    width: 100%;
    margin-top: 4px;
  }

  .favorite-error {
    color: var(--danger, #ff8f8f);
  }
</style>
