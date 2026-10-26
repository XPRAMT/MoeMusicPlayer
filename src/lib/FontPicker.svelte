<script module lang="ts">
  // One picker for the four font settings (ADR 0042). The settings page and the quick-settings drawer each mount the
  // pickers, and all of them show the same list: the list, its loading state and the background refresh live here,
  // once. The list is read only when a picker opens (never at startup). The first open waits (「讀取字型中…」) while the
  // backend scans every font and makes every name subset; later opens show the cached list at once and a background
  // refresh replaces it in place (`fonts-list-updated`), keeping each picker's scroll and search.
  // Every row is drawn in its own name subset at once (font-row-faces.js); the registered faces are shared.
  import { convertFileSrc } from '@tauri-apps/api/core';
  import type { FontFamilyEntry } from './ipc';
  import { addFontRowFace, createFontRowFaces, fontRowFamilyCss } from './font-row-faces.js';

  const catalog = $state({
    fonts: [] as FontFamilyEntry[],
    /** A background refresh is running. */
    refreshing: false,
    /** The first read has answered. */
    loaded: false,
    failure: null as string | null,
    /** Android: the family the system draws default text in (`fonts_list`). */
    systemDefault: null as string | null,
    /** The chosen interface font: the playing-page lyrics' default follows it. */
    interfaceFamily: null as string | null,
  });
  let loading: Promise<void> | null = null;
  let listening = false;
  /** Refreshes received so far: a `fonts_list` answer older than a refresh must not undo it. */
  let updates = 0;
  /** Bumped whenever the list is replaced (the virtual list rebuilds its rows). */
  let listRevision = $state(0);

  /** The rows' subset faces, registered as rows first render (shared by every picker). */
  const rowFaces = createFontRowFaces({ toUrl: convertFileSrc, addFace: addFontRowFace });

  /** The playing-page lyrics' 「預設（…）」 follows the interface font (App keeps this in step with the setting). */
  export function setInterfaceFontForPickers(family: string | null | undefined): void {
    catalog.interfaceFamily = family ?? null;
  }
</script>

