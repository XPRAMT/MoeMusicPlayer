import assert from 'node:assert/strict';
import test from 'node:test';
import {
  addShortcutBinding,
  defaultShortcutSettings,
  normalizeShortcutSettings,
  removeShortcutBinding,
  shortcutActionFor,
  shortcutBindingLabel,
} from '../src/lib/shortcuts.js';

test('shortcut defaults bind keyboard and mouse the way the player starts', () => {
  const shortcuts = defaultShortcutSettings();
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'F11'), 'fullscreen');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'Space'), 'playPause');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'ArrowLeft'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'ArrowRight'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'wheelUp'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'wheelDown'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageUp'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageDown'), 'next');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'back'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'forward'), 'next');
  assert.equal(shortcutBindingLabel(shortcuts.playPause[0]), '空白鍵');
  assert.equal(shortcutBindingLabel(shortcuts.previous[1]), '滑鼠上頁');
});

test('gamepad buttons can be bound and are absent from the defaults', () => {
  const defaults = defaultShortcutSettings();
  const boundDevices = Object.values(defaults).flat().map((binding) => binding.device);
  assert.equal(boundDevices.includes('gamepad'), false);
  const bound = addShortcutBinding(defaults, 'playPause', { device: 'gamepad', code: 'button0' });
  assert.equal(shortcutActionFor(bound, 'gamepad', 'button0'), 'playPause');
  assert.equal(shortcutActionFor(bound, 'keyboard', 'Space'), 'playPause');
  assert.equal(shortcutBindingLabel(bound.playPause.find((binding) => binding.device === 'gamepad')), '手柄 A');
  const stick = addShortcutBinding(defaults, 'seekForward', { device: 'gamepad', code: 'stickLeftXPlus' });
  assert.equal(shortcutActionFor(stick, 'gamepad', 'stickLeftXPlus'), 'seekForward');
  assert.equal(shortcutBindingLabel({ device: 'gamepad', code: 'stickLeftYMinus' }), '左搖桿上');
  assert.equal(Object.values(defaults).flat().some((binding) => binding.code.startsWith('stick')), false);
  const rejected = normalizeShortcutSettings({ playPause: [{ device: 'gamepad', code: 'button99' }] });
  assert.equal(shortcutActionFor(rejected, 'gamepad', 'button99'), null);
});

test('rebinding a shortcut moves it off the previous action', () => {
  const moved = addShortcutBinding(defaultShortcutSettings(), 'playPause', {
    device: 'keyboard',
    code: 'F11',
  });
  assert.equal(shortcutActionFor(moved, 'keyboard', 'F11'), 'playPause');
  assert.equal(shortcutActionFor(moved, 'keyboard', 'Space'), 'playPause');
  const removed = removeShortcutBinding(moved, 'playPause', { device: 'keyboard', code: 'Space' });
  assert.equal(shortcutActionFor(removed, 'keyboard', 'Space'), null);
});

test('saved wheel-up seek-back bindings flip to the corrected default', () => {
  const corrected = normalizeShortcutSettings({
    seekBack: [
      { device: 'keyboard', code: 'ArrowLeft' },
      { device: 'mouse', code: 'wheelUp' },
    ],
    seekForward: [
      { device: 'keyboard', code: 'ArrowRight' },
      { device: 'mouse', code: 'wheelDown' },
    ],
  });
  assert.equal(shortcutActionFor(corrected, 'mouse', 'wheelUp'), 'seekForward');
  assert.equal(shortcutActionFor(corrected, 'mouse', 'wheelDown'), 'seekBack');
});

test('invalid shortcut settings fall back per action without dropping the rest', () => {
  const normalized = normalizeShortcutSettings({
    playPause: [{ device: 'keyboard', code: 'KeyK' }, { device: 'mouse', code: 'left' }],
    seekBack: 'nope',
  });
  assert.deepEqual(normalized.playPause, [{ device: 'keyboard', code: 'KeyK' }]);
  assert.deepEqual(normalized.seekBack, defaultShortcutSettings().seekBack);
  assert.deepEqual(normalized.next, defaultShortcutSettings().next);
});
