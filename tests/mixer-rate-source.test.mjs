import assert from 'node:assert/strict';
import test from 'node:test';

import { mixerRatePanel, mixerRateSummary } from '../src/lib/mixer-rate-source.js';

const status = (state, extra = {}) => ({
  state,
  root: 'unknown',
  shizuku: 'not_installed',
  shizukuInstalled: false,
  source: 'platform',
  ...extra,
});

const STATES = ['none', 'shizukuNotRunning', 'shizukuDenied', 'shizukuNeedsPermission', 'shizukuActive', 'rootActive', 'tripped', 'disabled'];

test('every state has a title, text and a summary line', () => {
  for (const state of STATES) {
    const panel = mixerRatePanel(status(state));
    assert.equal(panel.state, state);
    assert.ok(panel.title.length > 0);
    assert.ok(panel.paragraphs.length >= 1 && panel.paragraphs.every((text) => text.length > 0), state);
    assert.ok(panel.summary.length > 0);
  }
});

test('no root and no Shizuku: the explanation offers the root check first', () => {
  const panel = mixerRatePanel(status('none'));
  assert.deepEqual(panel.buttons.map((b) => b.action), ['checkRoot']);
  assert.equal(panel.buttons[0].primary, true);
  const installed = mixerRatePanel(status('none', { shizukuInstalled: true }));
  assert.deepEqual(installed.buttons.map((b) => b.action), ['checkRoot', 'openShizuku']);
});

test('Shizuku installed but not running offers to open it', () => {
  const panel = mixerRatePanel(status('shizukuNotRunning', { shizukuInstalled: true }));
  assert.equal(panel.buttons[0].action, 'openShizuku');
  assert.ok(panel.buttons[0].primary);
});

test('a permission the user can still give is requested only by a button', () => {
  const panel = mixerRatePanel(status('shizukuNeedsPermission'));
  assert.equal(panel.buttons[0].action, 'requestShizuku');
  assert.equal(panel.buttons[0].label, '前往授權');
});

test('denied for good points to the Shizuku app, not to a request', () => {
  const panel = mixerRatePanel(status('shizukuDenied'));
  assert.ok(!panel.buttons.some((b) => b.action === 'requestShizuku'));
  assert.equal(panel.buttons[0].action, 'openShizuku');
});

test('granted states only offer a re-check, and a tripped or disabled lookup offers nothing', () => {
  assert.deepEqual(mixerRatePanel(status('rootActive')).buttons.map((b) => b.action), ['recheck']);
  assert.deepEqual(mixerRatePanel(status('shizukuActive')).buttons.map((b) => b.action), ['recheck']);
  assert.deepEqual(mixerRatePanel(status('tripped')).buttons, []);
  assert.deepEqual(mixerRatePanel(status('disabled')).buttons, []);
});

test('the summary names the source in use', () => {
  assert.match(mixerRateSummary(status('rootActive')), /root/);
  assert.match(mixerRateSummary(status('shizukuActive')), /Shizuku/);
  assert.match(mixerRateSummary(status('none')), /串流值/);
  assert.equal(mixerRateSummary(null), null);
});

test('the settings entry shows only while root and an authorized Shizuku are both missing', async () => {
  const { mixerRateEntryVisible } = await import('../src/lib/mixer-rate-source.js');
  assert.equal(mixerRateEntryVisible(null), false);
  for (const state of ['none', 'shizukuNotRunning', 'shizukuDenied', 'shizukuNeedsPermission']) {
    assert.equal(mixerRateEntryVisible(status(state)), true, state);
  }
  for (const state of ['rootActive', 'shizukuActive', 'tripped', 'disabled']) {
    assert.equal(mixerRateEntryVisible(status(state)), false, state);
  }
});
