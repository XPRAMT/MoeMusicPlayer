// Edge content keeps clear of the Android system bars on all four sides (landscape side navigation bar
// included), using the single --android-safe-* mechanism set from MainActivity's WindowInsets.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { getAppTooltipPosition } from '../src/lib/app-tooltip.js';
import { parseSafeInsetPx, readSafeInsets, SAFE_INSET_PROPERTIES } from '../src/lib/safe-viewport.js';
import { getPopoverHorizontalShiftPx, getVolumePopoverLeftShiftPx } from '../src/lib/volume-popover-position.js';

const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
const favorite = readFileSync(new URL('../src/lib/FavoritePopover.svelte', import.meta.url), 'utf8');
const tooltip = readFileSync(new URL('../src/lib/app-tooltip.js', import.meta.url), 'utf8');
const policy = readFileSync(
  new URL('../src-tauri/gen/android/app/src/main/java/com/moemusicplayer/app/AndroidInsetsPolicy.kt', import.meta.url),
  'utf8',
);
const activity = readFileSync(
  new URL('../src-tauri/gen/android/app/src/main/java/com/moemusicplayer/app/MainActivity.kt', import.meta.url),
  'utf8',
);

/** @param {string} selector */
function rule(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = css.match(new RegExp(`(?:^|\\n)${escaped} \\{([^}]*)\\}`));
  assert.ok(match, `missing rule ${selector}`);
  return match[1];
}

test('native insets cover left, top and right from systemBars | displayCutout, and reach the page on every change', () => {
  for (const side of ['left', 'top', 'right']) {
    assert.match(policy, new RegExp(`${side} = maxOf\\(systemBars\\.${side}, displayCutout\\.${side}, tappableElement\\.${side}\\)`));
  }
  assert.match(policy, /bottom = tappableElement\.bottom/);
  assert.match(activity, /setOnApplyWindowInsetsListener/);
  assert.match(activity, /\.put\("safeLeftPx", insets\.controls\.left\)/);
  assert.match(activity, /\.put\("safeRightPx", insets\.controls\.right\)/);
  assert.match(activity, /moe:android-insets/);
  for (const property of Object.values(SAFE_INSET_PROPERTIES)) {
    assert.ok(app.includes(`['${property}', insets.`), `App.svelte must set ${property}`);
  }
});

test('the app shell pads all four sides, so the top bars, sidebar, drawer, dock and lists sit inside the safe area', () => {
  assert.match(rule('.app-shell'), /padding: var\(--android-safe-top\) var\(--android-safe-right\) var\(--android-safe-bottom\) var\(--android-safe-left\);/);
  // Children of the shell grid (no fixed positioning): the main top bar lives in .workspace, the Now Playing bar
  // in the overlay, the drawer is absolute inside the overlay and every .locate-fab is absolute inside its list.
  assert.match(rule('.workspace'), /grid-row: 1;/);
  assert.doesNotMatch(rule('.workspace'), /position: fixed/);
  assert.match(rule('.now-playing-overlay'), /position: relative;[^]*grid-row: 1;/);
  assert.match(rule('.now-playing-quick-settings-drawer'), /position: absolute;/);
  assert.match(rule('.now-playing-quick-settings-drawer'), /inset: calc\(-1 \* var\(--android-safe-top, 0px\)\) 0 0 auto;/);
  assert.match(rule('.locate-fab'), /position: absolute;/);
  assert.match(rule('.sidebar'), /grid-column: 1;/);
});

test('window-fixed edge surfaces apply the safe insets themselves', () => {
  const panel = css.match(/\.app-shell \.sidebar\.floating-surface \{([^}]*)\}/)?.[1] ?? '';
  assert.match(panel, /padding-top: calc\(var\(--sidebar-brand-inset\) \+ var\(--android-safe-top, 0px\)\);/);
  assert.match(panel, /bottom: calc\(var\(--dock-height\) \+ var\(--android-safe-bottom, 0px\)/);
  assert.match(panel, /padding-left: calc\(var\(--sidebar-pad-x\) \+ var\(--android-safe-left, 0px\)\);/);
  assert.match(panel, /calc\(100vw - var\(--android-safe-right, 0px\) - 16px\)/);
  const notice = rule('.playback-notice');
  assert.match(notice, /left: calc\(50% \+ \(var\(--android-safe-left, 0px\) - var\(--android-safe-right, 0px\)\) \/ 2\);/);
  assert.match(notice, /max-width: calc\(100% - 32px - var\(--android-safe-left, 0px\) - var\(--android-safe-right, 0px\)\);/);
});

test('the Now Playing tools never shrink below their buttons, so none overflow past the safe edge', () => {
  assert.match(rule('.now-playing-topbar-tools'), /flex: 0 0 auto;/);
  assert.match(rule('.now-playing-header-track'), /min-width: 0;[^]*flex: 1 1 auto;/);
});

test('popovers and tooltips clamp to the safe area, not the raw window', () => {
  assert.match(app, /sleepTimerPopover\.getBoundingClientRect\(\)\.width, window\.innerWidth, undefined, readSafeInsets\(\)\)/);
  assert.match(app, /volumePopoverLeftShiftPx = getVolumePopoverLeftShiftPx\(\s*anchorRect\.left \+ anchorRect\.width \/ 2,\s*popover\.getBoundingClientRect\(\)\.width,\s*window\.innerWidth,\s*undefined,\s*readSafeInsets\(\),\s*\)/);
  assert.match(favorite, /window\.innerWidth,\s*undefined,\s*readSafeInsets\(\),\s*\)/);
  assert.match(tooltip, /window\.innerHeight, readSafeInsets\(\)\)/);
});

test('popover shift keeps the edge gap from a side navigation bar', () => {
  const right = { left: 0, right: 48 };
  assert.equal(getPopoverHorizontalShiftPx(500, 192, 1000, 8, right), 0);
  assert.equal(getPopoverHorizontalShiftPx(920, 192, 1000, 8, right), -72);
  assert.equal(getPopoverHorizontalShiftPx(80, 192, 1000, 8, { left: 30 }), 54);
  assert.equal(getVolumePopoverLeftShiftPx(940, 120, 1000, 8, right), 56);
  assert.equal(getVolumePopoverLeftShiftPx(940, 120, 1000), 8);
});

test('tooltips stay inside the safe area', () => {
  const safe = { left: 0, top: 24, right: 48, bottom: 0 };
  assert.deepEqual(getAppTooltipPosition({ left: 960, right: 990, top: 300, bottom: 330 }, 120, 32, 1000, 600, safe), { left: 824, top: 260 });
  assert.deepEqual(getAppTooltipPosition({ left: 100, right: 140, top: 30, bottom: 50 }, 120, 32, 1000, 600, safe), { left: 60, top: 58 });
});

test('safe inset values parse to CSS px, with 0 for unset or unresolved values', () => {
  assert.equal(parseSafeInsetPx('48px'), 48);
  assert.equal(parseSafeInsetPx(' 12.5px '), 12.5);
  assert.equal(parseSafeInsetPx('env(safe-area-inset-left, 0px)'), 0);
  assert.equal(parseSafeInsetPx(''), 0);
  assert.equal(parseSafeInsetPx('-3px'), 0);
  assert.deepEqual(readSafeInsets(null), { left: 0, top: 0, right: 0, bottom: 0 });
});
