import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SLEEP_TIMER_MAX_MINUTES,
  accumulateSleepTimerSteps,
  adjustSleepTimerMinutes,
  formatSleepTimer,
  getSleepTimerDisplayMinutes,
  getSleepTimerRemainingMs,
} from '../src/lib/sleep-timer.js';

test('remaining time comes from the wall-clock end timestamp', () => {
  assert.equal(getSleepTimerRemainingMs(10_000, 4_000), 6_000);
  assert.equal(getSleepTimerRemainingMs(10_000, 12_000), 0);
  assert.equal(getSleepTimerRemainingMs(null, 0), 0);
});

test('display rounds up to whole minutes and formats as hh:mm', () => {
  assert.equal(getSleepTimerDisplayMinutes(1), 1);
  assert.equal(getSleepTimerDisplayMinutes(60_000), 1);
  assert.equal(getSleepTimerDisplayMinutes(60_001), 2);
  assert.equal(getSleepTimerDisplayMinutes(0), 0);
  assert.equal(formatSleepTimer(0), '00:00');
  assert.equal(formatSleepTimer(95), '01:35');
  assert.equal(formatSleepTimer(SLEEP_TIMER_MAX_MINUTES), '23:55');
});

test('adjustment snaps to the 5-minute grid and clamps to 00:00..23:55', () => {
  assert.equal(adjustSleepTimerMinutes(0, 1), 5);
  assert.equal(adjustSleepTimerMinutes(13, 1), 15);
  assert.equal(adjustSleepTimerMinutes(13, -1), 10);
  assert.equal(adjustSleepTimerMinutes(15, 2), 25);
  assert.equal(adjustSleepTimerMinutes(5, -1), 0);
  assert.equal(adjustSleepTimerMinutes(3, -1), 0);
  assert.equal(adjustSleepTimerMinutes(0, -3), 0);
  assert.equal(adjustSleepTimerMinutes(23 * 60 + 50, 5), SLEEP_TIMER_MAX_MINUTES);
  assert.equal(adjustSleepTimerMinutes(42, 0), 42);
});

test('wheel and drag deltas accumulate into whole steps; up lengthens', () => {
  assert.deepEqual(accumulateSleepTimerSteps(0, -100, 50), { steps: 2, carry: 0 });
  assert.deepEqual(accumulateSleepTimerSteps(0, 30, 50), { steps: 0, carry: 30 });
  assert.deepEqual(accumulateSleepTimerSteps(30, 30, 50), { steps: -1, carry: 10 });
  assert.deepEqual(accumulateSleepTimerSteps(0, -13, 14), { steps: 0, carry: -13 });
  assert.deepEqual(accumulateSleepTimerSteps(-13, -2, 14), { steps: 1, carry: -1 });
});

test('sleep timer button sits immediately left of the lyrics controls and the deadline lives in Rust', async () => {
  const { readFile } = await import('node:fs/promises');
  const app = await readFile(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const rust = await readFile(new URL('../src-tauri/src/sleep_timer.rs', import.meta.url), 'utf8');
  const tools = app.indexOf('<div class="now-playing-topbar-tools">');
  const timer = app.indexOf('class="sleep-timer-anchor"', tools);
  const lyrics = app.indexOf('{#if lyricsTopbarStatus}', tools);
  assert.ok(tools >= 0 && timer > tools && lyrics > timer, 'timer anchor precedes the 歌詞 group');
  assert.match(app, /class="soft-button icon-button sleep-timer-toggle"[\s\S]*?aria-label="睡眠定時"[\s\S]*?<IconAlarm/);
  assert.match(app, /class="sleep-timer-popover floating-surface"/);
  assert.match(app, /invokeCommand\('sleep_timer_set', \{ durationMs \}\)/);
  assert.match(rust, /crate::playback_pause\(/, 'expiry pauses (not stops) playback');
  assert.match(rust, /SystemTime::now\(\)/, 'deadline uses the wall clock');
});
