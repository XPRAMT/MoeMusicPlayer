<script lang="ts">
  import { tr } from './i18n.js';
  import type { NowPlayingLayout } from './ipc';

  interface Props {
    layout: NowPlayingLayout;
    variant?: 'compact' | 'full';
    onChange: (layout: NowPlayingLayout) => void;
  }

  let { layout, variant = 'full', onChange }: Props = $props();
</script>

<div
  class={variant === 'compact' ? 'now-playing-layout-switch' : 'layout-choice-row'}
  role="group"
  aria-label={tr("正在播放頁排列")}
>
  {#if variant === 'compact'}<span>{tr("排列")}</span>{/if}
  <button
    type="button"
    aria-label={tr("排列 A：封面在前，歌詞在後")}
    aria-pressed={layout === 'a'}
    class:chosen={layout === 'a'}
    onclick={() => onChange('a')}
  >
    {#if variant === 'full'}<strong>{tr("排列 A")}</strong><span>{tr("封面在前，歌詞在後")}</span>{:else}A{/if}
  </button>
  <button
    type="button"
    aria-label={tr("排列 B：歌詞在前，封面在後")}
    aria-pressed={layout === 'b'}
    class:chosen={layout === 'b'}
    onclick={() => onChange('b')}
  >
    {#if variant === 'full'}<strong>{tr("排列 B")}</strong><span>{tr("歌詞在前，封面在後")}</span>{:else}B{/if}
  </button>
</div>
