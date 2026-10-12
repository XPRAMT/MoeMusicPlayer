import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';
import { getPlaybackControlTooltips } from '../src/lib/playback-control-tooltips.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const appSource = readFileSync(path.join(root, 'src/App.svelte'), 'utf8').replace(/\r\n/g, '\n');
const controlsStart = appSource.indexOf('<div class="dock-controls">');
const controlsEnd = appSource.indexOf('</div>\n      <PlaybackProgress', controlsStart);
assert.ok(controlsStart >= 0 && controlsEnd > controlsStart, 'footer playback controls should exist');
const controls = appSource.slice(controlsStart, controlsEnd);

test('playback tooltips describe the control action instead of service diagnostics', () => {
  assert.match(controls, /title=\{playbackControlTooltips\.playPause\}/);
  assert.match(controls, /title=\{playbackControlTooltips\.shuffle\}/);
  assert.match(controls, /title=\{playbackControlTooltips\.repeat\}/);
  assert.doesNotMatch(controls, /title=\{showCapabilityDetail\(capabilities\?\.playback(?:Modes)?\)\}/);
});

test('previous and next tooltips follow their available action', () => {
  assert.match(controls, /title=\{playbackControlTooltips\.previous\}/);
  assert.match(controls, /title=\{playbackControlTooltips\.next\}/);
  assert.doesNotMatch(controls, /title=\{playback\?\.canPrevious \? '播放佇列上一首'/);
});

test('play, shuffle, and repeat tooltips follow current state and next action', () => {
  const paused = getPlaybackControlTooltips({
    currentTrack: { id: 'track' },
    isPlaying: false,
    shuffle: false,
    repeatMode: 'off',
  });
  assert.equal(paused.playPause, '播放');
  assert.equal(paused.shuffle, '開啟隨機播放');
  assert.equal(paused.repeat, '循環：關閉；點擊切換全部循環');

  const playing = getPlaybackControlTooltips({
    currentTrack: { id: 'track' },
    isPlaying: true,
    shuffle: true,
    repeatMode: 'all',
  });
  assert.equal(playing.playPause, '暫停');
  assert.equal(playing.shuffle, '關閉隨機播放');
  assert.equal(playing.repeat, '循環：全部；點擊切換單曲循環');

  assert.equal(
    getPlaybackControlTooltips({ repeatMode: 'one' }).repeat,
    '循環：單曲；點擊關閉循環',
  );
  assert.equal(getPlaybackControlTooltips(null).playPause, '尚未選擇歌曲');
});

test('previous tooltip matches the three-second restart rule', () => {
  const base = { canPrevious: true };
  assert.equal(getPlaybackControlTooltips({ ...base, positionMs: 3000 }).previous, '播放佇列上一首');
  assert.equal(getPlaybackControlTooltips({ ...base, positionMs: 3001 }).previous, '從頭播放目前歌曲');
  assert.equal(getPlaybackControlTooltips({ canPrevious: false, positionMs: 8000 }).previous, '沒有可播放的上一首');
});
