<script module lang="ts">
  // One picker for the four font settings (ADR 0042). The settings page and the quick-settings drawer each mount the
  // pickers, and all of them show the same list: the list, its loading state and the background refresh live here,
  // once. The list is read only when a picker opens (never at startup); the cached list shows at once and a
  // background refresh replaces it in place (`fonts-list-updated`), keeping each picker's scroll and search.
  import type { FontFamilyEntry } from './ipc';

  const catalog = $state({
    fonts: [] as FontFamilyEntry[],
    /** A background refresh is running. */
    refreshing: false,
    /** The first read has answered. */
    loaded: false,
    failure: null as string | null,
  });
  let loading: Promise<void> | null = null;
  let listening = false;
  /** Refreshes received so far: a `fonts_list` answer older than a refresh must not undo it. */
  let updates = 0;
</script>

<script lang="ts">
  import { isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { tr } from './i18n.js';
  import { getErrorText, invokeCommand } from './ipc';
  import { ensureAndroidFontFaces, isAndroidWebView } from './font-faces.js';
  import {
    DEFAULT_INTERFACE_FONT_STACK,
    FONT_PICKER_ROW_PX,
    applyFontListUpdate,
    fontDisplayName,
    fontFamilyCss,
    fontPickerRows,
    visibleRowRange,
  } from './font-picker.js';

  let {
    value,
    onChange,
    idPrefix,
    label = tr('字型'),
  }: {
    /** The stored family; null: 「預設字型」. */
    value: string | null | undefined;
    onChange: (family: string | null) => void;
    idPrefix: string;
    label?: string;
  } = $props();

  const android = isAndroidWebView();
  let open = $state(false);
  let query = $state('');
  let scrollTop = $state(0);
  let viewportHeight = $state(FONT_PICKER_ROW_PX * 8);

  const rows = $derived(fontPickerRows(catalog.fonts, query));
  const range = $derived(visibleRowRange(scrollTop, viewportHeight, FONT_PICKER_ROW_PX, rows.length));
  const visibleRows = $derived(rows.slice(range.start, range.end).map((row, offset) => ({ row, index: range.start + offset })));
  const currentName = $derived(fontDisplayName(catalog.fonts, value) ?? tr('預設字型'));
  const currentKey = $derived((value ?? '').toLowerCase());
  const showLoading = $derived(open && catalog.fonts.length === 0 && (!catalog.loaded || catalog.refreshing));

  function ensureLoaded() {
    if (loading || !isTauri()) return;
    if (!listening) {
      listening = true;
      void listen<{ fonts: FontFamilyEntry[]; changed: boolean }>('fonts-list-updated', (event) => {
        updates += 1;
        catalog.fonts = applyFontListUpdate(catalog.fonts, event.payload);
        catalog.refreshing = false;
      });
    }
    catalog.failure = null;
    const before = updates;
    loading = invokeCommand('fonts_list', {})
      .then((list) => {
        // A refresh that already answered with a newer list wins; an unchanged one sends no list.
        if (updates === before || !catalog.fonts.length) catalog.fonts = list.fonts;
        if (updates === before) catalog.refreshing = list.refreshing;
      })
      .catch((error) => {
        catalog.failure = getErrorText(error);
        catalog.refreshing = false;
      })
      .finally(() => {
        catalog.loaded = true;
        // The next opening asks again: fonts may have been installed meanwhile.
        loading = null;
      });
  }

  function toggle() {
    open = !open;
    if (open) ensureLoaded();
  }

  function rowFont(font: FontFamilyEntry | null): string {
    if (!font) return DEFAULT_INTERFACE_FONT_STACK;
    return fontFamilyCss(font.family, DEFAULT_INTERFACE_FONT_STACK, { android });
  }

  // Android loads a font's files only for the rows in view.
  $effect(() => {
    if (!android || !open) return;
    for (const { row } of visibleRows) ensureAndroidFontFaces(row.font);
  });
</script>

<div class="font-picker cover-corner-setting">
  <button
    id="{idPrefix}-font-picker-button"
    class="font-picker-current"
    type="button"
    aria-expanded={open}
    aria-controls="{idPrefix}-font-picker-panel"
    onclick={toggle}
  >
    <strong>{label}</strong>
    <span class="font-picker-current-name" style={`font-family:${rowFont(catalog.fonts.find((font) => font.family.toLowerCase() === currentKey) ?? null)}`}>{currentName}</span>
    <span class="font-picker-chevron" aria-hidden="true">{open ? '▴' : '▾'}</span>
  </button>
  {#if open}
    <div id="{idPrefix}-font-picker-panel" class="font-picker-panel">
      <input
        class="font-picker-search"
        type="search"
        bind:value={query}
        placeholder={tr('搜尋字型…')}
        aria-label={tr('搜尋字型')}
        autocomplete="off"
        spellcheck="false"
        oninput={() => { scrollTop = 0; }}
      />
      {#if showLoading}
        <p class="font-picker-status" role="status">{tr('讀取字型中…')}</p>
      {:else if catalog.failure && catalog.fonts.length === 0}
        <p class="font-picker-status" role="alert">{catalog.failure}</p>
      {/if}
      <div
        class="font-picker-list"
        role="listbox"
        aria-label={label}
        tabindex="-1"
        bind:clientHeight={viewportHeight}
        onscroll={(event) => { scrollTop = event.currentTarget.scrollTop; }}
      >
        <div class="font-picker-spacer" style={`height:${rows.length * FONT_PICKER_ROW_PX}px`}>
          {#each visibleRows as { row, index } (row.key)}
            {@const chosen = (row.family ?? '').toLowerCase() === currentKey}
            <button
              class="font-picker-row"
              class:is-active={chosen}
              type="button"
              role="option"
              aria-selected={chosen}
              style={`top:${index * FONT_PICKER_ROW_PX}px;height:${FONT_PICKER_ROW_PX}px;font-family:${rowFont(row.font)}`}
              onclick={() => onChange(row.family)}
            >{row.font ? row.font.displayName : tr('預設字型')}</button>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .font-picker {
    min-width: 0;
  }

  .font-picker-current {
    display: flex;
    min-width: 0;
    min-height: 58px;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: 9px;
    color: var(--text);
    background: rgba(var(--text-rgb), 0.02);
    font: inherit;
    font-size: var(--font-caption);
    text-align: left;
    cursor: pointer;
  }

  .font-picker-current strong {
    flex: 0 0 auto;
    font-weight: 600;
  }

  .font-picker-current-name {
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
    color: var(--text-soft);
    font-size: var(--font-body);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .font-picker-chevron {
    flex: 0 0 auto;
    color: var(--muted);
  }

  .font-picker-panel {
    display: grid;
    gap: 8px;
    min-width: 0;
  }

  .font-picker-search {
    box-sizing: border-box;
    min-width: 0;
    min-height: 34px;
    padding: 0 11px;
    border: 1px solid rgba(var(--text-rgb), 0.18);
    border-radius: 10px;
    color: var(--text);
    background: transparent;
    font: inherit;
    font-size: var(--font-caption);
  }

  .font-picker-search::placeholder {
    color: var(--muted);
  }

  .font-picker-search:focus-visible {
    border-color: var(--accent);
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .font-picker-status {
    margin: 0;
    color: var(--muted);
    font-size: var(--font-caption);
  }

  .font-picker-list {
    position: relative;
    height: min(288px, 50vh);
    overflow-y: auto;
    overscroll-behavior: none;
    border: 1px solid var(--line);
    border-radius: 9px;
  }

  .font-picker-spacer {
    position: relative;
  }

  .font-picker-row {
    position: absolute;
    left: 0;
    right: 0;
    box-sizing: border-box;
    overflow: hidden;
    padding: 0 12px;
    border: 0;
    color: var(--text);
    background: transparent;
    font-size: var(--font-title);
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .font-picker-row:hover {
    background: rgba(var(--text-rgb), 0.06);
  }

  .font-picker-row.is-active {
    color: var(--accent);
    background: rgba(var(--text-rgb), 0.08);
  }

  .font-picker-row:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -1px;
  }
</style>
