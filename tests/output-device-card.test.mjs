import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { chooseCardOnClick, chooseCardOnEnter } from '../src/lib/output-device-card.js';

/** A card whose hidden input counts its clicks; `inside` is the selector the clicked element is inside of. */
function card(inside) {
  const input = { clicks: 0, click() { this.clicks += 1; } };
  const target = { closest: (/** @type {string} */ selector) => (inside && selector.split(', ').includes(inside) ? {} : null) };
  return { input, event: { currentTarget: { querySelector: () => input }, target } };
}

test('a click on the card free space chooses it through its input', () => {
  const free = card(null);
  chooseCardOnClick(free.event);
  assert.equal(free.input.clicks, 1);
});

test('the card controls keep their own clicks (volume, delay, remove, 前往授權, the label that toggles natively)', () => {
  for (const control of ['input', 'button', 'label', '[role="slider"]', 'select', 'a']) {
    const inside = card(control);
    chooseCardOnClick(inside.event);
    assert.equal(inside.input.clicks, 0, control);
  }
});

test('Enter switches the card like Space; a held key does not repeat it', () => {
  let clicks = 0;
  let prevented = false;
  const input = { click: () => { clicks += 1; } };
  chooseCardOnEnter({ key: 'Enter', preventDefault: () => { prevented = true; }, currentTarget: input });
  assert.equal(clicks, 1);
  assert.ok(prevented);
  chooseCardOnEnter({ key: 'Enter', repeat: true, preventDefault() {}, currentTarget: input });
  chooseCardOnEnter({ key: ' ', preventDefault() {}, currentTarget: input });
  assert.equal(clicks, 1);
});

test('both device lists draw cards without a visible checkbox or radio; the input stays for the keyboard', () => {
  const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');
  const hidden = /\.output-device-input \{([^}]*)\}/.exec(css)?.[1] ?? '';
  assert.match(hidden, /clip-path: inset\(50%\)/);
  assert.match(hidden, /opacity: 0/);
  assert.match(css, /\.output-device:has\(\.output-device-input:focus-visible\) \{\s*outline: var\(--focus-ring\);/);
  assert.match(css, /\.output-device\.is-playing \{\s*border-color: rgba\(var\(--accent-rgb\), 0\.64\);\s*background: rgba\(var\(--accent-rgb\), 0\.09\);/);
  for (const [file, type] of [['OutputDevicesSettings.svelte', 'checkbox'], ['AndroidOutputDevicesSettings.svelte', 'radio']]) {
    const component = readFileSync(new URL(`../src/lib/${file}`, import.meta.url), 'utf8');
    assert.match(component, new RegExp(String.raw`class="output-device-input"\s+type="${type}"`), file);
    assert.match(component, /onclick=\{chooseCardOnClick\}/, file);
    assert.match(component, /onkeydown=\{chooseCardOnEnter\}/, file);
    assert.match(component, /class:is-locked=/, file);
  }
});
