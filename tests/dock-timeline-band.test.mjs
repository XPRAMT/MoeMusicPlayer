import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../src/app.css', import.meta.url), 'utf8');

/** Body of the first rule whose selector is exactly `selector`. */
function rule(selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = css.match(new RegExp(`(?:^|\\n)${escaped} \\{([^}]*)\\}`));
  assert.ok(match, `missing rule ${selector}`);
  return match[1];
}

test('edge timeline is its own band row between the content area and the dock', () => {
  const shell = rule('.app-shell');
  assert.match(shell, /--dock-edge-progress-track: 3px;/);
  assert.match(shell, /--dock-timeline-band: 0px;/);
  assert.match(shell, /grid-template-rows: minmax\(0, 1fr\) var\(--dock-timeline-band\) auto;/);
  // The band is exactly one track high, only with the edge timeline.
  assert.match(rule('.app-shell:has(> .player-dock[data-timeline-style="edge"])'), /--dock-timeline-band: var\(--dock-edge-progress-track\);/);
  // Content (row 1) ends at the band top; the dock sits below the band.
  assert.match(rule('.workspace'), /grid-row: 1;/);
  assert.match(rule('.player-dock'), /grid-row: 3;/);
});

test('edge timeline hit area is the band plus 10px down into the dock, never up into the content', () => {
  assert.match(rule('.app-shell'), /--dock-edge-progress-hit-below: 10px;/);
  const row = rule('.player-dock[data-timeline-style="edge"] .progress-row');
  // The row starts at the band top: exactly one band (track) above the dock top, no extra upward margin.
  assert.match(row, /top: calc\(-1 \* var\(--dock-progress-track\)\);/);
  assert.match(row, /margin-top: 0;/);
  assert.match(row, /grid-template-rows: var\(--dock-progress-track\) auto;/);
  assert.match(rule('.player-dock[data-timeline-style="edge"]'), /--dock-progress-track: var\(--dock-edge-progress-track\);/);
  const hit = rule('.player-dock[data-timeline-style="edge"] .progress-hit');
  assert.match(hit, /height: var\(--dock-progress-track\);/);
  // The slider (the click target) starts at the band top and extends only downward.
  const slider = rule('.player-dock[data-timeline-style="edge"] .progress-slider');
  assert.match(slider, /inset: 0 0 calc\(-1 \* var\(--dock-edge-progress-hit-below\)\);/);
  // No edge rule reaches upward with a negative top margin or the old centred hit band.
  for (const match of css.matchAll(/\.player-dock\[data-timeline-style="edge"\][^{]*\{([^}]*)\}/g)) {
    assert.doesNotMatch(match[1], /margin-top: calc\(-/);
  }
  assert.doesNotMatch(css, /--dock-edge-progress-hit:|--dock-edge-progress-row|--dock-progress-hit|--dock-progress-row/);
  // Time labels keep their place under the dock top.
  assert.match(rule('.app-shell'), /--dock-edge-progress-label-top: 11\.5px;/);
  assert.match(row, /row-gap: var\(--dock-edge-progress-label-top\);/);
});

test('narrow nav panel stops at the top of the timeline band', () => {
  assert.match(css, /\.app-shell \.sidebar\.floating-surface \{[^}]*--nav-panel-dock-overhang: var\(--dock-timeline-band\);[^}]*bottom: calc\(var\(--dock-height\) \+ var\(--android-safe-bottom, 0px\) \+ var\(--nav-panel-dock-overhang\)\);/s);
});

test('the pinned 設定 button sits flush on the timeline band (no sidebar bottom padding)', () => {
  // Wide sidebar: bottom padding 0, so the button's bottom edge is the sidebar's bottom edge,
  // which is the top of the timeline band row.
  assert.match(rule('.sidebar'), /padding: var\(--sidebar-brand-inset\) var\(--sidebar-pad-x\) 0;/);
  assert.doesNotMatch(css, /\.sidebar \{[^}]*padding-bottom: (?!0[;\s])/s);
  // The button keeps its own 8 px top padding; only the gap below it is gone.
  assert.match(rule('.sidebar-settings'), /margin-top: auto;[^}]*padding-top: 8px;/s);
  // Narrow floating nav panel: same 0 px gap above its bottom edge (which already clears the
  // Android nav bar through --android-safe-bottom in `bottom`).
  const panel = css.match(/\.app-shell \.sidebar\.floating-surface \{([^}]*)\}/)?.[1] ?? '';
  assert.match(panel, /padding-bottom: 0;/);
  assert.match(panel, /bottom: calc\(var\(--dock-height\) \+ var\(--android-safe-bottom, 0px\) \+ var\(--nav-panel-dock-overhang\)\);/);
});
