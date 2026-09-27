import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8');

function progressSliderSource() {
  const start = appSource.indexOf('<input\n          class="progress-slider"');
  assert.notEqual(start, -1, 'missing playback progress slider');
  const end = appSource.indexOf('/>', start);
  assert.notEqual(end, -1, 'progress slider element must be self-closing');
  return appSource.slice(start, end + 2);
}

test('playback progress drag stays local until the user commits it', () => {
  const slider = progressSliderSource();
  assert.match(slider, /oninput=\{updatePlaybackSeek\}/);
  assert.match(slider, /onchange=\{commitPlaybackSeek\}/);
  assert.doesNotMatch(slider, /disabled=[^\n]*isSendingPlaybackCommand/);
});
