import assert from 'node:assert/strict';
import test from 'node:test';
import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const srcDir = fileURLToPath(new URL('../src/', import.meta.url));

async function appCss() {
  return readFile(path.join(srcDir, 'app.css'), 'utf8');
}

async function styleSources() {
  const files = [];
  async function walk(dir) {
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) await walk(full);
      else if (/\.(css|svelte)$/.test(entry.name)) files.push(full);
    }
  }
  await walk(srcDir);
  const sources = [];
  for (const file of files) {
    const text = await readFile(file, 'utf8');
    const css = file.endsWith('.svelte')
      ? [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n')
      : text;
    sources.push({ file: path.relative(srcDir, file), css: css.replace(/\/\*[\s\S]*?\*\//g, '') });
  }
  return sources;
}

/** Innermost `selector { declarations }` blocks (nested @media blocks yield their inner rules). */
function rules(css) {
  return [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({ selector: m[1].trim(), body: m[2] }));
}

test('the shared focus ring is 1px solid accent at 70% opacity', async () => {
  const css = await appCss();
  assert.match(css, /:root\s*\{[^}]*--focus-ring:\s*1px solid color-mix\(in srgb, var\(--accent\) 70%, transparent\);/);
});

test('every focus outline under src/ uses the shared ring', async () => {
  const offenders = [];
  let focusOutlines = 0;
  for (const { file, css } of await styleSources()) {
    for (const { selector, body } of rules(css)) {
      if (!/:focus(?!-within)/.test(selector)) continue;
      for (const [, property, value] of body.matchAll(/(outline(?:-width|-color|-style)?)\s*:\s*([^;]+)/g)) {
        const v = value.trim();
        if (property === 'outline' && v === 'var(--focus-ring)') {
          focusOutlines += 1;
          continue;
        }
        if (property === 'outline' && (v === 'none' || v === '0') && /:focus:not\(:focus-visible\)|:focus(?![-\w])/.test(selector)) continue;
        offenders.push(`${file}: ${selector} { ${property}: ${v} }`);
      }
    }
  }
  assert.deepEqual(offenders, []);
  assert.ok(focusOutlines >= 20, `expected the shared ring in many focus rules, found ${focusOutlines}`);
});

test('keyboard focus defaults to the ring with zero specificity; mouse/touch focus shows nothing', async () => {
  const css = await appCss();
  assert.match(css, /:where\(:focus-visible\)\s*\{\s*outline: var\(--focus-ring\);\s*\}/);
  // Buttons and inputs keep their 1px offset.
  assert.match(css, /button:focus-visible,\s*input:focus-visible\s*\{\s*outline: var\(--focus-ring\);\s*outline-offset: 1px;/);
  assert.match(css, /:focus:not\(:focus-visible\)\s*\{\s*outline: none;/);
});

test('list rows draw the same ring inside the row so the viewport never clips it', async () => {
  const css = await appCss();
  assert.match(css, /\.paged-virtual-row:focus-visible\s*\{\s*outline: var\(--focus-ring\);\s*outline-offset: -1px;\s*\}/);
});
