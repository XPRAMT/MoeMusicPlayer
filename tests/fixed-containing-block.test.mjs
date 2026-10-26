import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createsFixedContainingBlock,
  findFixedContainingBlock,
  fixedPlacement,
  viewportToContainingBlock,
} from '../src/lib/fixed-containing-block.js';

const plain = { transform: 'none', filter: 'none', backdropFilter: 'none', perspective: 'none', contain: 'none', willChange: 'auto', containerType: 'normal' };

test('only styles that capture fixed descendants count as containing blocks', () => {
  assert.equal(createsFixedContainingBlock(plain), false);
  assert.equal(createsFixedContainingBlock(null), false);
  assert.equal(createsFixedContainingBlock({ ...plain, backdropFilter: 'blur(5px)' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, transform: 'matrix(1, 0, 0, 1, 0, 0)' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, filter: 'blur(2px)' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, perspective: '100px' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, contain: 'paint' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, contain: 'strict' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, contain: 'size' }), false);
  assert.equal(createsFixedContainingBlock({ ...plain, willChange: 'transform' }), true);
  assert.equal(createsFixedContainingBlock({ ...plain, willChange: 'opacity' }), false);
  assert.equal(createsFixedContainingBlock({ ...plain, containerType: 'inline-size' }), true);
});

test('the nearest capturing ancestor is found, or null', () => {
  const root = { parentElement: null, style: plain };
  const drawer = { parentElement: root, style: { ...plain, backdropFilter: 'blur(5px)' } };
  const list = { parentElement: drawer, style: plain };
  const popover = { parentElement: list, style: plain };
  const getStyle = (node) => node.style;
  assert.equal(findFixedContainingBlock(popover, getStyle), drawer);
  const libraryPopover = { parentElement: { parentElement: root, style: plain }, style: plain };
  assert.equal(findFixedContainingBlock(libraryPopover, getStyle), null);
});

test('without a containing block the viewport point is used unchanged', () => {
  assert.deepEqual(viewportToContainingBlock(640, 400, null), { left: 640, top: 400 });
  const placed = fixedPlacement({ centreX: 640, centreY: 400, width: 193, height: 94, viewportWidth: 1536, viewportHeight: 832 });
  assert.equal(placed.left, 640);
  assert.equal(placed.top, 400);
});

const drawer = { left: 1076, top: 32, width: 460, height: 726, clientLeft: 0, clientTop: 0, clientWidth: 460, clientHeight: 726, scrollLeft: 0, scrollTop: 0 };

test('inside a drawer-like containing block the point is offset by the drawer position', () => {
  // The font list is centred at viewport (1306, 500); the drawer starts at (1076, 32).
  const placed = fixedPlacement({ centreX: 1306, centreY: 500, width: 193, height: 94, viewportWidth: 1536, viewportHeight: 832, block: drawer });
  assert.equal(placed.left, 230);
  assert.equal(placed.top, 468);
  assert.equal(placed.viewportX, 1306);
  assert.equal(placed.viewportY, 500);
});

test('border and scroll of the containing block are accounted for', () => {
  const scrolled = { ...drawer, clientLeft: 1, clientTop: 1, clientWidth: 458, clientHeight: 724, scrollTop: 300 };
  assert.deepEqual(viewportToContainingBlock(1306, 500, scrolled), { left: 229, top: 767 });
});

test('the popover is clamped inside the window and the drawer', () => {
  // Desired centre near the right edge: the 193 px box must end 12 px before the window (and drawer) edge.
  const placed = fixedPlacement({ centreX: 1500, centreY: 800, width: 193, height: 94, viewportWidth: 1536, viewportHeight: 832, block: drawer });
  assert.equal(placed.viewportX, 1536 - 12 - 193 / 2);
  assert.equal(placed.viewportY, 32 + 726 - 12 - 94 / 2);
  assert.equal(placed.left, placed.viewportX - 1076);
  assert.equal(placed.top, placed.viewportY - 32);
  const library = fixedPlacement({ centreX: 1500, centreY: 400, width: 193, height: 94, viewportWidth: 1536, viewportHeight: 832 });
  assert.equal(library.left, 1536 - 12 - 193 / 2);
  // A box larger than the space is centred in it.
  const huge = fixedPlacement({ centreX: 1500, centreY: 400, width: 600, height: 94, viewportWidth: 1536, viewportHeight: 832, block: drawer });
  assert.equal(huge.viewportX, 1076 + 230);
});
