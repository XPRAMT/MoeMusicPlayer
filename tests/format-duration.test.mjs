import test from 'node:test';
import assert from 'node:assert/strict';
import { formatAlbumDetailSummary, formatDurationHours } from '../src/lib/format.ts';

test('formats album duration summaries as hh:mm:ss without truncating hours', () => {
  assert.equal(formatDurationHours(0), '00:00:00');
  assert.equal(formatDurationHours(3_661_000), '01:01:01');
  assert.equal(formatDurationHours(100 * 3_600_000 + 2_000), '100:00:02');
});

test('formats the album detail summary with track count, cumulative playback, and track duration', () => {
  assert.equal(
    formatAlbumDetailSummary(3, 3_661_000, 7_325_000),
    '3首曲目．累積播放 01:01:01．播放時長 02:02:05',
  );
});
