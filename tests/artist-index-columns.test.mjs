import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const artistIndexSource = readFileSync(path.join(root, 'src/lib/ArtistIndex.svelte'), 'utf8').replace(/\r\n/g, '\n');

test('artist count and duration columns use six fullwidth character widths', () => {
  assert.match(artistIndexSource, /const ARTIST_GRID = '36px minmax\(150px,1fr\) 6em 6em 6em';/);
});
