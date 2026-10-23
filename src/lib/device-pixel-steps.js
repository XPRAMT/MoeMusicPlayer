// Continuous position animations move in whole device pixels (AGENTS.md rule, 2026-10-23): a linear CSS animation
// interpolates a sub-pixel position on every frame, so its window presents a new frame at the display's full rate even
// when the picture does not visibly change. All WebView2 windows share one GPU process that gets only about three
// presents per vsync through for every window together, so each window that presents every frame takes a share from the
// others (the main window dropped to 90 fps with the taskbar and floating lyrics open). A CSS `steps(n, end)` with one
// step per device pixel of the distance changes the position (and so damages and presents) only when it moves a pixel.

/**
 * Number of `steps()` for an animation that moves `distanceCssPx` CSS px at `devicePixelRatio`: one step per device
 * pixel (rounded up), at least 1. Each step is at most one device pixel, so with `steps(n, end)` the shown position lags
 * the linear one by less than one device pixel, and `forwards` lands exactly on the target.
 * @param {number} distanceCssPx
 * @param {number} devicePixelRatio
 * @returns {number}
 */
export function devicePixelSteps(distanceCssPx, devicePixelRatio) {
  const distance = Number.isFinite(distanceCssPx) ? Math.abs(distanceCssPx) : 0;
  const ratio = Number.isFinite(devicePixelRatio) && devicePixelRatio > 0 ? devicePixelRatio : 1;
  // The small tolerance keeps 2.0000000001 device px (float noise) at 2 steps.
  return Math.max(1, Math.ceil(distance * ratio - 1e-6));
}

/**
 * `steps(n, end)` for a move of `distanceCssPx` CSS px (see devicePixelSteps).
 * @param {number} distanceCssPx
 * @param {number} devicePixelRatio
 * @returns {string}
 */
export function devicePixelEasing(distanceCssPx, devicePixelRatio) {
  return `steps(${devicePixelSteps(distanceCssPx, devicePixelRatio)}, end)`;
}

/**
 * The position `steps(n, end)` shows at `progress` (0..1) of a move from `from` to `to` (for tests and checks).
 * @param {number} from
 * @param {number} to
 * @param {number} steps
 * @param {number} progress
 */
export function steppedPosition(from, to, steps, progress) {
  if (progress >= 1) return to;
  return from + ((to - from) * Math.floor(Math.max(0, progress) * steps)) / steps;
}

/**
 * Call `onChange(ratio)` whenever `window.devicePixelRatio` changes (the window moved to a monitor with another scale,
 * or the scale changed). Returns the function that stops watching.
 * @param {(ratio: number) => void} onChange
 * @returns {() => void}
 */
export function watchDevicePixelRatio(onChange) {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return () => {};
  /** @type {MediaQueryList | null} */
  let query = null;
  let stopped = false;
  const arm = () => {
    if (stopped) return;
    query = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    query.addEventListener('change', changed, { once: true });
  };
  function changed() {
    if (stopped) return;
    onChange(window.devicePixelRatio || 1);
    arm();
  }
  arm();
  return () => {
    stopped = true;
    query?.removeEventListener('change', changed);
  };
}
