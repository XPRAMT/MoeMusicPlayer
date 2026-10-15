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
  assert.match(css, /\.app-shell \.sidebar\.floating-surface \{[^}]*position: fixed;[^}]*top: calc\(var\(--window-titlebar-height\) \+ var\(--android-safe-top, 0px\)\);[^}]*bottom: calc\(var\(--dock-height\) \+ var\(--android-safe-bottom, 0px\)\);[^}]*left: 0;[^}]*border-radius: 0 var\(--floating-surface-radius\) var\(--floating-surface-radius\) 0;/s);

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
