import { NO_SAFE_INSETS, readSafeInsets } from './safe-viewport.js';

const APP_TOOLTIP_ID = 'moe-app-tooltip';
const APP_TOOLTIP_DELAY_MS = 500;
const APP_TOOLTIP_VIEWPORT_GAP = 8;

/** @type {HTMLDivElement | null} */
let tooltipElement = null;
/** @type {HTMLElement | null} */
let activeTarget = null;

/**
 * Place a tooltip near its anchor, preferring above unless that would clip.
 * @param {{ left: number, right: number, top: number, bottom: number }} anchor
 * @param {number} tooltipWidth
 * @param {number} tooltipHeight
 * @param {number} viewportWidth
 * @param {number} viewportHeight
 * @param {import('./safe-viewport.js').SafeInsets} [safeInsets] system-bar insets kept clear (Android)
 */
export function getAppTooltipPosition(anchor, tooltipWidth, tooltipHeight, viewportWidth, viewportHeight, safeInsets = NO_SAFE_INSETS) {
  const gap = APP_TOOLTIP_VIEWPORT_GAP;
  const minLeft = safeInsets.left + gap;
  const maxRight = viewportWidth - safeInsets.right - gap;
  const minTop = safeInsets.top + gap;
  const maxBottom = viewportHeight - safeInsets.bottom - gap;
  const centeredLeft = (anchor.left + anchor.right - tooltipWidth) / 2;
  const left = Math.min(Math.max(centeredLeft, minLeft), Math.max(minLeft, maxRight - tooltipWidth));
  const above = anchor.top - tooltipHeight - gap;
  const below = anchor.bottom + gap;
  const maxTop = Math.max(minTop, maxBottom - tooltipHeight);
  const aboveFits = above >= minTop;
  const belowFits = below + tooltipHeight <= maxBottom;
  const top = aboveFits
    ? above
    : belowFits
      ? below
      : Math.min(Math.max(anchor.top - (viewportHeight - anchor.bottom), minTop), maxTop);
  return { left, top };
}

function getTooltipElement() {
  if (tooltipElement?.isConnected) return tooltipElement;
  tooltipElement = document.createElement('div');
  tooltipElement.id = APP_TOOLTIP_ID;
  tooltipElement.className = 'app-tooltip floating-surface';
  tooltipElement.setAttribute('role', 'tooltip');
  tooltipElement.setAttribute('aria-hidden', 'true');
  document.body.append(tooltipElement);
  return tooltipElement;
}

/** @param {HTMLElement | null} target */
function removeTooltipDescription(target) {
  if (!target) return;
  const ids = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean);
  const remaining = ids.filter((id) => id !== APP_TOOLTIP_ID);
  if (remaining.length) target.setAttribute('aria-describedby', remaining.join(' '));
  else target.removeAttribute('aria-describedby');
}

function hideTooltip() {
  window.removeEventListener('resize', repositionTooltip);
  window.removeEventListener('scroll', hideTooltip, true);
  if (tooltipElement) {
    tooltipElement.classList.remove('is-visible');
    tooltipElement.setAttribute('aria-hidden', 'true');
  }
  removeTooltipDescription(activeTarget);
  activeTarget = null;
}

function repositionTooltip() {
  if (!activeTarget || !tooltipElement) return;
  const rect = activeTarget.getBoundingClientRect();
  const position = getAppTooltipPosition(rect, tooltipElement.offsetWidth, tooltipElement.offsetHeight, window.innerWidth, window.innerHeight, readSafeInsets());
  tooltipElement.style.left = `${position.left}px`;
  tooltipElement.style.top = `${position.top}px`;
}

