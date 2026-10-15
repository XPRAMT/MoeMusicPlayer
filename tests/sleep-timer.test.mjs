import test from 'node:test';
import assert from 'node:assert/strict';
import {
  SLEEP_TIMER_MAX_MINUTES,
  accumulateSleepTimerSteps,
  adjustSleepTimerField,
  splitSleepTimer,
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
  assert.equal(formatSleepTimer(SLEEP_TIMER_MAX_MINUTES), '23:59');
});

test('hours and minutes adjust independently by 1 and clamp without carrying', () => {
  assert.equal(adjustSleepTimerField(0, 'hours', 1), 60);
  assert.equal(adjustSleepTimerField(0, 'minutes', 1), 1);
  assert.equal(adjustSleepTimerField(13, 'minutes', -1), 12);
  assert.equal(adjustSleepTimerField(75, 'hours', -1), 15);
  assert.equal(adjustSleepTimerField(59, 'minutes', 1), 59);
  assert.equal(adjustSleepTimerField(60, 'minutes', -1), 60);
  assert.equal(adjustSleepTimerField(23 * 60 + 10, 'hours', 3), 23 * 60 + 10);
  assert.equal(adjustSleepTimerField(30, 'hours', -1), 30);
  assert.equal(adjustSleepTimerField(1, 'minutes', -5), 0);
  assert.equal(adjustSleepTimerField(42, 'minutes', 0), 42);
  assert.equal(adjustSleepTimerField(SLEEP_TIMER_MAX_MINUTES, 'minutes', 1), SLEEP_TIMER_MAX_MINUTES);
  assert.deepEqual(splitSleepTimer(125), { hours: '02', minutes: '05' });
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

test('sleep timer popover reuses the shared centered/clamped popover positioning', async () => {
  const { readFileSync } = await import('node:fs');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(app, /sleepTimerPopoverShiftX = getPopoverHorizontalShiftPx\(/);
  const rule = css.match(/\.sleep-timer-popover \{[^}]*\}/)[0];
  assert.match(rule, /left: 50%;/);
  assert.match(rule, /translateX\(calc\(-50% \+ var\(--sleep-timer-popover-shift-x, 0px\)\)\)/);
  assert.doesNotMatch(rule, /right: 0/);
});
