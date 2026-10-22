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
  const list = rule(component, '.output-device-list');
  assert.match(list, /max-height:\s*50vh;/);
  assert.match(list, /max-height:\s*50dvh;/);
  assert.ok(list.indexOf('50vh') < list.indexOf('50dvh'), 'the dvh value comes last and wins where supported');
  assert.match(list, /overflow-y:\s*auto;/);
  // At its top or bottom the wheel and touch scroll chain on to the settings page / drawer: no overscroll-behavior.
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
test('the device list does not swallow wheel or touch scrolling', () => {
  const markup = component.slice(component.indexOf('<div class="output-device-list"'));
  const list = markup.slice(0, markup.indexOf('>'));
  assert.doesNotMatch(list, /on(wheel|touchmove|touchstart)/, 'no handlers that could preventDefault or stopPropagation');
  assert.doesNotMatch(component, /stopPropagation\(\)[^\n]*(wheel|touch)|(wheel|touch)[^\n]*stopPropagation\(\)/);
  // The outer containers really scroll, so the chained scroll has somewhere to go.
  const styles = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  assert.match(styles, /\.page-scroll \{[^}]*overflow: auto;/);
  assert.match(styles, /\.quick-settings-drawer-scroll \{[^}]*overflow: auto;/);
});