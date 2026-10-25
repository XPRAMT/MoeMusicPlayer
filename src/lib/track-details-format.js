// Number formatting for the song details panel.

/** A number with at most `digits` decimals and no trailing zeros. @param {number} value @param {number} digits */
function trimmed(value, digits) {
  return String(Number(value.toFixed(digits)));
}

/**
 * The one bitrate rule for the whole app (details panel, Now Playing top bar, track list):
 * whole kbps below 1000 kbps; from 1000 kbps up, Mbps = kbps / 1000 with at most `mbpsDigits` decimals and no trailing zeros.
 * Nothing known shows a dash.
 * @param {number | null | undefined} kbps
 * @param {number} mbpsDigits
 */
export function formatBitrateKbps(kbps, mbpsDigits) {
  if (typeof kbps !== 'number' || !Number.isFinite(kbps) || kbps <= 0) return '—';
  return kbps >= 1000 ? `${trimmed(kbps / 1000, mbpsDigits)} Mbps` : `${trimmed(kbps, 0)} kbps`;
}

/** Details panel bitrate: the shared rule with up to three Mbps decimals. @param {number | null | undefined} kbps */
export function formatDetailsBitrate(kbps) {
  return formatBitrateKbps(kbps, 3);
}

/** Sample rate in kHz, or in MHz from 1 MHz up (DSD). Nothing known shows a dash. @param {number | null | undefined} hz */
export function formatDetailsSampleRate(hz) {
  if (typeof hz !== 'number' || !Number.isFinite(hz) || hz <= 0) return '—';
  return hz >= 1_000_000 ? `${trimmed(hz / 1_000_000, 4)} MHz` : `${trimmed(hz / 1000, 3)} kHz`;
}

/** @param {number} value */
const pad2 = (value) => String(value).padStart(2, '0');

/**
 * Local date and time to the second with the UTC offset, e.g. `2026-04-29 20:59:41 (UTC+8)`, `(UTC+5:30)`, `(UTC-3:30)`.
 * Nothing known shows a dash.
 * @param {number | null | undefined} ms Unix time in milliseconds.
 * @param {number} [offsetMinutes] Minutes east of UTC; defaults to this system's offset at that instant.
 */
export function formatDetailsDate(ms, offsetMinutes) {
  if (typeof ms !== 'number' || !Number.isFinite(ms)) return '—';
  const offset = offsetMinutes ?? -new Date(ms).getTimezoneOffset();
  const local = new Date(ms + offset * 60_000);
  const abs = Math.abs(offset);
  const zone = `UTC${offset >= 0 ? '+' : '-'}${Math.floor(abs / 60)}${abs % 60 ? `:${pad2(abs % 60)}` : ''}`;
  const day = `${local.getUTCFullYear()}-${pad2(local.getUTCMonth() + 1)}-${pad2(local.getUTCDate())}`;
  const time = `${pad2(local.getUTCHours())}:${pad2(local.getUTCMinutes())}:${pad2(local.getUTCSeconds())}`;
  return `${day} ${time} (${zone})`;
}

/** Length as m:ss.s (h:mm:ss.s from one hour), rounded to a tenth of a second. @param {number | null | undefined} ms */
export function formatDetailsDuration(ms) {
  if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) return '—';
  const tenths = Math.round(ms / 100);
  const wholeSeconds = Math.floor(tenths / 10);
  const tenth = tenths % 10;
  const seconds = String(wholeSeconds % 60).padStart(2, '0');
  const minutes = Math.floor(wholeSeconds / 60);
  if (minutes >= 60) return `${Math.floor(minutes / 60)}:${String(minutes % 60).padStart(2, '0')}:${seconds}.${tenth}`;
  return `${minutes}:${seconds}.${tenth}`;
}
