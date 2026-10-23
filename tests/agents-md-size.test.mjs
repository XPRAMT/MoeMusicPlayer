// The root AGENTS.md is a current-state snapshot for agents, not a log.
// Rules (see 「本文件維護規則」 in AGENTS.md):
// - at most 50,000 characters (JS string length of the file without a BOM);
// - no date stamps: dates belong in the commit message or the ADR. A date-like
//   string is allowed only when it is a real fact that cannot be written
//   otherwise (a key name, a release tag); list such strings in ALLOWED_DATES.
//   Version numbers such as 26.10.19 and file-name stamps such as
//   backup-20261022-163948 do not match the pattern and are fine.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const MAX_CHARS = 50_000;
const MAX_DATE_STAMPS = 0;
const ALLOWED_DATES = new Set([]);
const DATE_PATTERN = /\b20\d\d-\d\d-\d\d\b|（20\d\d-\d\d|\b20\d\d-\d\d-/g;

const HOW_TO_FIX =
  'Rewrite the existing entry for that feature in place (delete what it supersedes, never append a new paragraph), ' +
  'move design details, measurements and dates to the matching ADR in docs/adr/, and keep only a short pointer ' +
  '(「見 ADR NNNN」) in AGENTS.md.';

const text = readFileSync(new URL('../AGENTS.md', import.meta.url), 'utf8').replace(/^﻿/, '');

test('AGENTS.md stays within the size limit', () => {
  assert.ok(
    text.length <= MAX_CHARS,
    `AGENTS.md has ${text.length} characters; the limit is ${MAX_CHARS}. ${HOW_TO_FIX}`,
  );
});

test('AGENTS.md has no date stamps', () => {
  const found = [...new Set(text.match(DATE_PATTERN) ?? [])].filter((d) => !ALLOWED_DATES.has(d));
  assert.ok(
    found.length <= MAX_DATE_STAMPS,
    `AGENTS.md contains date stamps (${found.join(', ')}); at most ${MAX_DATE_STAMPS} are allowed. ${HOW_TO_FIX}`,
  );
});
