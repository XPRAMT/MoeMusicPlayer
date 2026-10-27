import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const component = readFileSync(new URL('../src/lib/OutputDevicesSettings.svelte', import.meta.url), 'utf8');
const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');

function rule(source, selector) {
  const start = source.indexOf(`${selector} {`);
  assert.notEqual(start, -1, `${selector} rule exists`);
  return source.slice(start, source.indexOf('}', start));
}

test('the output device list scrolls inside at most half the window', () => {
  // The row styles are shared by the Windows and the Android list, so they live in app.css (ADR 0044).
  const shared = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  const list = rule(shared, '.output-device-list');
  assert.match(list, /max-height:\s*50vh;/);
  assert.match(list, /max-height:\s*50dvh;/);
  assert.ok(list.indexOf('50vh') < list.indexOf('50dvh'), 'the dvh value comes last and wins where supported');
  assert.match(list, /overflow-y:\s*auto;/);
  // No overscroll-behavior: touch chains natively; the wheel is handed on by the scroll-chain action.
  assert.doesNotMatch(list, /overscroll-behavior\s*:/);
  assert.match(list, /align-content:\s*start;/);
  assert.match(component, /<div class="output-device-list" role="list"/);
});

test('the settings page and the quick-settings drawer render the same component (one style for both)', () => {
  // Both places render the shared settingsPanels snippet, which holds the one OutputDevicesSettings.
  assert.equal(app.match(/<OutputDevicesSettings\b/g)?.length, 1);
  assert.match(app, /\{#snippet settingsPanels\(/);
  assert.ok((app.match(/\{@render settingsPanels\(/g) ?? []).length >= 2);
});

test('a device name too long for the channel-map matrix scrolls with the play dock marquee, a short one stays still', () => {
  const marquee = readFileSync(new URL('../src/lib/dock-marquee.ts', import.meta.url), 'utf8');
  assert.match(marquee, /export function dockMarquee\(/);
  assert.match(marquee, /classList\.toggle\('is-overflowing', overflowing\)/, 'only overflowing text moves');
  // One marquee, shared: the play dock and the matrix both import it, App.svelte no longer has its own copy.
  assert.match(app, /import \{ dockMarquee \} from '\.\/lib\/dock-marquee\.js';/);
  assert.doesNotMatch(app, /function dockMarquee\(/);
  assert.match(component, /import \{ dockMarquee \} from '\.\/dock-marquee\.js';/);
  const device = component.slice(component.indexOf('class="channel-matrix-device"'));
  assert.match(
    device.slice(0, device.indexOf('</button>')),
    /<span class="dock-marquee"><span class="dock-marquee-text" use:dockMarquee>\{shortDeviceName\(deviceLabel\(index\)\)\}<\/span><\/span>$/,
  );
  const styles = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(styles, /\.dock-marquee-text\.is-overflowing \{[^}]*animation: dock-marquee-scroll/);
});
test('the device list hands the wheel on to the page at its ends (scroll-chain action), and only there', () => {
  const markup = component.slice(component.indexOf('<div class="output-device-list"'));
  const list = markup.slice(0, markup.indexOf('>'));
  assert.match(list, /use:scrollChain/);
  assert.match(component, /import \{ scrollChain \} from '\.\/scroll-chain\.js';/);
  assert.doesNotMatch(list, /on(wheel|touchmove|touchstart)/, 'no other handlers on the list');
  assert.doesNotMatch(component, /stopPropagation\(\)[^\n]*(wheel|touch)|(wheel|touch)[^\n]*stopPropagation\(\)/);
  const chain = readFileSync(new URL('../src/lib/scroll-chain.js', import.meta.url), 'utf8');
  assert.match(chain, /if \(event\.defaultPrevented \|\| event\.ctrlKey\) return;/, 'value wheels and zoom are left alone');
  assert.match(chain, /Math\.abs\(event\.deltaX\) > Math\.abs\(event\.deltaY\)\) return;/, 'horizontal wheels are left alone');
  assert.match(chain, /addEventListener\('wheel', onWheel, \{ passive: false \}\)/);
  assert.doesNotMatch(chain, /touch(start|move)/, 'touch keeps native chaining');
  // The outer containers really scroll, so the chained scroll has somewhere to go.
  const styles = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(styles, /\.page-scroll \{[^}]*overflow: auto;/);
  assert.match(styles, /\.quick-settings-drawer-scroll \{[^}]*overflow: auto;/);
});

test('a wheel step is split between the list and the outer scroller', async () => {
  const { splitWheelDelta, wheelDeltaPx, WHEEL_LINE_PX } = await import('../src/lib/scroll-chain.js');
  const box = (scrollTop) => ({ scrollTop, scrollHeight: 1000, clientHeight: 400 });
  assert.deepEqual(splitWheelDelta(100, box(200)), { inner: 100, outer: 0 }, 'room left: the list scrolls natively');
  assert.deepEqual(splitWheelDelta(100, box(560)), { inner: 40, outer: 60 }, 'the rest of the list first, the excess outside');
  assert.deepEqual(splitWheelDelta(100, box(600)), { inner: 0, outer: 100 }, 'at the bottom all goes outside');
  assert.deepEqual(splitWheelDelta(-100, box(30)), { inner: -30, outer: -70 });
  assert.deepEqual(splitWheelDelta(-100, box(0)), { inner: 0, outer: -100 }, 'at the top, upwards');
  assert.deepEqual(splitWheelDelta(50, { scrollTop: 0, scrollHeight: 300, clientHeight: 300 }), { inner: 0, outer: 50 }, 'a list that fits');
  assert.equal(wheelDeltaPx(3, 1, 400), 3 * WHEEL_LINE_PX, 'line mode');
  assert.equal(wheelDeltaPx(1, 2, 400), 400, 'page mode uses the visible height');
  assert.equal(wheelDeltaPx(57, 0, 400), 57);
  assert.equal(wheelDeltaPx(Number.NaN, 0, 400), 0);
});
test('handed-on wheel steps scroll smoothly and add up on the pending target', async () => {
  const { readFile } = await import('node:fs/promises');
  const src = await readFile(new URL('../src/lib/scroll-chain.js', import.meta.url), 'utf8');
  const { nextSmoothTarget, SMOOTH_TARGET_TTL_MS } = await import('../src/lib/scroll-chain.js');
  assert.doesNotMatch(src, /behavior: 'auto' \}/);
  assert.match(src, /prefers-reduced-motion: reduce\)'\)\.matches \? 'auto' : 'smooth'/);
  assert.match(src, /element\.scrollTo\(\{ top: target, behavior: scrollBehavior\(\) \}\)/);
  assert.match(src, /if \(inner !== 0\) smoothScrollBy\(node, inner, now\)/); // the list's own share is smooth too
  assert.equal(nextSmoothTarget(null, 100, 50, 1000, 0), 150);
  assert.equal(nextSmoothTarget({ target: 150, at: 0 }, 120, 50, 1000, 100), 200, 'adds to the pending target');
  assert.equal(nextSmoothTarget({ target: 150, at: 0 }, 120, -50, 1000, 100), 70, 'turning around starts from here');
  assert.equal(nextSmoothTarget({ target: 150, at: 0 }, 150, 50, 1000, 100), 200, 'reached: from here');
  assert.equal(nextSmoothTarget({ target: 900, at: 0 }, 120, 50, 1000, SMOOTH_TARGET_TTL_MS), 170, 'stale target dropped');
  assert.equal(nextSmoothTarget({ target: 980, at: 0 }, 900, 50, 1000, 10), 1000, 'clamped to the end');
  assert.equal(nextSmoothTarget(null, 10, -50, 1000, 0), 0, 'clamped to the top');
});
