// The playback-device card is the control (ADR 0020, ADR 0044): a click anywhere on it chooses it, like its native
// checkbox (Windows) or radio (Android), which stays in the card, out of sight, for the keyboard and screen readers.

/** Clicks on these act on their own control, never on the card (`[data-own-click]`: an area such as the USB descriptor
 *  reader of ADR 0045, whose text can be selected and whose `<summary>` opens the raw bytes). */
const OWN_CONTROLS = 'input, button, select, textarea, a, label, summary, [role="slider"], [contenteditable], [data-own-click]';

/**
 * Card `onclick`: hand a click on the card's free space to its input (which keeps its own rules: a disabled input does
 * nothing, the change handler applies the choice).
 * @param {{ currentTarget: { querySelector(selector: string): { click(): void } | null } | null, target: unknown }} event
 */
export function chooseCardOnClick(event) {
  const target = /** @type {{ closest?: (selector: string) => unknown }} */ (event.target);
  if (!event.currentTarget || typeof target?.closest !== 'function' || target.closest(OWN_CONTROLS)) return;
  event.currentTarget.querySelector('.output-device-input')?.click();
}

/**
 * Input `onkeydown`: Enter switches the card too (Space already does, natively).
 * @param {{ key: string, repeat?: boolean, preventDefault(): void, currentTarget: { click(): void } | null }} event
 */
export function chooseCardOnEnter(event) {
  if (event.key !== 'Enter' || event.repeat || !event.currentTarget) return;
  event.preventDefault();
  event.currentTarget.click();
}
