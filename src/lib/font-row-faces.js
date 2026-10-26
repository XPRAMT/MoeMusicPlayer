// The font pickers' rows (ADR 0042) are drawn in their own fonts without blocking the page: a row starts in the
// default font and switches once its face has loaded through the FontFace API, off the layout path. The face is the
// family's name subset (a tiny file with only the glyphs of its shown name) when the backend has made one, else the
// installed family by name (`local()`). A row's face is loaded only after the row has stayed in view for
// `FONT_ROW_SETTLE_MS` (so fast scrolling loads nothing), at most `FONT_ROW_MAX_LOADS` at a time; rows that leave the
// view before their turn are skipped, and a loaded face is remembered and never loaded again.

/** How long a row must stay in view before its face is loaded (ms). */
export const FONT_ROW_SETTLE_MS = 100;
/** Faces loading at the same time. */
export const FONT_ROW_MAX_LOADS = 4;

/**
 * What a row loads for `font`: its subset file (through `toUrl`, `convertFileSrc`) or, without one, the installed
 * family by name. `key` identifies the face across rows, pickers and list refreshes.
 * @param {{ family: string, subset?: string } | null | undefined} font
 * @param {(path: string) => string} toUrl
 * @returns {{ key: string, source: string } | null}
 */
export function fontRowSource(font, toUrl) {
  if (!font) return null;
  if (font.subset) return { key: `subset:${font.subset}`, source: `url(${JSON.stringify(toUrl(font.subset))})` };
  // local() matches a full or PostScript name; a regular face's full name is usually the family or "<family> Regular".
  const names = [font.family, `${font.family} Regular`].map((name) => `local(${JSON.stringify(name)})`);
  return { key: `local:${font.family.toLowerCase()}`, source: names.join(', ') };
}

/**
 * The scheduler: `show(row)` when a row comes into view, `hide(key)` when it leaves (or is removed). Pure apart from
 * the injected timer and `load`, so it is tested without a browser.
 * @param {{
 *   load: (row: { key: string, source: string }) => Promise<string>,
 *   onLoaded: (key: string, face: string) => void,
 *   settleMs?: number,
 *   maxLoads?: number,
 *   setTimer?: (callback: () => void, ms: number) => unknown,
 *   clearTimer?: (handle: any) => void,
 * }} options `load` resolves to the CSS family name the face was added under.
 */
export function createFontRowScheduler({
  load,
  onLoaded,
  settleMs = FONT_ROW_SETTLE_MS,
  maxLoads = FONT_ROW_MAX_LOADS,
  setTimer = (callback, ms) => setTimeout(callback, ms),
  clearTimer = (handle) => clearTimeout(handle),
}) {
  /** Rows in view, counted (two pickers may show the same face). @type {Map<string, number>} */
  const visible = new Map();
  /** @type {Map<string, { key: string, source: string }>} */
  const sources = new Map();
  /** @type {Map<string, unknown>} */
  const timers = new Map();
  /** @type {string[]} */
  const queue = [];
  /** @type {Set<string>} */
  const running = new Set();
  /** @type {Map<string, string>} */
  const loaded = new Map();
  /** Faces that failed: not tried again (the row keeps the default font). @type {Set<string>} */
  const failed = new Set();

  function pump() {
    while (running.size < maxLoads && queue.length) {
      const key = /** @type {string} */ (queue.shift());
      const row = sources.get(key);
      if (!visible.has(key) || !row) continue;
      running.add(key);
      Promise.resolve()
        .then(() => load(row))
        .then((face) => {
          loaded.set(key, face);
          onLoaded(key, face);
        })
        .catch(() => failed.add(key))
        .finally(() => {
          running.delete(key);
          pump();
        });
    }
  }

  return {
    /** @param {{ key: string, source: string }} row */
    show(row) {
      visible.set(row.key, (visible.get(row.key) ?? 0) + 1);
      sources.set(row.key, row);
      if (loaded.has(row.key) || failed.has(row.key) || running.has(row.key) || timers.has(row.key) || queue.includes(row.key)) return;
      timers.set(row.key, setTimer(() => {
        timers.delete(row.key);
        if (!visible.has(row.key)) return;
        queue.push(row.key);
        pump();
      }, settleMs));
    },
    /** @param {string} key */
    hide(key) {
      const count = (visible.get(key) ?? 0) - 1;
      if (count > 0) {
        visible.set(key, count);
        return;
      }
      visible.delete(key);
      if (timers.has(key)) {
        clearTimer(timers.get(key));
        timers.delete(key);
      }
      const queued = queue.indexOf(key);
      if (queued >= 0) queue.splice(queued, 1);
    },
    /** The family name a loaded face was added under. @param {string} key */
    loaded(key) {
      return loaded.get(key);
    },
    /** Faces loading now (for tests). */
    get running() {
      return running.size;
    },
  };
}

let nextFace = 0;

/**
 * Load a row's face with the FontFace API and add it to the document under a name of its own.
 * @param {{ key: string, source: string }} row
 */
export async function loadFontRowFace(row) {
  nextFace += 1;
  const name = `MoeFontRow ${nextFace}`;
  const face = new FontFace(name, row.source);
  await face.load();
  document.fonts.add(face);
  return name;
}
