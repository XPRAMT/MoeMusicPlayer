import test from 'node:test';
import assert from 'node:assert/strict';
import { formatDurationHours } from '../src/lib/format.ts';

test('formats album duration summaries as hh:mm:ss without truncating hours', () => {
  assert.equal(formatDurationHours(0), '00:00:00');
  assert.equal(formatDurationHours(3_661_000), '01:01:01');
  assert.equal(formatDurationHours(100 * 3_600_000 + 2_000), '100:00:02');
});
