import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const view = readFileSync(new URL('../src/lib/TaskbarLyricsView.svelte', import.meta.url), 'utf8');

test('taskbar / floating lyrics shadow is a single filter layer (each layer repaints every frame of the glide)', () => {
  const rule = view.match(/data-text-effect='shadow'\] \.taskbar-lyrics-translation \{([^}]*)\}/);
  assert.ok(rule, 'shadow rule exists');
  assert.equal((rule[1].match(/drop-shadow\(/g) ?? []).length, 1);
  assert.doesNotMatch(view, /taskbar-lyrics-glow-blur/);
});

test('outline mode stays a single stroke layer', () => {
  assert.equal((view.match(/-webkit-text-stroke:/g) ?? []).length, 1);
});
