// The font pickers' rows (ADR 0042) are drawn in their own fonts. A row never uses the family's real file when the
// backend has made its name subset (a tiny file with only the glyphs of its shown name): the subset is registered
// through the FontFace API the first time a row needs it, synchronously, and the row uses it at once. Its ~5 KB load
// finishes within the face's block period (`display: 'block'`), so the row appears directly in its own font, never
// in the default font first. A family without a subset (legacy-encoding and symbol fonts, or a font installed since
// the last open whose subset is still being made) is drawn in the installed family itself.
import { quoteCssFamily } from './font-picker.js';

/**
 * The CSS `font-family` of a row: its subset face (named by `faceFor`) or, without a subset, the installed family,
 * then `fallback`.
 * @param {{ family: string, subset?: string } | null | undefined} font
 * @param {(subset: string) => string} faceFor
 * @param {string} fallback
 */
export function fontRowFamilyCss(font, faceFor, fallback) {
  if (!font) return fallback;
  const family = font.subset ? faceFor(font.subset) : font.family;
  return `${quoteCssFamily(family)}, ${fallback}`;
}

/**
 * The faces of the subsets: `faceFor(subset)` registers a subset file once (through `addFace`, with its URL from
 * `toUrl`) and returns the family name it was added under; later calls (any row, any picker, any list refresh) get
 * the same name.
 * @param {{ toUrl: (path: string) => string, addFace: (name: string, source: string) => void }} options
 */
export function createFontRowFaces({ toUrl, addFace }) {
  /** @type {Map<string, string>} */
  const faces = new Map();
  return {
    /** @param {string} subset */
    faceFor(subset) {
      let name = faces.get(subset);
      if (!name) {
        name = `MoeFontRow ${faces.size + 1}`;
        faces.set(subset, name);
        addFace(name, `url(${JSON.stringify(toUrl(subset))})`);
      }
      return name;
    },
    /** Subsets registered so far. */
    get size() {
      return faces.size;
    },
  };
}

/**
 * Add a subset face to the document and start loading it; the rows using it show nothing until it has loaded (a
 * local ~5 KB file), then their own font. A face the WebView rejects draws in the fallback.
 * @param {string} name
 * @param {string} source
 */
export function addFontRowFace(name, source) {
  if (typeof FontFace === 'undefined' || typeof document === 'undefined' || !document.fonts) return;
  try {
    const face = new FontFace(name, source, { display: 'block' });
    document.fonts.add(face);
    face.load().catch(() => {});
  } catch {
    // An unusable source: the row draws in the fallback.
  }
}
