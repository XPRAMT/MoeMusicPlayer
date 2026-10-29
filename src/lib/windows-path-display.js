/**
 * Windows paths for display only. The backend stores some source paths in the extended-length form (`\\?\D:\x`,
 * `\\?\UNC\server\share`), which keeps paths over 260 characters and names ending in a dot or space working, so the
 * stored value must stay as it is; only what the user reads drops the prefix.
 */

/** @param {unknown} path */
export function displayWindowsPath(path) {
  const text = typeof path === 'string' ? path : '';
  if (/^\\\\\?\\UNC\\/i.test(text)) return '\\\\' + text.slice(8);
  if (text.startsWith('\\\\?\\')) return text.slice(4);
  return text;
}
