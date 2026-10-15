import test from 'node:test';
import assert from 'node:assert/strict';
import { NAV_PANEL_BREAKPOINT_PX, NAV_PANEL_MEDIA_QUERY, NAV_PANEL_NAVIGATION_SELECTOR, isNavPanelLayout, reduceNavPanelOpen } from '../src/lib/nav-panel.js';

test('narrow navigation panel reuses the existing 820px breakpoint', () => {
  assert.equal(NAV_PANEL_BREAKPOINT_PX, 820);
  assert.equal(NAV_PANEL_MEDIA_QUERY, '(max-width: 820px)');
  assert.equal(isNavPanelLayout(820), true);
  assert.equal(isNavPanelLayout(390), true);
  assert.equal(isNavPanelLayout(821), false);
  assert.equal(isNavPanelLayout(Number.NaN), false);
});

test('logo toggle opens and closes the panel; every dismissal closes it', () => {
  assert.equal(reduceNavPanelOpen(false, 'toggle'), true);
  assert.equal(reduceNavPanelOpen(true, 'toggle'), false);
  for (const event of /** @type {const} */ (['escape', 'outside', 'navigate', 'widen', 'now-playing'])) {
    assert.equal(reduceNavPanelOpen(true, event), false, event);
    assert.equal(reduceNavPanelOpen(false, event), false, event);
  }
});

test('the panel never opens outside the narrow layout', () => {
  assert.equal(reduceNavPanelOpen(false, 'toggle', false), false);
  assert.equal(reduceNavPanelOpen(true, 'toggle', false), false);
});

test('navigation targets cover primary links, settings, and playlist entries', () => {
  for (const cls of ['.nav-link', '.playlist-tree-open', '.playlist-tree-item']) assert.ok(NAV_PANEL_NAVIGATION_SELECTOR.split(', ').includes(cls));
});
