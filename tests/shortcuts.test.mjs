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
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'wheelUp'), 'seekBack');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'wheelDown'), 'seekForward');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageUp'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'keyboard', 'PageDown'), 'next');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'back'), 'previous');
  assert.equal(shortcutActionFor(shortcuts, 'mouse', 'forward'), 'next');
  assert.equal(shortcutBindingLabel(shortcuts.playPause[0]), '空白鍵');
  assert.equal(shortcutBindingLabel(shortcuts.previous[1]), '滑鼠上頁');
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

test('invalid shortcut settings fall back per action without dropping the rest', () => {
  const normalized = normalizeShortcutSettings({
    playPause: [{ device: 'keyboard', code: 'KeyK' }, { device: 'mouse', code: 'left' }],
    seekBack: 'nope',
  });
  assert.deepEqual(normalized.playPause, [{ device: 'keyboard', code: 'KeyK' }]);
  assert.deepEqual(normalized.seekBack, defaultShortcutSettings().seekBack);
  assert.deepEqual(normalized.next, defaultShortcutSettings().next);
});
