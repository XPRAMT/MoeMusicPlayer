// Interface-language catalogs (ADR 0019). Finds every `tr('原文')` / `tr("原文", [..])` in src/, then writes
// src/lib/i18n/zh-CN.json: the Simplified text of each source string, converted from Traditional with OpenCC
// (Taiwan phrases -> Mainland), then the Mainland software wording in zh-CN.terms.json, then any hand-written
// entry in zh-CN.overrides.json (which wins).
//
//   node scripts/i18n-catalog.mjs          write the catalogs
//   node scripts/i18n-catalog.mjs --check  exit 1 when a catalog is out of date (used by tests/i18n-catalog.test.mjs)
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as OpenCC from 'opencc-js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const srcDir = path.join(root, 'src');
const catalogDir = path.join(srcDir, 'lib', 'i18n');

const CALL = /\btr\(\s*('(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*")/g;

function listSources(dir, out = []) {
  for (const name of fs.readdirSync(dir)) {
    const file = path.join(dir, name);
    if (fs.statSync(file).isDirectory()) listSources(file, out);
    else if (/\.(svelte|js|ts)$/.test(name) && name !== 'i18n.js') out.push(file);
  }
  return out;
}

/** Every source string passed to tr(), sorted. */
export function extractKeys() {
  const keys = new Set();
  for (const file of listSources(srcDir)) {
    const text = fs.readFileSync(file, 'utf8');
    for (const match of text.matchAll(CALL)) {
      // The literal is plain JS string syntax, so evaluating it yields the key.
      keys.add(new Function(`return ${match[1]}`)());
    }
  }
  return [...keys].sort((a, b) => a.localeCompare(b, 'zh-Hant'));
}

function readJson(file) {
  return fs.existsSync(file) ? JSON.parse(fs.readFileSync(file, 'utf8').replace(/^﻿/, '')) : {};
}

/** @returns {Record<string, string>} */
export function buildSimplified(keys) {
  const convert = OpenCC.Converter({ from: 'twp', to: 'cn' });
  const overrides = readJson(path.join(catalogDir, 'zh-CN.overrides.json'));
  const terms = readJson(path.join(catalogDir, 'zh-CN.terms.json')).terms ?? [];
  const applyTerms = (text) => terms.reduce((value, [from, to]) => value.split(from).join(to), text);
  /** @type {Record<string, string>} */
  const catalog = {};
  for (const key of keys) {
    catalog[key] = Object.hasOwn(overrides, key) ? overrides[key] : applyTerms(convert(key));
  }
  return catalog;
}

function serialize(catalog) {
  return `${JSON.stringify(catalog, null, 2)}\n`;
}

/** Hand-written catalogs (Japanese, English) must cover every source string and keep its placeholders. */
export const HAND_WRITTEN_LANGUAGES = ['ja', 'en'];

/** @param {string} language @returns {{ missing: string[], extra: string[] }} */
export function translationGaps(language) {
  const keys = extractKeys();
  const catalog = readJson(path.join(catalogDir, `${language}.json`));
  return {
    missing: keys.filter((key) => !Object.hasOwn(catalog, key)),
    extra: Object.keys(catalog).filter((key) => !keys.includes(key)),
  };
}

export function catalogUpToDate() {
  const expected = serialize(buildSimplified(extractKeys()));
  const file = path.join(catalogDir, 'zh-CN.json');
  const actual = fs.existsSync(file) ? fs.readFileSync(file, 'utf8').replace(/^﻿/, '').replace(/\r\n/g, '\n') : '';
  return { upToDate: expected === actual, expected };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const { upToDate, expected } = catalogUpToDate();
  if (process.argv.includes('--missing')) {
    for (const language of HAND_WRITTEN_LANGUAGES) {
      const { missing, extra } = translationGaps(language);
      console.log(`${language}: ${missing.length} missing, ${extra.length} no longer used`);
      for (const key of missing) console.log(`  + ${JSON.stringify(key)}`);
      for (const key of extra) console.log(`  - ${JSON.stringify(key)}`);
    }
  } else if (process.argv.includes('--check')) {
    if (!upToDate) {
      console.error('src/lib/i18n/zh-CN.json is out of date: run `node scripts/i18n-catalog.mjs`.');
      process.exit(1);
    }
  } else {
    fs.writeFileSync(path.join(catalogDir, 'zh-CN.json'), expected, 'utf8');
    console.log(`zh-CN.json: ${Object.keys(JSON.parse(expected)).length} strings`);
  }
}
