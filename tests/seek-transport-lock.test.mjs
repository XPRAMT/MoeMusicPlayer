import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const ipc = readFileSync(new URL('../src/lib/ipc.ts', import.meta.url), 'utf8');

function fn(name) {
  const start = app.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `${name} exists`);
  return app.slice(start, app.indexOf('\n  }\n', start) === -1 ? app.indexOf('\r\n  }\r\n', start) : app.indexOf('\n  }\n', start));
}

test('a seek holds the transport through isSeeking, not the disabled busy flag', () => {
  assert.match(app, /let isSeeking = \$state\(false\);/);
  assert.match(fn('commitPlaybackSeek'), /seekPlayback\(positionMs, playback\?\.durationMs\), \{ seek: true \}\)/);
  const send = fn('sendPlaybackCommand');
  assert.match(send, /if \(options\.seek\) isSeeking = true; else isSendingPlaybackCommand = true;/);
  assert.match(send, /if \(options\.seek\) isSeeking = false; else if \(isLatest\(\)\) isSendingPlaybackCommand = false;/);
});

test('the transport click handlers ignore clicks while seeking', () => {
  for (const name of ['togglePlayback', 'controlPlayback', 'commitPlaybackSeek']) {
    assert.match(fn(name), /isSendingPlaybackCommand \|\| isSeeking\)/, `${name} ignores clicks while seeking`);
  }
});

test('the transport buttons keep their look while seeking (no disabled or grey from isSeeking)', () => {
  const controls = app.slice(app.indexOf('onclick={toggleShuffle}') - 400, app.indexOf('onclick={setRepeatMode}'));
  assert.ok(controls.length > 200, 'found the transport buttons');
  assert.doesNotMatch(controls, /isSeeking/);
  assert.match(controls, /disabled=\{!playbackReady \|\| isSendingPlaybackCommand\}/);
});

test('previous/next grey out only when there is nothing to go to; a busy command is ignored in the click handler', () => {
  assert.match(app, /disabled=\{!playbackNavigationReady \|\| \(!playback\?\.canPrevious && \(playback\?\.positionMs \?\? 0\) <= 3000\)\} onclick=\{\(\) => void controlPlayback\('playback_previous'\)\}/);
  assert.match(app, /disabled=\{!playbackNavigationReady \|\| !playback\?\.canNext\} onclick=\{\(\) => void controlPlayback\('playback_next'\)\}/);
  assert.match(fn('controlPlayback'), /if \(!playbackReady \|\| isSendingPlaybackCommand \|\| isSeeking\) return;/);
});

test('"buffering" is a playback state', () => {
  assert.match(ipc, /\| 'buffering'/);
});