import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const view = readFileSync(new URL('../src/lib/TaskbarLyricsView.svelte', import.meta.url), 'utf8');
const css = view.slice(view.indexOf('<style>'));
const rules = [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)].map((m) => ({ selector: m[1].trim(), body: m[2] }));
const shadowRules = rules.filter((r) => /data-text-effect='shadow'/.test(r.selector));

test('taskbar / floating lyrics shadow is a text-shadow on the text, never a filter (a parent filter is re-blurred every frame of the glide)', () => {
  assert.doesNotMatch(css, /drop-shadow/);
  assert.doesNotMatch(css, /(^|[;{\s])(backdrop-)?filter\s*:/);
  const textRule = shadowRules.find((r) => /taskbar-lyrics-strip-text/.test(r.selector) && /taskbar-lyrics-track/.test(r.selector));
  assert.ok(textRule, 'a shadow rule targets the track and the strip text');
  assert.equal((textRule.body.match(/text-shadow:/g) ?? []).length, 1);
  assert.doesNotMatch(textRule.body.replace(/rgba\([^;]*\)(?=;)/, 'COLOR'), /,/, 'a single shadow layer');
  assert.match(textRule.body, /text-shadow: 0 0 var\(--taskbar-lyrics-shadow-radius\) rgba\(var\(--taskbar-lyrics-effect-rgb\), 0\.75\)/);
  assert.doesNotMatch(css, /taskbar-lyrics-glow-blur/);
});

test('shadow radius keeps the old drop-shadow look (sigma 0.3em = text-shadow blur radius 0.6em)', () => {
  assert.match(css, /--taskbar-lyrics-shadow-blur: 0\.3em;/);
  assert.match(css, /--taskbar-lyrics-shadow-radius: calc\(var\(--taskbar-lyrics-shadow-blur\) \* 2\);/);
});

test('shadow is not cut by the clips: overflow clip margin, not padding (measured widths stay)', () => {
  const clip = shadowRules.find((r) => /taskbar-lyrics-main/.test(r.selector) && /taskbar-lyrics-translation/.test(r.selector));
  assert.ok(clip, 'a shadow rule targets the clipped lines');
  assert.match(clip.body, /overflow: clip;/);
  assert.match(clip.body, /overflow-clip-margin: var\(--taskbar-lyrics-shadow-radius\);/);
  assert.doesNotMatch(clip.body, /padding|filter/);
});

test('animated cells (opacity / transform) are not under a filter, and no new compositing layer is added', () => {
  const animated = rules.filter((r) => /strip-cell|taskbar-lyrics-strip\b|taskbar-lyrics-track/.test(r.selector));
  for (const r of animated) assert.doesNotMatch(r.body, /filter/);
  assert.equal((view.match(/will-change:/g) ?? []).length, 1, 'only the existing strip will-change');
  assert.match(css, /\.taskbar-lyrics-strip \{[^}]*will-change: transform;/);
});

test('the opacity fade is on the cell, whose text carries its own shadow, so the shadow fades with the text', () => {
  assert.match(view, /class="taskbar-lyrics-strip-cell"[^>]*style=\{cellFadeStyle\(cell\.index\)\}><span class="taskbar-lyrics-strip-text">/);
  assert.match(css, /@keyframes -global-taskbar-lyrics-fade-a \{\s*from \{ opacity: var\(--fade-from\); \}/);
});

test('outline mode stays a single stroke layer', () => {
  assert.equal((view.match(/-webkit-text-stroke:/g) ?? []).length, 1);
  assert.match(css, /data-text-effect='stroke'\] \.taskbar-lyrics-main,\s*\.taskbar-lyrics\[data-text-effect='stroke'\] \.taskbar-lyrics-translation \{\s*padding-block: 0\.06em;\s*margin-block: -0\.06em;/);
});