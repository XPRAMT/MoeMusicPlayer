import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

import { isLosslessCodec } from '../src/lib/track-columns.js';

// tests/fixtures/codec-table.json is written by the player-formats codec table (cargo test -p player-formats codec).
const table = JSON.parse(readFileSync(new URL('./fixtures/codec-table.json', import.meta.url), 'utf8'));

test('the front end knows exactly the lossless codecs of the Rust codec table', () => {
  for (const { name, lossless } of table.codecs) {
    assert.equal(isLosslessCodec(name), lossless, `${name} lossless`);
    assert.equal(isLosslessCodec(name.toLowerCase()), lossless, `${name} lossless (lower case)`);
  }
  for (const rate of ['DSD64', 'DSD128', 'DSD256']) {
    assert.equal(isLosslessCodec(rate), true, rate);
  }
});
