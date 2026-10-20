// Three ASCII dots: the middle dot (U+00B7) is drawn full-width by the CJK fonts of Android, which wastes a column of space.
const ELLIPSIS = '...';

/**
 * Keep the start and end of text that does not fit, hiding the middle.
 * `measure` returns a width in the same unit as `maxWidth`.
 * @param {string} text
 * @param {number} maxWidth
 * @param {(value: string) => number} measure
 * @returns {string}
 */
export function fitMiddleEllipsis(text, maxWidth, measure) {
  if (!text) return '';
  if (!(maxWidth > 0) || measure(text) <= maxWidth) return text;
  const ellipsisWidth = measure(ELLIPSIS);
  const available = maxWidth - ellipsisWidth;
  if (available <= 0) return ELLIPSIS;

  const half = available / 2;
  let low = 0;
  let high = text.length;
  let head = 0;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (measure(text.slice(0, mid)) <= half) {
      head = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  const tailBudget = available - measure(text.slice(0, head));
  low = 0;
  high = text.length - head;
  let tail = 0;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (measure(text.slice(text.length - mid)) <= tailBudget) {
      tail = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }

  if (head + tail >= text.length) return text;
  if (head === 0 && tail === 0) return ELLIPSIS;
  return `${text.slice(0, head)}${ELLIPSIS}${text.slice(text.length - tail)}`;
}

/**
 * Keep the start of text that does not fit, hiding the end.
 * @param {string} text
 * @param {number} maxWidth
 * @param {(value: string) => number} measure
 * @returns {string}
 */
export function fitEndEllipsis(text, maxWidth, measure) {
  if (!text) return '';
  if (!(maxWidth > 0) || measure(text) <= maxWidth) return text;
  const ellipsisWidth = measure(ELLIPSIS);
  if (ellipsisWidth >= maxWidth) return ELLIPSIS;

  const characters = Array.from(text);
  let low = 0;
  let high = characters.length;
  let fittingLength = 0;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (measure(characters.slice(0, mid).join('') + ELLIPSIS) <= maxWidth) {
      fittingLength = mid;
      low = mid + 1;
    } else {
      high = mid - 1;
    }
  }
  return `${characters.slice(0, fittingLength).join('')}${ELLIPSIS}`;
}
