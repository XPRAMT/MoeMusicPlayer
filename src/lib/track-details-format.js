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
