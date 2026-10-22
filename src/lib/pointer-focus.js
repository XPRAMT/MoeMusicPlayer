// Pointer focus release (AGENTS.md): a mouse or touch press on a button, range, checkbox/radio or select must not
// leave keyboard focus there (global shortcuts would be swallowed). Keyboard focus (Tab, Enter/Space) is untouched.

const TEXT_INPUT_TYPES = new Set(['text', 'search', 'number', 'email', 'password', 'tel', 'url', 'date', 'time', 'datetime-local', 'month', 'week']);
const CONTROL_SELECTOR = 'button, input, select, [role="button"], [role="slider"], [role="checkbox"], [role="radio"], [role="switch"], [role="tab"]';
const MODAL_SELECTOR = 'dialog[open], [aria-modal="true"]';

/**
 * How focus on a control is released after a pointer press.
 * @param {{ tagName: string, type?: string, isContentEditable?: boolean }} control
 * @returns {'blur' | 'after-change' | 'keep'}
 */
export function pointerFocusReleaseKind(control) {
  if (control.isContentEditable) return 'keep';
  const tag = control.tagName.toUpperCase();
  if (tag === 'TEXTAREA') return 'keep';
  if (tag === 'SELECT') return 'after-change'; // its list is open now; let go once a value is picked
  if (tag === 'INPUT') {
    const type = (control.type || 'text').toLowerCase();
    if (TEXT_INPUT_TYPES.has(type)) return 'keep';
    if (type === 'range' || type === 'checkbox' || type === 'radio' || type === 'button' || type === 'submit' || type === 'reset' || type === 'color') return 'blur';
    return 'keep';
  }
  return 'blur';
}

/** @param {Element} control */
function releaseFocus(control) {
  if (document.activeElement !== control) return; // a click handler moved focus on purpose (an input it opened)
  const modal = control.closest(MODAL_SELECTOR);
  if (modal) {
    // Keep focus inside the modal so its Escape / focus trap keeps working; the container itself takes it.
    if (modal instanceof HTMLElement && modal !== control && (modal.tabIndex >= 0 || modal.hasAttribute('tabindex') || modal.tagName === 'DIALOG')) {
      if (!modal.hasAttribute('tabindex')) modal.setAttribute('tabindex', '-1');
      modal.focus({ preventScroll: true });
    }
    return;
  }
  if (control instanceof HTMLElement) control.blur();
}

/**
 * Install once on the document. Runs after the press (pointerup + a task) so drags finish and click handlers run first.
 * @param {Document} doc
 * @returns {() => void}
 */
export function installPointerFocusRelease(doc = document) {
  /** @type {Element | null} */
  let pressed = null;
  const onDown = (/** @type {PointerEvent} */ event) => {
    const target = event.target instanceof Element ? event.target.closest(CONTROL_SELECTOR) : null;
    pressed = target;
  };
  const onUp = () => {
    const control = pressed;
    pressed = null;
    if (!control) return;
    const kind = pointerFocusReleaseKind(/** @type {HTMLInputElement} */ (control));
    if (kind === 'keep') return;
    if (kind === 'after-change') {
      control.addEventListener('change', () => setTimeout(() => releaseFocus(control), 0), { once: true });
      return;
    }
    setTimeout(() => releaseFocus(control), 0);
  };
  doc.addEventListener('pointerdown', onDown, true);
  doc.addEventListener('pointerup', onUp, true);
  doc.addEventListener('pointercancel', onUp, true);
  return () => {
    doc.removeEventListener('pointerdown', onDown, true);
    doc.removeEventListener('pointerup', onUp, true);
    doc.removeEventListener('pointercancel', onUp, true);
  };
}
