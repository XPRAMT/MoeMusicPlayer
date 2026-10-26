// Quick index popovers in the browser top layer (HTML Popover API, `popover="manual"`).
//
// A list can sit inside an ancestor that would capture a `position: fixed` popover: the quick-settings drawer has
// `backdrop-filter` (so it is both the containing block and the backdrop root) and `overflow: hidden`. An element
// shown with `showPopover()` is rendered in the top layer instead: its containing block is the viewport, it is not
// clipped, it is painted above everything shown before it (a modal `<dialog>` included; it stays interactive because
// it is a DOM descendant of that dialog), and its own `backdrop-filter` blurs the page like any floating surface.
// `manual` keeps the component's own closing rules (transparent click-catcher, Esc, choosing a label); the UA does
// not light-dismiss it. Without Popover API support the element simply stays where it is in the page.

/**
 * Svelte action: shows `node` as a manual popover once it is mounted, then calls `onShown` (focus has to wait until
 * the popover is shown: a hidden `[popover]` is `display: none`). Hides it again when it is removed.
 * @param {HTMLElement} node
 * @param {{ onShown?: (node: HTMLElement) => void }} [options]
 */
export function topLayerPopover(node, options = {}) {
  if (typeof node.showPopover === 'function' && !isPopoverOpen(node)) {
    try {
      node.showPopover();
    } catch {
      // Not connected or already shown elsewhere: it stays a normal element.
    }
  }
  options.onShown?.(node);
  return {
    destroy() {
      if (typeof node.hidePopover === 'function' && isPopoverOpen(node)) {
        try {
          node.hidePopover();
        } catch {
          // Already hidden (removing it from the document hides it too).
        }
      }
    },
  };
}

/** @param {HTMLElement} node */
function isPopoverOpen(node) {
  try {
    return typeof node.matches === 'function' && node.matches(':popover-open');
  } catch {
    return false;
  }
}
