import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8');

test('floating panels share Quick Settings surface tokens', async () => {
  const [css, app, lyrics, update, favorites] = await Promise.all([
    read('../src/app.css'),
    read('../src/App.svelte'),
    read('../src/lib/LyricsView.svelte'),
    read('../src/lib/UpdateCenter.svelte'),
    read('../src/lib/FavoritePopover.svelte'),
  ]);

  assert.match(css, /\.floating-surface\s*\{[\s\S]*?background: color-mix\(in srgb, var\(--text-inverse\) var\(--quick-settings-opacity, 70%\), transparent\)/);
  assert.match(css, /backdrop-filter: blur\(var\(--quick-settings-blur, 12px\)\)/);
  assert.match(css, /box-shadow: var\(--floating-surface-shadow\)/);
  // Narrow-layout navigation panel reuses the shared floating surface.
  assert.match(app, /class:floating-surface=\{isNarrowNav\}/);
  // No sidebar rule may override the shared settings-driven surface background.
  assert.match(css, /\.sidebar:not\(\.floating-surface\) \{\s*background: transparent;/);
  for (const rule of css.matchAll(/(^|\n)([^{}\n]*\.sidebar[^{}\n]*)\{([^}]*)\}/g)) {
    if (/\.sidebar(?![-\w])(?![^,]*\s)/.test(rule[2]) && /background\s*:|backdrop-filter\s*:/.test(rule[3])) assert.match(rule[2], /:not\(\.floating-surface\)/, `sidebar rule overrides floating surface: ${rule[2].trim()}`);
  }
  // The sleep timer popover uses the same surface and never sets its own background.
  assert.match(app, /class="sleep-timer-popover floating-surface"/);
  assert.doesNotMatch(css.match(/\.sleep-timer-popover \{[^}]*\}/)?.[0] ?? '', /background|backdrop-filter/);
  assert.match(css, /\.app-shell \.sidebar\.floating-surface \{[^}]*position: fixed;[^}]*top: calc\(var\(--window-titlebar-height\) \+ var\(--android-safe-top, 0px\)\);[^}]*bottom: calc\(var\(--dock-height\) \+ var\(--android-safe-bottom, 0px\) \+ var\(--nav-panel-dock-overhang\)\);[^}]*left: 0;[^}]*border-radius: 0;/s);
  // Edge timeline overhang is derived from the shared progress tokens, not a literal.
  assert.match(css, /\.app-shell:has\(\.player-dock\[data-timeline-style="edge"\]\) \.sidebar\.floating-surface \{\s*--nav-panel-dock-overhang: var\(--dock-edge-progress-track\);/);
  assert.match(css, /margin-top: calc\(-0\.5 \* var\(--dock-progress-hit\) - 0\.5 \* var\(--dock-progress-track\)\);/);
  assert.match(css, /--dock-progress-hit: var\(--dock-edge-progress-hit\);/);

  assert.match(app, /class="now-playing-quick-settings-drawer floating-surface"/);
  assert.match(app, /class="volume-popover floating-surface"/);
  assert.match(css, /\.volume-popover\s*\{[^}]*bottom:\s*calc\(100% \+ 14px\)/s);
  assert.match(favorites, /class="favorite-popover floating-surface"/);
  assert.match(favorites, /bottom:\s*calc\(100% \+ 14px\)/);
  assert.match(favorites, /getPopoverHorizontalShiftPx/);
  assert.match(lyrics, /class="lyrics-timing-panel floating-surface"/);
  assert.match(update, /class="update-dialog floating-surface"/);
  assert.match(app, /surfaceOpacityPercent=\{themePreferences\.quickSettingsOpacityPercent\}/);
  assert.match(app, /surfaceBlurPx=\{themePreferences\.quickSettingsBlurPx\}/);
  assert.doesNotMatch(css, /\.app-shell\.has-now-playing-backdrop \.volume-popover\s*\{/);
});

test('shared panel skin keeps update modal interaction and backdrop behavior', async () => {
  const [css, update] = await Promise.all([read('../src/app.css'), read('../src/lib/UpdateCenter.svelte')]);
  assert.match(update, /dialog\.showModal\(\)/);
  assert.match(update, /dialog\.close\(\)/);
  assert.match(css, /\.update-dialog::backdrop \{ background: #0006; \}/);
});
