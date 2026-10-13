let rememberedScrollTop = 0;

export function getAlbumIndexScrollTop() {
  return rememberedScrollTop;
}

/** @param {number} value */
export function setAlbumIndexScrollTop(value) {
  rememberedScrollTop = Number.isFinite(value) ? Math.max(0, value) : 0;
}
