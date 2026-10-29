// Guard: where the app may store things (see the hard rule about settings storage in AGENTS.md).
// Scans the frontend (localStorage / sessionStorage / indexedDB and their keys) and Android Kotlin/Java
// (SharedPreferences names, DataStore). Rust is out of scope. Allowlist: tests/fixtures/storage-locations.json.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join, relative, extname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const CATEGORIES = new Set(['debug-flag', 'cache', 'device-specific', 'pending-migration', 'framework']);
const SKIP_DIRS = new Set(['build', '.tauri', 'tauri-api', 'node_modules', '.gradle', '.git', 'target']);

const HOW_TO_FIX =
  'HOW TO FIX (read before touching this test): user settings (anything the user can see or change, including panel sizes, ' +
  'selected tabs, option switches) must be stored ONLY in settings.json through the Rust SettingsStore (src-tauri/src/settings.rs, ' +
  'one shared Windows/Android implementation, included in backup export). Data tied to a song (e.g. per-track lyric delay) belongs in the ' +
  'library database. localStorage / sessionStorage / IndexedDB / Android SharedPreferences / DataStore are allowed only for debug flags, ' +
  'pure caches, device-specific state and framework needs. Only then add an entry to tests/fixtures/storage-locations.json with ' +
  '`where`, `key`, `category` (debug-flag | cache | device-specific | framework) and a one-line `reason`. NEVER add a user-visible setting ' +
  'to the allowlist as a permanent exception (`pending-migration` is only for the already known entries awaiting migration).';

function walk(dir, exts, out = []) {
  for (const name of readdirSync(dir)) {
    if (SKIP_DIRS.has(name)) continue;
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) walk(full, exts, out);
    else if (exts.has(extname(name))) out.push(full);
  }
  return out;
}
const rel = (f) => relative(root, f).split('\\').join('/');
// Read as a buffer (some sources contain NUL bytes and must not be skipped).
const read = (f) => readFileSync(f).toString('utf8');
function stripComments(text) {
  return text.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:'"`\\])\/\/.*$/gm, '$1');
}
const unquote = (expr) => {
  const m = /^\s*(['"`])((?:(?!\1)[^\\$]|\\.)*)/.exec(expr);
  return m ? m[2] : null;
};
const firstIdent = (expr) => /^\s*([A-Za-z_$][\w$]*)\s*(?:$|\+)/.exec(expr)?.[1] ?? null;

// ---- frontend ----
const jsFiles = walk(join(root, 'src'), new Set(['.js', '.ts', '.svelte'])).map((f) => ({
  file: rel(f),
  code: stripComments(read(f)),
}));
// String constants: the file's own first, then exported ones from any file (a key constant may be imported).
const CONST_RE = /\b(export\s+)?const\s+([A-Za-z_$][\w$]*)\s*(?::\s*string\s*)?=\s*(['"])([^'"\n]*)\3/g;
const exportedConsts = new Map();
const localConsts = new Map();
for (const { file, code } of jsFiles) {
  const local = new Map();
  for (const m of code.matchAll(CONST_RE)) {
    local.set(m[2], m[4]);
    if (m[1]) exportedConsts.set(m[2], m[4]);
  }
  localConsts.set(file, local);
}
const API = /\b(localStorage|sessionStorage|indexedDB)\b/;
const KEY_SHAPE = /['"`]((?:moe[._]|moemusicplayer\.)[\w.:-]*)['"`]/g;

/** file -> Set of keys / names in use */
const used = new Map();
const problems = [];
for (const { file, code } of jsFiles) {
  if (!API.test(code)) continue;
  const keys = new Set();
  if (/\bindexedDB\b/.test(code)) keys.add('indexedDB');
  for (const m of code.matchAll(KEY_SHAPE)) keys.add(m[1]);
  for (const m of code.matchAll(/\.(?:getItem|setItem|removeItem)\(\s*([^,)]+)/g)) {
    const lit = unquote(m[1]);
    if (lit !== null) {
      keys.add(lit);
      continue;
    }
    const id = firstIdent(m[1]);
    const value = id ? (localConsts.get(file).get(id) ?? exportedConsts.get(id)) : undefined;
    if (value !== undefined) keys.add(value);
  }
  if (keys.size === 0) {
    problems.push(`${file} uses browser storage but no storage key could be determined; use a string literal or a top-level string constant for the key.`);
  }
  used.set(file, keys);
}

// ---- Android ----
const androidRoots = [
  join(root, 'plugins/tauri-plugin-media-index/android/src'),
  join(root, 'src-tauri/gen/android/app/src'),
];
for (const dir of androidRoots) {
  let files = [];
  try {
    files = walk(dir, new Set(['.kt', '.java']));
  } catch {
    /* directory absent */
  }
  for (const f of files) {
    const code = stripComments(read(f));
    const names = new Set();
    for (const m of code.matchAll(/getSharedPreferences\(\s*([^,)]+)/g)) {
      const lit = unquote(m[1]);
      if (lit !== null) {
        names.add(lit);
        continue;
      }
      const id = firstIdent(m[1]);
      const c = id && new RegExp(`\\bval\\s+${id}\\s*(?::\\s*String)?\\s*=\\s*"([^"]*)"`).exec(code);
      if (c) names.add(c[1]);
      else problems.push(`${rel(f)}: getSharedPreferences(${m[1].trim()}) name could not be resolved; use a string literal or a const val in the same file.`);
    }
    if (/\bgetDefaultSharedPreferences\b|\bPreferenceManager\b/.test(code)) names.add('DefaultSharedPreferences');
    if (/\bDataStore\b|\bpreferencesDataStore\b|\bdataStore\b/.test(code)) names.add('DataStore');
    if (names.size) used.set(rel(f), names);
  }
}

// ---- allowlist ----
const allow = JSON.parse(readFileSync(new URL('./fixtures/storage-locations.json', import.meta.url), 'utf8')).entries;
const allowed = new Set(allow.map((e) => `${e.where}\u0000${e.key}`));

test('allowlist entries are well formed', () => {
  for (const e of allow) {
    assert.ok(e.where && e.key, `allowlist entry needs where and key: ${JSON.stringify(e)}`);
    assert.ok(CATEGORIES.has(e.category), `bad category in ${JSON.stringify(e)}; allowed: ${[...CATEGORIES].join(', ')}`);
    assert.ok(typeof e.reason === 'string' && e.reason.trim().length > 0, `allowlist entry needs a one-line reason: ${JSON.stringify(e)}`);
  }
  assert.equal(allowed.size, allow.length, 'duplicate allowlist entries (same where + key)');
});

test('every storage use (browser storage, SharedPreferences, DataStore) is listed in the allowlist', () => {
  assert.deepEqual(problems, [], `${problems.join('\n')}\n${HOW_TO_FIX}`);
  const missing = [];
  for (const [file, keys] of used) {
    for (const key of keys) if (!allowed.has(`${file}\u0000${key}`)) missing.push(`${file}  ->  ${key}`);
  }
  assert.deepEqual(
    missing,
    [],
    `Storage not covered by tests/fixtures/storage-locations.json:\n  ${missing.join('\n  ')}\n${HOW_TO_FIX}`,
  );
});

test('allowlist has no stale entries', () => {
  const stale = allow.filter((e) => !used.get(e.where)?.has(e.key)).map((e) => `${e.where}  ->  ${e.key}`);
  assert.deepEqual(
    stale,
    [],
    `Allowlist entries whose storage use no longer exists in the code (delete them from tests/fixtures/storage-locations.json; ` +
      `a finished migration should remove its entry):\n  ${stale.join('\n  ')}`,
  );
});
