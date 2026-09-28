async page => {
  const assert = {
    equal(actual, expected, message = 'values differ') {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    ok(value, message = 'expected a truthy value') {
      if (!value) throw new Error(message);
    },
  };
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto('http://127.0.0.1:4173/tests/lyrics-view-harness.html');
  await page.waitForFunction(() => window.lyricsViewHarness?.snapshot().phase === 'ready');

  const initial = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(initial.trackId, 'timed-track');
  assert.equal(initial.getCount, 1);
  assert.equal(initial.searchCount, 0, 'local lyrics must suppress remote lookup');
  assert.ok(initial.renderedTimedRows <= 13);

  await page.evaluate(() => window.lyricsViewHarness.setPosition(84_000));
  const soughtForward = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(soughtForward.activeIndex, 84);
  assert.ok(soughtForward.renderedTimedRows <= 13);

  await page.evaluate(() => window.lyricsViewHarness.setPlaying(false));
  await page.waitForTimeout(300);
  const paused = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(paused.activeIndex, 84, 'paused lyrics stay at the held playback position');
  assert.equal(paused.getCount, 1, 'playback position updates do not reload lyrics');
  assert.equal(paused.searchCount, 0, 'playback polling does not invoke remote lookup');

  await page.evaluate(() => window.lyricsViewHarness.setPosition(12_000));
  const soughtBackward = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(soughtBackward.activeIndex, 12, 'backward seek immediately updates the active lyric');
  assert.ok(soughtBackward.renderedTimedRows <= 13);

  await page.evaluate(() => window.lyricsViewHarness.setTrack('candidate-track'));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().phase === 'candidates');
  const candidateState = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(candidateState.getCount, 2);
  assert.equal(candidateState.searchCount, 1, 'remote lookup begins after local miss in Now Playing');
  assert.equal(candidateState.qrcNoticeVisible, true, 'QQ QRC without a preview is explained as undecoded');
  assert.equal(candidateState.candidateActionEnabled, true, 'QRC-only candidates remain selectable');
  await page.getByRole('button', { name: '使用這份' }).click();
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().selectedSource === 'manual');
  const selected = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(selected.phase, 'ready');
  assert.equal(selected.selectedSource, 'manual');

  await page.evaluate(() => window.lyricsViewHarness.setTrack('plain-track'));
  await page.waitForFunction(() => document.querySelector('[data-testid="plain-lyrics"]') !== null);
  await page.evaluate(() => window.lyricsViewHarness.scrollPlainTo(60));
  const plain = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.ok(plain.renderedPlainRows <= 20);
  assert.ok(plain.plainFirstIndex >= 54);
  assert.equal(plain.documentWidth, 360, 'narrow lyrics remain inside the viewport');

  return {
    result: 'PASS',
    timedSeekForward: soughtForward.activeIndex,
    pausedActiveIndex: paused.activeIndex,
    timedSeekBackward: soughtBackward.activeIndex,
    maxTimedRows: Math.max(initial.renderedTimedRows, soughtForward.renderedTimedRows, soughtBackward.renderedTimedRows),
    candidateRemoteSearches: candidateState.searchCount,
    manualSelection: selected.selectedSource,
    plainRows: plain.renderedPlainRows,
    plainFirstIndex: plain.plainFirstIndex,
    viewportWidth: plain.viewportWidth,
    documentWidth: plain.documentWidth,
  };
}
