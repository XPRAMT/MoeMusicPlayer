import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('the progress bar and the marquees run linear (sub-pixel)', () => {
  const progress = read('src/lib/PlaybackProgress.svelte');
  assert.match(progress, /ms linear forwards/);
  assert.doesNotMatch(progress, /steps\(/);
  const marquee = read('src/lib/dock-marquee.ts');
  assert.doesNotMatch(marquee, /animationTimingFunction|steps\(/);
  const css = read('src/app.css');
  assert.match(css, /animation: dock-marquee-scroll 9s linear infinite/);
  // Both pauses hold a single transform, so they present nothing.
  assert.match(css, /0%,\s*12% \{\s*transform: translateX\(0\);\s*\}\s*88%,\s*100% \{/);
});
