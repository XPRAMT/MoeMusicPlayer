/** @typedef {import('./ipc').LyricLine} LyricLine */

import { LYRIC_MIN_ROW_HEIGHT_PX } from './lyrics-preferences.js';

export const TIMED_LYRIC_RADIUS = 6;
export const TIMED_LYRIC_ROW_HEIGHT = LYRIC_MIN_ROW_HEIGHT_PX;
export const PLAIN_LYRIC_ROW_HEIGHT = LYRIC_MIN_ROW_HEIGHT_PX;

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
 * @param {number} [rowHeight]
 * @returns {{ start: number, end: number, beforeHeight: number, afterHeight: number, rows: TimedLyric[] }}
 */
export function getTimedLyricWindow(timeline, activeIndex, rowHeight = TIMED_LYRIC_ROW_HEIGHT) {
  const safeRowHeight = normalizeRowHeight(rowHeight, TIMED_LYRIC_ROW_HEIGHT);
  if (timeline.length === 0) {
    return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0, rows: [] };
  }

  const center = activeIndex < 0 ? 0 : Math.min(activeIndex, timeline.length - 1);
  const start = Math.max(0, center - TIMED_LYRIC_RADIUS);
  const end = Math.min(timeline.length, center + TIMED_LYRIC_RADIUS + 1);

  return {
    start,
    end,
    beforeHeight: start * safeRowHeight,
    afterHeight: (timeline.length - end) * safeRowHeight,
    rows: timeline.slice(start, end),
  };
}

/**
 * Virtual window for plain, untimed lyrics. Rows are fixed-height and clamped
 * to two lines so the scroll spacer remains accurate on narrow screens.
 * @param {number} totalCount
 * @param {number} scrollTop
 * @param {number} viewportHeight
 * @param {number} [rowHeight]
 * @returns {{ start: number, end: number, beforeHeight: number, afterHeight: number }}
 */
export function getPlainLyricWindow(totalCount, scrollTop, viewportHeight, rowHeight = PLAIN_LYRIC_ROW_HEIGHT) {
  const count = Math.max(0, Math.floor(Number.isFinite(totalCount) ? totalCount : 0));
  if (count === 0) return { start: 0, end: 0, beforeHeight: 0, afterHeight: 0 };

  const safeRowHeight = normalizeRowHeight(rowHeight, PLAIN_LYRIC_ROW_HEIGHT);
  const safeScrollTop = Math.max(0, Number.isFinite(scrollTop) ? scrollTop : 0);
  const safeHeight = Math.max(safeRowHeight, Number.isFinite(viewportHeight) ? viewportHeight : safeRowHeight);
  const firstVisible = Math.min(count - 1, Math.floor(safeScrollTop / safeRowHeight));
  const visibleRows = Math.ceil(safeHeight / safeRowHeight);
  const start = Math.max(0, firstVisible - TIMED_LYRIC_RADIUS);
  const end = Math.min(count, firstVisible + visibleRows + TIMED_LYRIC_RADIUS);

  return {
    start,
    end,
    beforeHeight: start * safeRowHeight,
    afterHeight: (count - end) * safeRowHeight,
  };
}

/** @param {number} candidate @param {number} fallback */
function normalizeRowHeight(candidate, fallback) {
  return Number.isFinite(candidate) && candidate >= 1 ? Math.ceil(candidate) : fallback;
}
