/** Narrow layouts (<= 820 CSS px) replace the sidebar with a MOE-logo toggled overlay panel. */
export const NAV_PANEL_BREAKPOINT_PX = 820;
export const NAV_PANEL_MEDIA_QUERY = `(max-width: ${NAV_PANEL_BREAKPOINT_PX}px)`;
/** Clicking any of these inside the open panel navigates, so the panel closes afterwards. */
export const NAV_PANEL_NAVIGATION_SELECTOR = '.nav-link, .playlist-tree-open, .playlist-tree-item';

/** @param {number} viewportWidthPx */
export function isNavPanelLayout(viewportWidthPx) {
  return Number.isFinite(viewportWidthPx) && viewportWidthPx <= NAV_PANEL_BREAKPOINT_PX;
}

/**
 * Next open state of the narrow navigation panel.
 * @param {boolean} open
 * @param {'toggle' | 'escape' | 'outside' | 'navigate' | 'widen' | 'now-playing'} event
 * @param {boolean} [narrow] whether the panel layout is active; outside it the panel is always closed
 */
export function reduceNavPanelOpen(open, event, narrow = true) {
  if (!narrow) return false;
  if (event === 'toggle') return !open;
  if (event === 'escape' || event === 'outside' || event === 'navigate' || event === 'widen' || event === 'now-playing') return false;
  return open;
}
