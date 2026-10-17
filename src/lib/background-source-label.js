// @ts-check

/**
 * Human-readable label for a stored slideshow folder. Desktop paths are shown as-is; Android SAF
 * tree URIs (content://.../tree/primary%3APictures%2FWallpapers) become "內部儲存空間/Pictures/Wallpapers".
 * @param {string | null | undefined} value
 * @returns {string | null}
 */
export function formatSlideshowFolderLabel(value) {
  if (!value) return null;
  if (!value.startsWith('content://')) return value;
  const match = /\/tree\/([^/]+)/.exec(value);
  if (!match) return value;
  let documentId;
  try {
    documentId = decodeURIComponent(match[1]);
  } catch {
    return value;
  }
  const colon = documentId.indexOf(':');
  if (colon < 0) return documentId;
  const volume = documentId.slice(0, colon);
  const path = documentId.slice(colon + 1);
  const volumeLabel = volume === 'primary' ? '內部儲存空間' : volume;
  return path ? `${volumeLabel}/${path}` : volumeLabel;
}
