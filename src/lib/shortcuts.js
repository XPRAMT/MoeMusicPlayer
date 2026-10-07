/** @typedef {'fullscreen' | 'playPause' | 'seekBack' | 'seekForward' | 'previous' | 'next'} ShortcutAction */
/** @typedef {'keyboard' | 'mouse'} ShortcutDevice */
/**
 * @typedef {Object} ShortcutBinding
 * @property {ShortcutDevice} device
 * @property {string} code
 */
/**
 * @typedef {Record<ShortcutAction, ShortcutBinding[]>} ShortcutSettings
 */

/** @type {{ id: ShortcutAction, label: string }[]} */
export const SHORTCUT_ACTIONS = [
  { id: 'fullscreen', label: '全螢幕切換' },
  { id: 'playPause', label: '播放／暫停' },
  { id: 'seekBack', label: '後退 5 秒' },
  { id: 'seekForward', label: '前進 5 秒' },
  { id: 'previous', label: '上一首' },
  { id: 'next', label: '下一首' },
];

/** @type {ShortcutAction[]} */
const ACTIONS = SHORTCUT_ACTIONS.map((action) => action.id);
const MOUSE_CODES = new Set(['wheelUp', 'wheelDown', 'back', 'forward']);
const KEY_LABELS = {
  F11: 'F11',
  Space: '空白鍵',
  ArrowLeft: '左方向鍵',
  ArrowRight: '右方向鍵',
  PageUp: 'Page Up',
  PageDown: 'Page Down',
  wheelUp: '滾輪向上',
  wheelDown: '滾輪向下',
  back: '滑鼠上頁',
  forward: '滑鼠下頁',
};

/** @returns {ShortcutSettings} */
export function defaultShortcutSettings() {
  return {
    fullscreen: [{ device: 'keyboard', code: 'F11' }],
    playPause: [{ device: 'keyboard', code: 'Space' }],
    seekBack: [
      { device: 'keyboard', code: 'ArrowLeft' },
      { device: 'mouse', code: 'wheelUp' },
    ],
    seekForward: [
      { device: 'keyboard', code: 'ArrowRight' },
      { device: 'mouse', code: 'wheelDown' },
    ],
    previous: [
      { device: 'keyboard', code: 'PageUp' },
      { device: 'mouse', code: 'back' },
    ],
    next: [
      { device: 'keyboard', code: 'PageDown' },
      { device: 'mouse', code: 'forward' },
    ],
  };
}

/**
 * @param {string} code
 * @returns {boolean}
 */
function validKeyboardCode(code) {
  return /^[A-Za-z0-9]{1,32}$/.test(code);
}

/**
 * @param {unknown} binding
 * @returns {ShortcutBinding | null}
 */
function normalizeBinding(binding) {
  if (!binding || typeof binding !== 'object') return null;
  const device = /** @type {{ device?: unknown, code?: unknown }} */ (binding).device;
  const code = /** @type {{ device?: unknown, code?: unknown }} */ (binding).code;
  if (device === 'keyboard' && typeof code === 'string' && validKeyboardCode(code)) {
    return { device, code };
  }
  if (device === 'mouse' && typeof code === 'string' && MOUSE_CODES.has(code)) {
    return { device, code };
  }
  return null;
}

/**
 * @param {unknown} value
 * @returns {ShortcutSettings}
 */
export function normalizeShortcutSettings(value) {
  const defaults = defaultShortcutSettings();
  if (!value || typeof value !== 'object') return defaults;
  const source = /** @type {Record<string, unknown>} */ (value);
  /** @type {ShortcutSettings} */
  const next = defaultShortcutSettings();
  for (const action of ACTIONS) {
    const raw = source[action];
    if (!Array.isArray(raw)) continue;
    const bindings = [];
    const seen = new Set();
    for (const item of raw) {
      const binding = normalizeBinding(item);
      if (!binding) continue;
      const key = `${binding.device}:${binding.code}`;
      if (seen.has(key)) continue;
      seen.add(key);
      bindings.push(binding);
      if (bindings.length >= 4) break;
    }
    next[action] = bindings;
  }
  return next;
}

/**
 * @param {ShortcutBinding} binding
 * @returns {string}
 */
export function shortcutBindingLabel(binding) {
  if (Object.prototype.hasOwnProperty.call(KEY_LABELS, binding.code)) {
    return KEY_LABELS[/** @type {keyof typeof KEY_LABELS} */ (binding.code)];
  }
  return binding.code;
}

/**
 * @param {ShortcutSettings} settings
 * @param {ShortcutDevice} device
 * @param {string} code
 * @returns {ShortcutAction | null}
 */
export function shortcutActionFor(settings, device, code) {
  for (const action of ACTIONS) {
    if (settings[action].some((binding) => binding.device === device && binding.code === code)) {
      return action;
    }
  }
  return null;
}

/**
 * One physical input belongs to one action. Adding it removes older copies.
 * @param {ShortcutSettings} settings
 * @param {ShortcutAction} action
 * @param {ShortcutBinding} binding
 * @returns {ShortcutSettings}
 */
export function addShortcutBinding(settings, action, binding) {
  const next = normalizeShortcutSettings(settings);
  for (const id of ACTIONS) {
    next[id] = next[id].filter((item) => item.device !== binding.device || item.code !== binding.code);
  }
  if (next[action].length < 4) next[action] = [...next[action], binding];
  return next;
}

/**
 * @param {ShortcutSettings} settings
 * @param {ShortcutAction} action
 * @param {ShortcutBinding} binding
 * @returns {ShortcutSettings}
 */
export function removeShortcutBinding(settings, action, binding) {
  const next = normalizeShortcutSettings(settings);
  next[action] = next[action].filter((item) => item.device !== binding.device || item.code !== binding.code);
  return next;
}

/**
 * Wheel and side buttons should not seek or change tracks while a list can scroll,
 * or while the user is typing.
 * @param {EventTarget | null} target
 * @returns {boolean}
 */
export function shortcutTargetIsEditable(target) {
  if (!(target instanceof Element)) return false;
  return Boolean(target.closest('input, textarea, select, [contenteditable="true"]'));
}

/**
 * @param {EventTarget | null} target
 * @returns {boolean}
 */
export function shortcutTargetIsScrollable(target) {
  if (!(target instanceof Element)) return false;
  return Boolean(target.closest(
    '.lyrics-view, .lyrics-candidates, .paged-viewport, .settings-panel, .settings-source-panel, .quick-settings-drawer-scroll, .source-list',
  ));
}
