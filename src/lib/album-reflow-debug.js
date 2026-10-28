// Opt-in measurement of the album detail summary reflow (the text beside the cover re-wraps while the cover collapses).
// Enable with localStorage `moe.debug.albumReflow` = "1" and reload; while the page scrolls, once a second it logs
// "[album-reflow] frames N, forced layout avg X ms max Y ms, summary W x H px" to the console (logcat tag chromium on Android).

export const ALBUM_REFLOW_FLAG = 'moe.debug.albumReflow';

/** Whether the probe is switched on (storage can be blocked: then it is off). */
export function albumReflowProbeEnabled(storage = globalThis.localStorage) {
  try { return storage?.getItem(ALBUM_REFLOW_FLAG) === '1'; } catch { return false; }
}

/** Aggregates per-frame layout costs between two reports. */
export function createReflowStats() {
  let frames = 0;
  let total = 0;
  let max = 0;
  let width = 0;
  let height = 0;
  return {
    /** @param {number} costMs @param {number} summaryWidth @param {number} summaryHeight */
    add(costMs, summaryWidth, summaryHeight) {
      frames += 1;
      total += costMs;
      if (costMs > max) max = costMs;
      width = summaryWidth;
      height = summaryHeight;
    },
    /** The report line for the frames since the last one, or null when there were none. */
    take() {
      if (frames === 0) return null;
      const line = `[album-reflow] frames ${frames}, forced layout avg ${(total / frames).toFixed(2)} ms max ${max.toFixed(2)} ms, summary ${Math.round(width)} x ${Math.round(height)} px`;
      frames = 0; total = 0; max = 0;
      return line;
    },
  };
}

/**
 * While `shell` scrolls (scroll events do not bubble: captured), once per frame force the summary's layout and time it: the
 * cost of re-wrapping the text in the contained heading, an upper bound of what a frame spends on it.
 * @param {HTMLElement} shell @param {HTMLElement} summary @param {(line: string) => void} [log]
 * @returns {() => void} stop
 */
export function startAlbumReflowProbe(shell, summary, log = (/** @type {string} */ line) => console.info(line)) {
  const stats = createReflowStats();
  let queued = false;
  const frame = () => {
    queued = false;
    const started = performance.now();
    const width = summary.offsetWidth;
    const height = summary.offsetHeight;
    stats.add(performance.now() - started, width, height);
  };
  const onScroll = () => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(frame);
  };
  shell.addEventListener('scroll', onScroll, { capture: true, passive: true });
  const timer = setInterval(() => { const line = stats.take(); if (line) log(line); }, 1000);
  return () => {
    shell.removeEventListener('scroll', onScroll, { capture: true });
    clearInterval(timer);
  };
}
