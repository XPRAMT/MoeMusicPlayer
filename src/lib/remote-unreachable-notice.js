/** Texts of an unreachable server (ADR 0026, 0036): the notice rule matches Rust `REMOTE_LINK_LOST` and `REMOTE_UNREACHABLE`. */
/** Two reports of the same failed attempt (the command's error and the snapshot's `lastError`) are one notice. */
export const UNREACHABLE_NOTICE_DEDUPE_MS = 2000;

/** How long the notice stays on screen. */
export const UNREACHABLE_NOTICE_VISIBLE_MS = 4000;

/**
 * @param {unknown} message
 * @returns {boolean}
 */
export function isRemoteUnreachableMessage(message) {
  return typeof message === 'string'
    && (message.includes('伺服器無法連線') || message.includes('無法連線到伺服器'));
}

/**
 * Whether a snapshot's `lastError` is a new unreachable-server failure (the error text changed to one).
 * @param {string | null | undefined} previousError
 * @param {string | null | undefined} nextError
 * @returns {boolean}
 */
export function snapshotReportsNewUnreachable(previousError, nextError) {
  return isRemoteUnreachableMessage(nextError) && previousError !== nextError;
}

/**
 * Decide whether a failure shows the notice now. `lastShownMs` is when the notice was last shown (or null).
 * @param {number | null} lastShownMs
 * @param {number} nowMs
 * @returns {boolean}
 */
export function shouldShowUnreachableNotice(lastShownMs, nowMs) {
  return lastShownMs === null || nowMs - lastShownMs >= UNREACHABLE_NOTICE_DEDUPE_MS;
}