/** @param {HTMLElement} target @param {string} text */
function showTooltip(target, text) {
  const element = getTooltipElement();
  if (activeTarget !== target) hideTooltip();
  activeTarget = target;
  const describedBy = (target.getAttribute('aria-describedby') ?? '').split(/\s+/).filter(Boolean);
  if (!describedBy.includes(APP_TOOLTIP_ID)) {
    target.setAttribute('aria-describedby', [...describedBy, APP_TOOLTIP_ID].join(' '));
  }
  element.textContent = text;
  // The tooltip is portaled to body; copy the settings values inherited from
  // the app shell so the floating surface exactly tracks its configured skin.
  const targetStyle = window.getComputedStyle(target);
  element.style.setProperty('--quick-settings-opacity', targetStyle.getPropertyValue('--quick-settings-opacity').trim() || '70%');
  element.style.setProperty('--quick-settings-blur', targetStyle.getPropertyValue('--quick-settings-blur').trim() || '12px');
  element.setAttribute('aria-hidden', 'false');
  element.classList.add('is-visible');
  repositionTooltip();
  window.addEventListener('resize', repositionTooltip);
  window.addEventListener('scroll', hideTooltip, true);
}

/**
 * Svelte action that replaces the browser's unstyled `title` popup with the
 * shared quick-settings floating surface while preserving hover and focus use.
 * @param {HTMLElement} node
 * @param {string | null | undefined} value
 */
export function appTooltip(node, value) {
  let text = normalizeText(value);
  let timer = 0;
  let pointerInside = false;
  let focused = false;
  let pointerDismissedFocus = false;

  function hasActiveTrigger() {
    return pointerInside || (focused && !pointerDismissedFocus);
  }

  function cancelTimer() {
    if (timer) window.clearTimeout(timer);
    timer = 0;
  }

  function schedule() {
    cancelTimer();
    if (!text || !hasActiveTrigger()) {
      if (activeTarget === node) hideTooltip();
      return;
    }
    timer = window.setTimeout(() => {
      timer = 0;
      if (hasActiveTrigger()) showTooltip(node, text);
    }, APP_TOOLTIP_DELAY_MS);
  }

  /** @param {PointerEvent} event */
  function onPointerEnter(event) {
    if (event.pointerType === 'touch') return;
    pointerInside = true;
    pointerDismissedFocus = false;
    schedule();
  }
  function onPointerLeave() {
    pointerInside = false;
    schedule();
  }
  function onFocusIn() {
    focused = true;
    pointerDismissedFocus = false;
    schedule();
  }
  /** @param {FocusEvent} event */
  function onFocusOut(event) {
    const next = event.relatedTarget;
    if (next instanceof Node && node.contains(next)) return;
    focused = false;
    pointerDismissedFocus = false;
    schedule();
  }
  /** @param {MouseEvent} event */
  function onClick(event) {
    // A pointer click can leave a button focused after the pointer moves away.
    // Keep keyboard-focus tooltips, but don't let that mouse focus pin a tooltip
    // open; pointerenter or a new focus transition can activate it again.
    if (event.detail > 0) {
      pointerDismissedFocus = true;
      schedule();
    }
  }

  node.addEventListener('pointerenter', onPointerEnter);
  node.addEventListener('pointerleave', onPointerLeave);
  node.addEventListener('focusin', onFocusIn);
  node.addEventListener('focusout', onFocusOut);
  node.addEventListener('click', onClick);

  return {
    /** @param {string | null | undefined} nextValue */
    update(nextValue) {
      text = normalizeText(nextValue);
      if (activeTarget === node && text && hasActiveTrigger()) showTooltip(node, text);
      else if (activeTarget === node) hideTooltip();
      else if (!text) schedule();
    },
    destroy() {
      cancelTimer();
      if (activeTarget === node) hideTooltip();
      node.removeEventListener('pointerenter', onPointerEnter);
      node.removeEventListener('pointerleave', onPointerLeave);
      node.removeEventListener('focusin', onFocusIn);
      node.removeEventListener('focusout', onFocusOut);
      node.removeEventListener('click', onClick);
    },
  };
}

/** @param {string | null | undefined} value */
function normalizeText(value) {
  return typeof value === 'string' && value.trim() ? value : '';
}
