import './helpers/untr-fs.mjs';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8').replace(/\r\n/g, '\n');
const styles = readFileSync(path.join(root, 'src/app.css'), 'utf8').replace(/\r\n/g, '\n');

test('unavailable previous and next controls are half transparent', () => {
  assert.match(appSource, /class="control-button navigation-control"[^>]*aria-label="上一首"[^>]*disabled=/);
  assert.match(appSource, /class="control-button navigation-control"[^>]*aria-label="下一首"[^>]*disabled=/);
  assert.match(styles, /\.player-dock \.navigation-control:disabled\s*\{\s*opacity:\s*0\.5\s*;?\s*\}/);
});
