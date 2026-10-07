<script lang="ts">
  import { onMount } from 'svelte';
  import { fitMiddleEllipsis } from './middle-ellipsis.js';

  interface Props {
    text: string;
  }

  let { text }: Props = $props();
  let host = $state<HTMLElement | null>(null);
  let shown = $state('');

  const canvas = typeof document === 'undefined' ? null : document.createElement('canvas');

  function measure(value: string, font: string): number {
    const context = canvas?.getContext('2d');
    if (!context) return value.length;
    context.font = font;
    return context.measureText(value).width;
  }

  function layout(): void {
    if (!host) return;
    const font = getComputedStyle(host).font;
    shown = fitMiddleEllipsis(text, host.clientWidth, (value) => measure(value, font));
  }

  $effect(() => {
    text;
    layout();
  });

  onMount(() => {
    if (!host) return;
    const observer = new ResizeObserver(() => layout());
    observer.observe(host);
    layout();
    return () => observer.disconnect();
  });
</script>

<span class="middle-ellipsis" bind:this={host}>{shown || text}</span>

<style>
  .middle-ellipsis {
    display: block;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
</style>
