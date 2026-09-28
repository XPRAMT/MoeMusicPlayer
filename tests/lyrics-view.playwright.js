async page => {
  const assert = {
    equal(actual, expected, message = 'values differ') {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    ok(value, message = 'expected a truthy value') {
      if (!value) throw new Error(message);
    },
    deepEqual(actual, expected, message = 'values differ') {
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
    },
  };
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto('http://127.0.0.1:4173/tests/lyrics-view-harness.html');
  await page.waitForFunction(() => window.lyricsViewHarness?.snapshot().phase === 'ready');

  function assertGeometry(state, label) {
    assert.ok(state.actualRowHeights.length > 0, `${label}: rows should be rendered`);
    assert.ok(state.actualRowHeights.every((height) => Math.abs(height - state.rowHeight) < 0.6), `${label}: each DOM row must use the declared row height`);
    if (state.timedWindowStart !== null && state.timedWindowEnd !== null) {
      assert.ok(Math.abs(state.beforeSpacerHeight - state.timedWindowStart * state.rowHeight) < 0.6, `${label}: timed leading spacer must align`);
      assert.ok(Math.abs(state.afterSpacerHeight - (120 - state.timedWindowEnd) * state.rowHeight) < 0.6, `${label}: timed trailing spacer must align`);
    } else {
      const firstIndex = state.plainFirstIndex;
      const endIndex = firstIndex + state.renderedPlainRows;
      assert.ok(Math.abs(state.beforeSpacerHeight - firstIndex * state.rowHeight) < 0.6, `${label}: plain leading spacer must align`);
      assert.ok(Math.abs(state.afterSpacerHeight - (120 - endIndex) * state.rowHeight) < 0.6, `${label}: plain trailing spacer must align`);
    }
  }

  const initial = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(initial.trackId, 'timed-track');
  assert.equal(initial.getCount, 1);
  assert.equal(initial.searchCount, 0, 'local lyrics must suppress remote lookup');
  assert.ok(initial.renderedTimedRows <= 13);
  assert.equal(initial.rowHeight, 76, 'default settings retain the base row height');
  assertGeometry(initial, 'default timed');
  assert.equal(initial.toolbarAvailable, true, 'lyrics toggles remain enabled');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true }));
  const translationOnly = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(translationOnly.rowHeight, 76);
  assert.equal(translationOnly.firstRowTranslationCount, 1);
  assert.equal(translationOnly.firstRowRomanizationCount, 0);
  assertGeometry(translationOnly, 'translation only');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: false, showRomanization: true }));
  const romanizationOnly = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(romanizationOnly.rowHeight, 76);
  assert.equal(romanizationOnly.firstRowTranslationCount, 0);
  assert.equal(romanizationOnly.firstRowRomanizationCount, 1);
  assertGeometry(romanizationOnly, 'romanization only');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: true }));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().rowHeight === 82);
  const bothAuxiliary = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assertGeometry(bothAuxiliary, 'default font with both auxiliary lines');
  assert.ok(bothAuxiliary.activeRowContentHeight <= bothAuxiliary.rowHeight + 0.6, 'default text and both auxiliary lines fit without clipping');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ primaryFontSizePx: 36, auxiliaryFontSizePx: 24, inactiveOpacityPercent: 35 }));
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: false }));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().rowHeight === 145);
  const maximumTranslationOnly = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(maximumTranslationOnly.firstRowTranslationCount, 1);
  assert.equal(maximumTranslationOnly.firstRowRomanizationCount, 0);
  assert.ok(maximumTranslationOnly.activeRowContentHeight <= maximumTranslationOnly.rowHeight + 0.6);
  assertGeometry(maximumTranslationOnly, 'maximum translation only');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: false, showRomanization: true }));
  const maximumRomanizationOnly = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(maximumRomanizationOnly.rowHeight, 145);
  assert.equal(maximumRomanizationOnly.firstRowTranslationCount, 0);
  assert.equal(maximumRomanizationOnly.firstRowRomanizationCount, 1);
  assert.ok(maximumRomanizationOnly.activeRowContentHeight <= maximumRomanizationOnly.rowHeight + 0.6);
  assertGeometry(maximumRomanizationOnly, 'maximum romanization only');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: true }));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().rowHeight === 177);
  await page.evaluate(() => window.lyricsViewHarness.setPosition(90_000));
  await page.waitForTimeout(40);
  const maximum = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(maximum.activeIndex, 90);
  assert.equal(maximum.rowHeight, 177, 'maximum fonts and both auxiliary lines use the conservative row geometry');
  assert.ok(maximum.actualRowHeights.length <= 13);
  assertGeometry(maximum, 'maximum timed');
  assert.ok(maximum.activeRowContentHeight <= maximum.rowHeight + 0.6, 'two primary lines and both auxiliary lines fit at maximum sizes');
  assert.ok(maximum.bothAuxiliaryRowContentHeight <= maximum.rowHeight + 0.6, 'DOM row with both auxiliary lines stays inside the maximum row box');
  const expectedScrollTop = Math.max(0, 90 * maximum.rowHeight - (maximum.timedViewportHeight - maximum.rowHeight) / 2);
  assert.ok(Math.abs(maximum.timedScrollTop - expectedScrollTop) < 1.5, 'auto-scroll uses the same row-height geometry');
  assert.equal(maximum.activeRowOpacity, 1, 'row background and border stay fully opaque');
  assert.equal(maximum.activeRowTextOpacity, 1);
  assert.equal(maximum.firstRowOpacity, 1, 'inactive dimming must not fade the row background or border');
  assert.equal(maximum.firstRowTextOpacity, 0.35, 'only inactive lyric text uses the configured opacity');

  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('paused'));
  const pausedGeometry = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(pausedGeometry.activeIndex, 90, 'paused playback keeps the active cue');

  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('stopped'));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().firstRowTextOpacity === 1);
  const stopped = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(stopped.activeIndex, null, 'stopped playback has no focused lyric row');
  assert.equal(stopped.timedWindowStart, 84, 'stopped playback keeps the virtual window around the held middle cue');
  assert.equal(stopped.timedScrollTop, maximum.timedScrollTop, 'stopping keeps the held lyric position visible');
  assert.equal(stopped.firstRowTextOpacity, 1, 'without an active row all timed lines remain fully readable');
  assert.equal(stopped.toolbarAvailable, true);

  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('playing'));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(-1_000));
  await page.waitForFunction(() => {
    const state = window.lyricsViewHarness.snapshot();
    return state.firstRowTextOpacity === 1 && state.timedScrollTop === 0;
  });
  const beforeFirstCue = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(beforeFirstCue.activeIndex, null, 'before the first cue there is no active line');
  assert.equal(beforeFirstCue.timedWindowStart, 0);
  assert.equal(beforeFirstCue.timedScrollTop, 0, 'no-cue fallback aligns the top window and scroll position');
  assert.equal(beforeFirstCue.firstRowTextOpacity, 1, 'cue-less timed playback remains fully readable');

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
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({
    showTranslation: false,
    showRomanization: false,
    primaryFontSizePx: 14,
    auxiliaryFontSizePx: 10,
  }));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().rowHeight === 76);
  const plainDefault = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assertGeometry(plainDefault, 'default plain');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({
    showTranslation: true,
    showRomanization: true,
    primaryFontSizePx: 36,
    auxiliaryFontSizePx: 24,
  }));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().rowHeight === 177);
  await page.evaluate(() => window.lyricsViewHarness.scrollPlainTo(60));
  const plain = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.ok(plain.renderedPlainRows <= 20);
  assert.ok(plain.plainFirstIndex >= 54);
  assert.equal(plain.documentWidth, 360, 'narrow lyrics remain inside the viewport');
  assertGeometry(plain, 'maximum plain');
  assert.ok(plain.bothAuxiliaryRowContentHeight <= plain.rowHeight + 0.6, 'plain row with both auxiliary lines fits at maximum sizes');
  assert.equal(plain.firstRowTextOpacity, 1, 'plain lyrics are never dimmed without a timed active row');
  assert.equal(plain.toolbarAvailable, true);

  await page.getByRole('button', { name: '切換譯文顯示' }).click();
  const toggled = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(toggled.preferences.showTranslation, false, 'viewport toolbar changes the shared preference');

  await page.evaluate(() => window.lyricsViewHarness.setTrack(null));
  const noTrack = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(noTrack.toolbarAvailable, true, 'toolbar remains usable without a current track');
  assert.equal(noTrack.preferences.showTranslation, false, 'track changes preserve global preferences');

  await page.goto('http://127.0.0.1:4173/');
  await page.locator('.nav-link').filter({ hasText: '設定' }).click();
  await page.getByRole('tab', { name: '歌詞' }).click();
  const settingsControls = await page.evaluate(() => ({
    translationDefault: document.querySelectorAll('.lyrics-preference-toggle input')[0]?.checked,
    romanizationDefault: document.querySelectorAll('.lyrics-preference-toggle input')[1]?.checked,
    opacity: document.querySelectorAll('.lyrics-preference-range input')[0]?.value,
    primary: document.querySelectorAll('.lyrics-preference-range input')[1]?.value,
    auxiliary: document.querySelectorAll('.lyrics-preference-range input')[2]?.value,
    opacityBounds: [document.querySelectorAll('.lyrics-preference-range input')[0]?.min, document.querySelectorAll('.lyrics-preference-range input')[0]?.max],
    primaryBounds: [document.querySelectorAll('.lyrics-preference-range input')[1]?.min, document.querySelectorAll('.lyrics-preference-range input')[1]?.max],
    auxiliaryBounds: [document.querySelectorAll('.lyrics-preference-range input')[2]?.min, document.querySelectorAll('.lyrics-preference-range input')[2]?.max],
    activeTab: document.querySelector('#lyrics-tab')?.getAttribute('aria-selected'),
  }));
  assert.equal(settingsControls.translationDefault, false);
  assert.equal(settingsControls.romanizationDefault, false);
  assert.equal(settingsControls.opacity, '70');
  assert.equal(settingsControls.primary, '14');
  assert.equal(settingsControls.auxiliary, '10');
  assert.deepEqual(settingsControls.opacityBounds, ['10', '100']);
  assert.deepEqual(settingsControls.primaryBounds, ['12', '36']);
  assert.deepEqual(settingsControls.auxiliaryBounds, ['9', '24']);
  assert.equal(settingsControls.activeTab, 'true', 'lyrics preferences are accessible from Settings');

  await page.locator('input[aria-label="原文字級"]').evaluate((input) => {
    input.value = '36';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  const changedSettings = await page.locator('input[aria-label="原文字級"]').inputValue();
  assert.equal(changedSettings, '36', 'settings controls update the stored preference draft');

  return {
    result: 'PASS',
    timedSeekForward: soughtForward.activeIndex,
    pausedActiveIndex: paused.activeIndex,
    timedSeekBackward: soughtBackward.activeIndex,
    maxTimedRows: Math.max(initial.renderedTimedRows, soughtForward.renderedTimedRows, soughtBackward.renderedTimedRows),
    defaultRowHeight: initial.rowHeight,
    bothAuxiliaryDefaultRowHeight: bothAuxiliary.rowHeight,
    maximumSingleAuxiliaryRowHeight: maximumTranslationOnly.rowHeight,
    maximumRowHeight: maximum.rowHeight,
    maximumActiveContentHeight: maximum.activeRowContentHeight,
    maximumScrollTop: maximum.timedScrollTop,
    stoppedTextOpacity: stopped.firstRowTextOpacity,
    plainTextOpacity: plain.firstRowTextOpacity,
    candidateRemoteSearches: candidateState.searchCount,
    manualSelection: selected.selectedSource,
    plainRows: plain.renderedPlainRows,
    plainFirstIndex: plain.plainFirstIndex,
    viewportWidth: plain.viewportWidth,
    documentWidth: plain.documentWidth,
    lyricsSettingsDefaults: settingsControls,
  };
}
