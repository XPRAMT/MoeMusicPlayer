/**
 * Sleep timer (睡眠定時) display and adjustment math. The Rust core owns the deadline
 * (`sleep_timer_set` / `sleep_timer_get`); the UI derives the remaining time from the
 * wall-clock end timestamp, so a throttled WebView only delays the label, never expiry.
 */

/** 23:59: hours 0..23 and minutes 0..59 are adjusted independently. */
export const SLEEP_TIMER_MAX_HOURS = 23;
export const SLEEP_TIMER_MAX_MINUTE_FIELD = 59;
export const SLEEP_TIMER_MAX_MINUTES = SLEEP_TIMER_MAX_HOURS * 60 + SLEEP_TIMER_MAX_MINUTE_FIELD;
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
 * Change one field by whole notches (1 hour or 1 minute each). Fields are independent and clamp
 * at their ends: minutes never wrap or carry into hours (00:59 +1 min stays 00:59), hours stay 0..23.
 * The result is total minutes; 0 means "cancel".
 * @param {number} currentMinutes @param {'hours' | 'minutes'} field @param {number} steps positive = longer
 */
export function adjustSleepTimerField(currentMinutes, field, steps) {
  const current = Number.isFinite(currentMinutes) ? Math.max(0, Math.min(SLEEP_TIMER_MAX_MINUTES, Math.round(currentMinutes))) : 0;
  const whole = Number.isFinite(steps) ? Math.trunc(steps) : 0;
  let hours = Math.floor(current / 60);
  let minutes = current % 60;
  if (field === 'hours') hours = Math.max(0, Math.min(SLEEP_TIMER_MAX_HOURS, hours + whole));
  else minutes = Math.max(0, Math.min(SLEEP_TIMER_MAX_MINUTE_FIELD, minutes + whole));
  return hours * 60 + minutes;
}

/** @param {number} minutes */
export function splitSleepTimer(minutes) {
  const [hours, mins] = formatSleepTimer(minutes).split(':');
  return { hours, minutes: mins };
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
