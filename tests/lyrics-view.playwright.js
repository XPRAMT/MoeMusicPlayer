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
  const baseUrl = page.url().split('/').slice(0, 3).join('/');
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto(`${baseUrl}/tests/lyrics-view-harness.html`);
  await page.waitForFunction(() => window.lyricsViewHarness?.snapshot().phase === 'ready');

  function assertGeometry(state, label) {
    assert.ok(state.actualRowHeights.length > 0, `${label}: rows should be rendered`);
    assert.equal(state.actualRowHeights.length, state.expectedRowHeights.length);
    assert.ok(state.actualRowHeights.every((height, index) => Math.abs(height - state.expectedRowHeights[index]) < 0.6), `${label}: DOM row heights must match their variable geometry`);
    const renderedHeight = state.actualRowHeights.reduce((sum, height, index) => sum + height + state.rowMargins[index], 0);
    assert.ok(Math.abs(state.beforeSpacerHeight + renderedHeight + state.afterSpacerHeight - state.totalHeight) < 1.2, `${label}: row geometry and spacers must share prefix offsets`);
  }

  const initial = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(initial.trackId, 'timed-track');
  assert.equal(initial.getCount, 1);
  assert.equal(initial.searchCount, 0, 'local lyrics must suppress remote lookup');
  assert.ok(initial.renderedTimedRows <= 13);
  assertGeometry(initial, 'default timed');
  assert.equal(initial.toolbarAvailable, true, 'lyrics toggles remain enabled');
  assert.equal(initial.lineGapPx, 24);

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: true }));
  const mixed = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assertGeometry(mixed, 'mixed timed rows');
  const missingAuxCue = await page.evaluate(async () => {
    await window.lyricsViewHarness.setPosition(4_000);
    const root = document.querySelector('[data-testid="lyrics-view"]');
    const row = document.querySelector('[data-testid="timed-lyrics"] [aria-current="true"]');
    return {
      cueIndex: Number(root?.dataset.activeCueIndex),
      translationCount: row?.querySelectorAll('.lyric-translation').length ?? -1,
      romanizationCount: row?.querySelectorAll('.lyric-romanization').length ?? -1,
      height: row?.getBoundingClientRect().height ?? 0,
    };
  });
  assert.equal(missingAuxCue.cueIndex, 4);
  assert.equal(missingAuxCue.translationCount, 0, 'blank translations do not render');
  assert.equal(missingAuxCue.romanizationCount, 0, 'blank romanization does not render');
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: false, showRomanization: false }));
  const hiddenAuxCue = await page.evaluate(() => document.querySelector('[data-testid="timed-lyrics"] [aria-current="true"]')?.getBoundingClientRect().height ?? 0);
  assert.ok(Math.abs(missingAuxCue.height - hiddenAuxCue) < 0.6, 'enabled but missing auxiliary text does not add row height');
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: true }));
  const presentAuxCue = await page.evaluate(async () => {
    await window.lyricsViewHarness.setPosition(3_000);
    const row = document.querySelector('[data-testid="timed-lyrics"] [aria-current="true"]');
    return {
      cueIndex: Number(document.querySelector('[data-testid="lyrics-view"]')?.dataset.activeCueIndex),
      translationCount: row?.querySelectorAll('.lyric-translation').length ?? -1,
      height: row?.getBoundingClientRect().height ?? 0,
    };
  });
  assert.equal(presentAuxCue.cueIndex, 3);
  assert.equal(presentAuxCue.translationCount, 1, 'a real translation renders');
  assert.ok(presentAuxCue.height > missingAuxCue.height, 'present auxiliary text contributes to row height');
  await page.evaluate(() => window.lyricsViewHarness.setPosition(1_000));
  await page.waitForTimeout(180);
  const emptyCue = await page.evaluate(() => ({
    cueIndex: Number(document.querySelector('[data-testid="lyrics-view"]')?.dataset.activeCueIndex),
    currentRowCount: document.querySelectorAll('[data-testid="timed-lyrics"] [aria-current="true"]').length,
    allTextOpaque: [...document.querySelectorAll('[data-testid="timed-lyrics"] .lyric-primary')]
      .every((text) => getComputedStyle(text).opacity === '1'),
  }));
  assert.equal(emptyCue.cueIndex, 1, 'the timeline retains an empty-content cue');
  assert.equal(emptyCue.currentRowCount, 0, 'an empty-content cue is not rendered or transferred to the next line');
  assert.equal(emptyCue.allTextOpaque, true, 'an empty-content cue does not dim every visible row');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ primaryFontSizePx: 36, auxiliaryFontSizePx: 24, lineGapPx: 64 }));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(90_000));
  await page.waitForTimeout(40);
  const maximum = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(maximum.activeIndex, 90);
  assert.ok(maximum.actualRowHeights.length <= 13);
  assertGeometry(maximum, 'maximum timed');
  assert.ok(maximum.activeRowContentHeight <= maximum.activeRowBoxHeight + 0.6, 'clamped primary and present auxiliaries fit in the row');
  const activeVisuals = await page.evaluate(() => {
    const viewport = document.querySelector('[data-testid="timed-lyrics"]');
    const active = viewport?.querySelector('[aria-current="true"]');
    const text = active?.querySelector('.lyric-primary');
    return {
      activeBackground: active ? getComputedStyle(active).backgroundColor : null,
      activeTextOpacity: text ? Number(getComputedStyle(text).opacity) : null,
      inactiveTextOpacity: document.querySelector('[data-testid="timed-lyrics"] .lyric-primary')
        ? Number(getComputedStyle(document.querySelector('[data-testid="timed-lyrics"] .lyric-primary')).opacity) : null,
      viewportBackground: viewport ? getComputedStyle(viewport).backgroundColor : null,
      activeCentered: active ? getComputedStyle(active.querySelector('.lyric-primary')).textAlign : null,
      centeredInViewport: active && viewport
        ? Math.abs((active.getBoundingClientRect().top + active.getBoundingClientRect().height / 2)
          - (viewport.getBoundingClientRect().top + viewport.clientHeight / 2)) < 2 : false,
    };
  });
  assert.equal(activeVisuals.activeBackground, 'rgba(0, 0, 0, 0)', 'active lyric rows have no background');
  assert.equal(activeVisuals.viewportBackground, 'rgba(0, 0, 0, 0)', 'lyrics pane has no background');
  assert.equal(activeVisuals.activeTextOpacity, 1, 'active lyric text stays opaque');
  assert.equal(activeVisuals.activeCentered, 'center');
  assert.ok(activeVisuals.centeredInViewport, 'auto-scroll centers the active row using variable offsets');

  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('paused'));
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).activeIndex, 90, 'paused playback keeps the active cue');
  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('stopped'));
  const stopped = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(stopped.activeIndex, null, 'stopped playback has no focused lyric row');
  assert.equal(stopped.timedWindowStart, 84, 'stopping keeps the held cue in the bounded window');
  assert.equal(stopped.toolbarAvailable, true);
  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('playing'));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(-1_000));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().timedScrollTop === 0);
  const beforeFirstCue = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(beforeFirstCue.activeIndex, null);
  assert.equal(beforeFirstCue.timedScrollTop, 0);
  await page.evaluate(() => window.lyricsViewHarness.setPosition(84_000));
  const soughtForward = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(soughtForward.activeIndex, 84);
  assert.ok(soughtForward.renderedTimedRows <= 13);
  await page.evaluate(() => window.lyricsViewHarness.setPlaying(false));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(12_000));
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).activeIndex, 12, 'backward seek updates the active cue');
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).getCount, 1, 'position updates do not reload lyrics');

  await page.evaluate(() => window.lyricsViewHarness.setTrack('candidate-track'));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().phase === 'candidates');
  const candidateState = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(candidateState.searchCount, 1, 'remote search starts after a local miss');
  assert.equal(candidateState.qrcNoticeVisible, true);
  assert.equal(candidateState.candidateActionEnabled, true);
  await page.getByRole('button', { name: '使用這份' }).click();
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().selectedSource === 'manual');

  await page.evaluate(() => window.lyricsViewHarness.setTrack('plain-track'));
  await page.waitForFunction(() => document.querySelector('[data-testid="plain-lyrics"]') !== null);
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({
    showTranslation: true,
    showRomanization: true,
    primaryFontSizePx: 36,
    auxiliaryFontSizePx: 24,
    lineGapPx: 0,
  }));
  await page.evaluate(() => window.lyricsViewHarness.scrollPlainTo(60));
  const plain = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.ok(plain.renderedPlainRows <= 20);
  assert.ok(plain.plainFirstIndex >= 54);
  assert.equal(plain.documentWidth, 360, 'narrow lyrics remain within the viewport');
  assertGeometry(plain, 'plain variable rows');
  assert.equal(plain.lineGapPx, 0);
  assert.equal(plain.toolbarAvailable, true);
  await page.getByRole('button', { name: '切換譯文顯示' }).click();
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).preferences.showTranslation, false);
  await page.evaluate(() => window.lyricsViewHarness.setTrack(null));
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).toolbarAvailable, true);

  await page.evaluate(() => localStorage.removeItem('__lyricsPreferences'));
  await page.goto(`${baseUrl}/tests/volume-slider-harness.html`);
  await page.locator('.nav-link').filter({ hasText: '設定' }).click();
  await page.getByRole('tab', { name: '歌詞' }).click();
  const settingsControls = await page.evaluate(() => {
    const sliders = [...document.querySelectorAll('.lyrics-preference-range input')];
    return {
      translationDefault: document.querySelectorAll('.lyrics-preference-toggle input')[0]?.checked,
      romanizationDefault: document.querySelectorAll('.lyrics-preference-toggle input')[1]?.checked,
      values: sliders.map((input) => input.value),
      bounds: sliders.map((input) => [input.min, input.max]),
      lineGapLabel: document.querySelector('input[aria-label="歌詞句間距"]')?.closest('label')?.innerText,
      activeTab: document.querySelector('#lyrics-tab')?.getAttribute('aria-selected'),
    };
  });
  assert.equal(settingsControls.translationDefault, false);
  assert.equal(settingsControls.romanizationDefault, false);
  assert.deepEqual(settingsControls.values, ['70', '14', '10', '24']);
  assert.deepEqual(settingsControls.bounds[3], ['0', '64']);
  assert.ok(settingsControls.lineGapLabel.includes('24px'));
  assert.equal(settingsControls.activeTab, 'true');
  await page.locator('input[aria-label="歌詞句間距"]').evaluate((input) => {
    input.value = '64';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  assert.equal(await page.locator('input[aria-label="歌詞句間距"]').inputValue(), '64', 'settings control updates the preference draft');
  await page.waitForFunction(() => document.querySelector('#lyrics-panel [role="status"]')?.textContent?.includes('歌詞設定已保存'));
  await page.reload();
  await page.waitForFunction(() => document.querySelector('.nav-link') !== null);
  await page.locator('.nav-link').filter({ hasText: '設定' }).click();
  await page.getByRole('tab', { name: '歌詞' }).click();
  await page.waitForFunction(() => document.querySelector('input[aria-label="歌詞句間距"]')?.value === '64');
  assert.equal(await page.locator('input[aria-label="歌詞句間距"]').inputValue(), '64', 'saved line gap is loaded after app reload');

  return {
    result: 'PASS',
    maxTimedRows: maximum.renderedTimedRows,
    activeAutoCentered: activeVisuals.centeredInViewport,
    missingAuxHeight: missingAuxCue.height,
    presentAuxHeight: presentAuxCue.height,
    plainRows: plain.renderedPlainRows,
    plainFirstIndex: plain.plainFirstIndex,
    settingsDefaults: settingsControls.values,
    settingsLineGapRange: settingsControls.bounds[3],
  };
}
