import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
const css = await readFile(new URL('../src/app.css', import.meta.url), 'utf8');

test('the playback bar is not part of the quick settings close area', () => {
  // No fixed layer over the dock, and the dock is not inert while the drawer is open.
  assert.doesNotMatch(app, /quick-settings-dock-dismiss/);
  assert.doesNotMatch(css, /quick-settings-dock-dismiss/);
  const footer = app.match(/<footer class="player-dock"[^>]*>/)?.[0] ?? '';
  assert.ok(footer.length > 0);
  assert.doesNotMatch(footer, /inert/);
});

test('clicking outside the drawer still closes it through the scrim, which stays inside the overlay', () => {
  assert.match(app, /class="now-playing-quick-settings-scrim"[\s\S]{0,200}closeQuickSettings/);
  const scrim = css.match(/\.now-playing-quick-settings-scrim \{[^}]*\}/)?.[0] ?? '';
  assert.match(scrim, /position: absolute/);
  assert.doesNotMatch(scrim, /position: fixed/);
});
