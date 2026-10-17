// Source-reading tests match Traditional Chinese literals such as `aria-label="播放"`. Since every UI string is wrapped as
// `tr('播放')`, this helper (import it first) makes fs reads of src/**/*.{svelte,js,ts} return the source with the wrapping
// removed again, so those tests keep checking what they always checked.
//
// The migration wrote plain attributes and text runs as `name={tr("x")}` / `{tr("x")}` (double quotes) and kept the
// original quotes for string literals inside expressions (`{tr('x')}` was `{'x'}`); templates became `tr("a{0}b", [e])`.
import fs from 'node:fs';
import fsPromises from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';

const SINGLE = String.raw`'((?:[^'\\]|\\.)*)'`;
const DOUBLE = String.raw`"((?:[^"\\]|\\.)*)"`;

/** Split `a, b(c, d), [e]` at top-level commas. @param {string} text */
function splitTopLevel(text) {
  const parts = [];
  let depth = 0;
  let quote = '';
  let start = 0;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (quote) {
      if (ch === '\\') i += 1;
      else if (ch === quote) quote = '';
    } else if (ch === '"' || ch === "'" || ch === '`') quote = ch;
    else if ('([{'.includes(ch)) depth += 1;
    else if (')]}'.includes(ch)) depth -= 1;
    else if (ch === ',' && depth === 0) {
      parts.push(text.slice(start, i).trim());
      start = i + 1;
    }
  }
  parts.push(text.slice(start).trim());
  return parts;
}

/** `tr("a{0}b", [x, y])` -> a template literal `a${x}b`. @param {string} source */
function unwrapTemplates(source) {
  let out = '';
  let cursor = 0;
  const open = /\btr\("((?:[^"\\]|\\.)*)", \[/g;
  for (let match = open.exec(source); match; match = open.exec(source)) {
    // find the matching `]` then `)`
    let depth = 1;
    let i = open.lastIndex;
    let quote = '';
    for (; i < source.length && depth > 0; i += 1) {
      const ch = source[i];
      if (quote) {
        if (ch === '\\') i += 1;
        else if (ch === quote) quote = '';
      } else if (ch === '"' || ch === "'" || ch === '`') quote = ch;
      else if (ch === '[') depth += 1;
      else if (ch === ']') depth -= 1;
    }
    const params = splitTopLevel(source.slice(open.lastIndex, i - 1));
    const template = match[1].replace(/\{(\d+)\}/g, (_, n) => `\${${params[Number(n)] ?? ''}}`);
    out += `${source.slice(cursor, match.index)}\`${template}\``;
    cursor = source[i] === ')' ? i + 1 : i;
    open.lastIndex = cursor;
  }
  return out + source.slice(cursor);
}

/** @param {string} source */
export function untr(source) {
  return unwrapTemplates(source)
    // name={tr("x")} came from name="x"
    .replace(new RegExp(String.raw`([\w:-]+)=\{tr\(${DOUBLE}\)\}`, 'g'), '$1="$2"')
    // {tr("x")} came from plain text x
    .replace(new RegExp(String.raw`\{tr\(${DOUBLE}\)\}`, 'g'), '$1')
    // everything else was a string literal: tr('x') -> 'x'
    .replace(new RegExp(String.raw`\btr\(('(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*")\)`, 'g'), '$1');
}

const wanted = (file) => /[\\/]src[\\/].*\.(svelte|js|ts)$/.test(String(file)) && !/i18n\.js$/.test(String(file));
const readSync = fs.readFileSync;
fs.readFileSync = function patched(file, ...rest) {
  const value = readSync.call(this, file, ...rest);
  return typeof value === 'string' && wanted(file instanceof URL ? file.pathname : file) ? untr(value) : value;
};
const read = fsPromises.readFile;
fsPromises.readFile = async function patched(file, ...rest) {
  const value = await read.call(this, file, ...rest);
  return typeof value === 'string' && wanted(file instanceof URL ? file.pathname : file) ? untr(value) : value;
};
syncBuiltinESMExports();
