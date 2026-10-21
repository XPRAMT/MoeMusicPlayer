<script lang="ts">
  import { IconCloud, IconDeviceDesktop } from '@tabler/icons-svelte-runes';
  import { tr } from './i18n.js';
  import { appTooltip } from './app-tooltip.js';
  import { getScopeIndexFromPosition, SOURCE_SCOPES } from './source-scope.js';

  type SourceScope = 'local' | 'all' | 'remote';

  let { value, onChange }: { value: SourceScope; onChange: (next: SourceScope) => void } = $props();

  let track = $state<HTMLDivElement>();
  /** The position the knob follows while it is being dragged; null when it rests on the chosen value. */
  let dragIndex = $state<number | null>(null);
  let dragStartX = 0;
  let moved = false;

  const labels: Record<SourceScope, string> = {
    local: tr('只顯示本地歌曲'),
    all: tr('混合顯示全部歌曲'),
    remote: tr('只顯示遠端歌曲'),
  };
  const index = $derived(dragIndex ?? SOURCE_SCOPES.indexOf(value));

  function commit(next: SourceScope): void {
    if (next !== value) onChange(next);
  }

  function pointerIndex(event: PointerEvent): number {
    const box = track?.getBoundingClientRect();
    return box ? getScopeIndexFromPosition(event.clientX - box.left, box.width) : 1;
  }

  function startDrag(event: PointerEvent): void {
    if (event.button !== 0 || !track) return;
    dragStartX = event.clientX;
    moved = false;
    dragIndex = SOURCE_SCOPES.indexOf(value);
    try { track.setPointerCapture(event.pointerId); } catch { /* pointer already gone */ }
    event.preventDefault();
  }

  function moveDrag(event: PointerEvent): void {
    if (dragIndex === null) return;
    if (Math.abs(event.clientX - dragStartX) > 3) moved = true;
    dragIndex = pointerIndex(event);
  }

  function endDrag(event: PointerEvent): void {
    if (dragIndex === null) return;
    const target = dragIndex;
    dragIndex = null;
    try { track?.releasePointerCapture(event.pointerId); } catch { /* gone */ }
    if (moved && event.type === 'pointerup') commit(SOURCE_SCOPES[target]);
  }
</script>

<div
  class="source-scope-switch"
  role="radiogroup"
  aria-label={tr('歌曲來源篩選')}
  bind:this={track}
  data-swipe-ignore
>
  <span class="source-scope-icon source-scope-icon-start" aria-hidden="true"><IconDeviceDesktop size={14} stroke={1.8} /></span>
  <span class="source-scope-icon source-scope-icon-end" aria-hidden="true"><IconCloud size={14} stroke={1.8} /></span>
  {#each SOURCE_SCOPES as scope, position (scope)}
    <button
      class="source-scope-stop"
      type="button"
      role="radio"
      aria-checked={value === scope}
      aria-label={labels[scope]}
      use:appTooltip={labels[scope]}
      style:grid-column={position + 1}
      onclick={() => commit(scope)}
    ></button>
  {/each}
  <span
    class="source-scope-knob"
    class:is-dragging={dragIndex !== null}
    style:--scope-index={index}
    role="presentation"
    onpointerdown={startDrag}
    onpointermove={moveDrag}
    onpointerup={endDrag}
    onpointercancel={endDrag}
  ></span>
</div>

<style>
  .source-scope-switch {
    --scope-width: 78px;
    --scope-height: 30px;
    --scope-knob: 22px;
    --scope-pad: 4px;
    position: relative;
    display: grid;
    width: var(--scope-width);
    height: var(--scope-height);
    flex: 0 0 auto;
    grid-template-columns: repeat(3, 1fr);
    border: 1px solid var(--line);
    border-radius: 999px;
    /* Transparent: only the outline shows; the knob takes the accent colour from the theme. */
    background: transparent;
    user-select: none;
    -webkit-app-region: no-drag;
  }

  .source-scope-stop {
    z-index: 1;
    grid-row: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    border-radius: 999px;
    background: transparent;
    cursor: pointer;
  }

  .source-scope-stop:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }

  .source-scope-icon {
    position: absolute;
    top: 50%;
    display: flex;
    color: var(--text-soft);
    opacity: 0.7;
    transform: translateY(-50%);
    pointer-events: none;
  }

  .source-scope-icon-start { left: 7px; }
  .source-scope-icon-end { right: 7px; }

  .source-scope-knob {
    position: absolute;
    z-index: 2;
    top: 50%;
    left: calc(var(--scope-pad) + var(--scope-index) * ((var(--scope-width) - 2px - 2 * var(--scope-pad) - var(--scope-knob)) / 2));
    width: var(--scope-knob);
    height: var(--scope-knob);
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
    cursor: grab;
    transform: translateY(-50%);
    transition: left 140ms ease;
    touch-action: none;
  }

  .source-scope-knob.is-dragging {
    cursor: grabbing;
    transition: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .source-scope-knob { transition: none; }
  }
</style>