<script lang="ts">
  import { isTauri } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { tr } from './i18n.js';
  import { getErrorText, invokeCommand } from './ipc';
  import { ensureAndroidFontFaces, isAndroidWebView } from './font-faces.js';
  import PagedVirtualList from './PagedVirtualList.svelte';
  import {
    DEFAULT_INTERFACE_FONT_STACK,
    FONT_PICKER_ROW_PX,
    applyFontListUpdate,
    defaultFontFamily,
    fontDisplayName,
    fontFamilyCss,
    fontPickerGroups,
    fontPickerRows,
  } from './font-picker.js';

  type FontTarget = 'interface' | 'lyrics' | 'taskbar' | 'floating';
  type PickerRow = ReturnType<typeof fontPickerRows>[number];

  let {
    value,
    onChange,
    idPrefix,
    label = tr('字型'),
    target = 'interface',
  }: {
    /** The stored family; null: 「預設字型」. */
    value: string | null | undefined;
    onChange: (family: string | null) => void;
    idPrefix: string;
    label?: string;
    /** What the setting draws: decides which font 「預設（…）」 names. */
    target?: FontTarget;
  } = $props();

  const android = isAndroidWebView();
  const platform = android ? 'android' : typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent ?? '') ? 'windows' : 'other';
  const groupLabels: Record<FontFamilyEntry['group'], string> = {
    traditional: tr('繁體中文'),
    simplified: tr('簡體中文'),
    japanese: tr('日文'),
    latin: tr('英文'),
  };
  let open = $state(false);
  let query = $state('');

  const rows = $derived(fontPickerRows(catalog.fonts, query));
  const groups = $derived(fontPickerGroups(rows));
  const groupRows = $derived(groups.map(({ group, beforeIndex }) => ({ label: groupLabels[group], beforeIndex })));
  const quickIndexLabels = $derived(groupRows.map(({ label }) => label));
  const itemAt = $derived((index: number): PickerRow | null => rows[index] ?? null);
  const currentKey = $derived((value ?? '').toLowerCase());
  const defaultFont = $derived(defaultFontFamily({
    target,
    platform,
    fonts: catalog.fonts,
    systemDefault: catalog.systemDefault,
    interfaceFamily: catalog.interfaceFamily,
  }));
  const defaultLabel = $derived(defaultFont ? tr('預設（{0}）', [defaultFont.displayName]) : tr('預設字型'));
  const currentFont = $derived(catalog.fonts.find((font) => font.family.toLowerCase() === currentKey) ?? null);
  const currentName = $derived(fontDisplayName(catalog.fonts, value) ?? defaultLabel);
  const showLoading = $derived(open && catalog.fonts.length === 0 && (!catalog.loaded || catalog.refreshing));

  function replaceFonts(fonts: FontFamilyEntry[]) {
    if (fonts === catalog.fonts) return;
    catalog.fonts = fonts;
    listRevision += 1;
  }

  function ensureLoaded() {
    if (loading || !isTauri()) return;
    if (!listening) {
      listening = true;
      void listen<{ fonts: FontFamilyEntry[]; changed: boolean; systemDefault?: string | null }>('fonts-list-updated', (event) => {
        updates += 1;
        replaceFonts(applyFontListUpdate(catalog.fonts, event.payload));
        if (event.payload.changed) catalog.systemDefault = event.payload.systemDefault ?? null;
        catalog.refreshing = false;
      });
    }
    catalog.failure = null;
    const before = updates;
    loading = invokeCommand('fonts_list', {})
      .then((list) => {
        // A refresh that already answered with a newer list wins; an unchanged one sends no list.
        if (updates === before || !catalog.fonts.length) {
          replaceFonts(list.fonts);
          catalog.systemDefault = list.systemDefault ?? null;
        }
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

  /** The current setting's label keeps the real family (as the setting is applied). */
  function currentFontCss(font: FontFamilyEntry | null): string {
    if (!font) return DEFAULT_INTERFACE_FONT_STACK;
    return fontFamilyCss(font.family, DEFAULT_INTERFACE_FONT_STACK, { android });
  }

  /** A row: its name subset (registered now if this is its first row), else the installed family itself. */
  function rowFontCss(font: FontFamilyEntry | null): string {
    return fontRowFamilyCss(font, rowFaces.faceFor, DEFAULT_INTERFACE_FONT_STACK);
  }

  // Android draws the current setting's label through the chosen family's @font-face (as App applies it).
  $effect(() => {
    if (android && open && currentFont) ensureAndroidFontFaces(currentFont);
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
    <span class="font-picker-current-name" style={`font-family:${currentFontCss(currentFont)}`}>{currentName}</span>
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
      />
      {#if showLoading}
        <p class="font-picker-status" role="status">{tr('讀取字型中…')}</p>
      {:else if catalog.failure && catalog.fonts.length === 0}
        <p class="font-picker-status" role="alert">{catalog.failure}</p>
      {/if}
      <!-- The library's virtual list: same group rows (0.7× height, 1 px line below) and quick index panel. -->
      <PagedVirtualList
        totalCount={rows.length}
        revision={listRevision}
        generation={0}
        scrollResetKey={query}
        {itemAt}
        getKey={(row: PickerRow) => row.key}
        isSelected={(row: PickerRow) => (row.family ?? '').toLowerCase() === currentKey}
        rowHeight={FONT_PICKER_ROW_PX}
        headerHeight={0}
        maxViewportHeight={288}
        listId="{idPrefix}-font-picker"
        ariaLabel={label}
        columnCount={1}
        gridTemplate="minmax(0,1fr)"
        className="font-picker-list"
        rowClassName="font-picker-row"
        listName="fonts"
        onRange={() => {}}
        onPlay={(row: PickerRow) => onChange(row.family)}
        {groupRows}
        {quickIndexLabels}
      >
        {#snippet row({ item })}
          {#if item}
            <span
              class="font-picker-row-name"
              role="gridcell"
              style={`font-family:${item.font ? rowFontCss(item.font) : currentFontCss(null)}`}
            >{item.font ? item.font.displayName : defaultLabel}</span>
          {/if}
        {/snippet}
      </PagedVirtualList>
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

  /* Rows are PagedVirtualList's (rowClassName); the name is this component's. */
  :global(.paged-virtual-row.font-picker-row) {
    display: flex;
    align-items: center;
    color: var(--text);
  }

  :global(.paged-virtual-row.font-picker-row:hover) {
    background: rgba(var(--text-rgb), 0.06);
  }

  :global(.paged-virtual-row.font-picker-row.selected) {
    color: var(--accent);
    background: rgba(var(--text-rgb), 0.08);
  }

  :global(.paged-virtual-viewport:focus-visible .paged-virtual-row.font-picker-row.active) {
    box-shadow: inset 2px 0 0 var(--line);
  }

  .font-picker-row-name {
    min-width: 0;
    overflow: hidden;
    padding: 0 12px;
    font-size: var(--font-title);
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
