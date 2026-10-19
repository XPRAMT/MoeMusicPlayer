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

test('shortcut defaults bind the keyboard and the gamepad the way the player starts', () => {
  const shortcuts = defaultShortcutSettings();
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'F11'), 'fullscreen');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'Space'), 'playPause');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'ArrowLeft'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'ArrowRight'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageUp'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageDown'), 'next');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button6'), 'fullscreen');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button9'), 'playPause');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button14'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'stickLeftXMinus'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button15'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'stickLeftXPlus'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button2'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'stickRightXMinus'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'button1'), 'next');
  assert.equal(shortcutActionFor(shortcuts, 'gamepad', 'stickRightXPlus'), 'next');
  assert.equal(Object.values(shortcuts).flat().some((binding) => binding.device === 'mouse'), false, 'no mouse bindings by default');
  assert.equal(shortcutBindingLabel(shortcuts.playPause[0]), '空白鍵');
});

test('more gamepad buttons and sticks can be bound on top of the defaults', () => {
  const defaults = defaultShortcutSettings();
  const bound = addShortcutBinding(defaults, 'playPause', { device: 'gamepad', code: 'button0' });
  assert.equal(shortcutActionFor(bound, 'gamepad', 'button0'), 'playPause');
  assert.equal(shortcutActionFor(bound, 'keyboard', 'Space'), 'playPause');
  assert.equal(shortcutBindingLabel(bound.playPause.find((binding) => binding.code === 'button0')), '手柄 A');
  const stick = addShortcutBinding(defaults, 'seekForward', { device: 'gamepad', code: 'stickLeftYPlus' });
  assert.equal(shortcutActionFor(stick, 'gamepad', 'stickLeftYPlus'), 'seekForward');
  assert.equal(shortcutBindingLabel({ device: 'gamepad', code: 'stickLeftYMinus' }), '左搖桿上');
  assert.equal(shortcutActionFor(defaults, 'gamepad', 'stickLeftYMinus'), null, 'the vertical sticks are free');
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
