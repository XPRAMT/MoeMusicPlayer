/**
 * Sleep timer (睡眠定時) display and adjustment math. The Rust core owns the deadline
 * (`sleep_timer_set` / `sleep_timer_get`); the UI derives the remaining time from the
 * wall-clock end timestamp, so a throttled WebView only delays the label, never expiry.
 */

/** Minutes per wheel notch / drag step / arrow key. */
export const SLEEP_TIMER_STEP_MINUTES = 5;
/** 23:55 — the largest multiple of 5 minutes below 24 hours. */
export const SLEEP_TIMER_MAX_MINUTES = 23 * 60 + 55;
/** Vertical drag distance (CSS px) per step; touch and mouse share it. */
export const SLEEP_TIMER_DRAG_STEP_PX = 14;
/** Pixel-mode wheel distance per step (one mouse notch is ~100 px, trackpads send small deltas). */
export const SLEEP_TIMER_WHEEL_STEP_PX = 50;

/** @param {number | null | undefined} endsAtMs @param {number} nowMs */
export function getSleepTimerRemainingMs(endsAtMs, nowMs) {
  if (typeof endsAtMs !== 'number' || !Number.isFinite(endsAtMs) || !Number.isFinite(nowMs)) return 0;
  return Math.max(0, endsAtMs - nowMs);
}

/** Whole minutes shown for a remaining time, rounded up so 00:00 appears only at expiry. */
/** @param {number} remainingMs */
export function getSleepTimerDisplayMinutes(remainingMs) {
  if (!Number.isFinite(remainingMs) || remainingMs <= 0) return 0;
  return Math.min(SLEEP_TIMER_MAX_MINUTES, Math.ceil(remainingMs / 60_000));
}

/** @param {number} minutes */
export function formatSleepTimer(minutes) {
  const safe = Number.isFinite(minutes) ? Math.max(0, Math.min(SLEEP_TIMER_MAX_MINUTES, Math.floor(minutes))) : 0;
  const hours = Math.floor(safe / 60);
  return `${String(hours).padStart(2, '0')}:${String(safe % 60).padStart(2, '0')}`;
}

/**
 * Move by whole steps, snapping to the 5-minute grid first: from 00:13, +1 → 00:15 and −1 → 00:10.
 * Result is clamped to 0..23:55; 0 means "cancel".
 * @param {number} currentMinutes @param {number} steps positive = longer
 */
export function adjustSleepTimerMinutes(currentMinutes, steps) {
  const step = SLEEP_TIMER_STEP_MINUTES;
  const current = Number.isFinite(currentMinutes) ? Math.max(0, Math.min(SLEEP_TIMER_MAX_MINUTES, Math.round(currentMinutes))) : 0;
  const whole = Number.isFinite(steps) ? Math.trunc(steps) : 0;
  if (whole === 0) return current;
  const base = whole > 0 ? Math.floor(current / step) * step : Math.ceil(current / step) * step;
  return Math.max(0, Math.min(SLEEP_TIMER_MAX_MINUTES, base + whole * step));
}

/**
 * Accumulate a continuous delta into whole steps. Returns the steps and the leftover delta.
 * Negative deltas (wheel up / drag up) lengthen the timer.
 * @param {number} carry @param {number} delta @param {number} stepPx
 */
export function accumulateSleepTimerSteps(carry, delta, stepPx) {
  const size = Number.isFinite(stepPx) && stepPx > 0 ? stepPx : 1;
  const total = (Number.isFinite(carry) ? carry : 0) + (Number.isFinite(delta) ? delta : 0);
  const raw = Math.trunc(total / size);
  return { steps: raw === 0 ? 0 : -raw, carry: total - raw * size };
}
