/** @typedef {import('./ipc').LyricLine} LyricLine */

export const TIMED_LYRIC_RADIUS = 6;
export const TIMED_LYRIC_ROW_HEIGHT = 76;
export const PLAIN_LYRIC_ROW_HEIGHT = 76;

/**
 * @typedef {{ index: number, startMs: number, line: LyricLine }} TimedLyric
 */

/** @param {LyricLine[] | null | undefined} lines @returns {TimedLyric[]} */
export function buildTimedLyricTimeline(lines) {
  if (!Array.isArray(lines)) return [];

  return lines
    .flatMap((line, index) => (
      typeof line.startMs === 'number' && Number.isFinite(line.startMs)
        ? [{ index, startMs: line.startMs, line }]
        : []
    ))
    .sort((left, right) => left.startMs - right.startMs || left.index - right.index);
}

/**
 * Return the last timed line at or before the playback position. A result of
 * -1 means playback is before the first lyric cue.
 * @param {TimedLyric[]} timeline
 * @param {number} positionMs
 * @param {number} [offsetMs]
 */
export function findActiveLyricIndex(timeline, positionMs, offsetMs = 0) {
  if (timeline.length === 0 || !Number.isFinite(positionMs)) return -1;
  const targetMs = positionMs + (Number.isFinite(offsetMs) ? offsetMs : 0);
  let low = 0;
  let high = timeline.length;

  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (timeline[middle].startMs <= targetMs) low = middle + 1;
    else high = middle;
  }

  return low - 1;
}

/**
 * @param {TimedLyric[]} timeline
 * @param {number} activeIndex
 * @returns {{ start: number, end: number, beforeHeight: number, afterHeight: number, rows: TimedLyric[] }}
 */
export function getTimedLyricWindow(timeline, activeIndex) {
  if (timeline.length === 0) {
    return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  }

  const center = activeIndex < 0 ? 0 : Math.min(activeIndex, timeline.length - 1);
  const start = Math.max(0, center - TIMED_LYRIC_RADIUS);
  const end = Math.min(timeline.length, center + TIMED_LYRIC_RADIUS + 1);

  return {
    start,
    end,
    beforeHeight: start * TIMED_LYRIC_ROW_HEIGHT,
    afterHeight: (timeline.length - end) * TIMED_LYRIC_ROW_HEIGHT,
    rows: timeline.slice(start, end),
  };
}

/**
 * Virtual window for plain, untimed lyrics. Rows are fixed-height and clamped
 * to two lines so the scroll spacer remains accurate on narrow screens.
 * @param {number} totalCount
 * @param {number} scrollTop
 * @param {number} viewportHeight
 * @returns {{ start: number, end: number, beforeHeight: number, afterHeight: number }}
 */
export function getPlainLyricWindow(totalCount, scrollTop, viewportHeight) {
  const count = Math.max(0, Math.floor(Number.isFinite(totalCount) ? totalCount : 0));
  if (count === 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0 };

  const safeScrollTop = Math.max(0, Number.isFinite(scrollTop) ? scrollTop : 0);
  const safeHeight = Math.max(PLAIN_LYRIC_ROW_HEIGHT, Number.isFinite(viewportHeight) ? viewportHeight : PLAIN_LYRIC_ROW_HEIGHT);
  const firstVisible = Math.min(count - 1, Math.floor(safeScrollTop / PLAIN_LYRIC_ROW_HEIGHT));
  const visibleRows = Math.ceil(safeHeight / PLAIN_LYRIC_ROW_HEIGHT);
  const start = Math.max(0, firstVisible - TIMED_LYRIC_RADIUS);
  const end = Math.min(count, firstVisible + visibleRows + TIMED_LYRIC_RADIUS);

  return {
    start,
    end,
    beforeHeight: start * PLAIN_LYRIC_ROW_HEIGHT,
    afterHeight: (count - end) * PLAIN_LYRIC_ROW_HEIGHT,
  };
}
