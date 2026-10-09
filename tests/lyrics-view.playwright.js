async page => {
  const assert = {
    equal(actual, expected, message = 'values differ') {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    ok(value, message = 'expected a truthy value') {
      if (!value) throw new Error(message);
    },
    match(actual, pattern, message = 'value does not match') {
      if (!pattern.test(actual)) throw new Error(`${message}: ${pattern} vs ${JSON.stringify(actual)}`);
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
    assert.ok(Math.abs(state.headPaddingHeight + state.beforeSpacerHeight + renderedHeight + state.afterSpacerHeight + state.tailPaddingHeight - state.totalHeight) < 1.2, `${label}: row geometry and spacers must share prefix offsets`);
  }

  const initial = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(initial.trackId, 'timed-track');
  assert.equal(initial.getCount, 1);
  assert.equal(initial.searchCount, 0, 'local lyrics must suppress remote lookup');
  assert.ok(initial.renderedTimedRows <= 41, 'timed rows must stay within the 20-row-per-side virtual window');
  assertGeometry(initial, 'default timed');
  assert.equal(initial.toolbarAvailable, true, 'lyrics toggles remain enabled');
  assert.equal(initial.lineGapPx, 24);
  assert.equal(await page.locator('[data-testid="lyrics-view"] #lyrics-heading').count(), 0, 'LyricsView has no standalone heading');
  assert.equal(await page.locator('[data-testid="lyrics-view"] .lyrics-toggle').count(), 0, 'the translation controls belong to the Now Playing toolbar');
  assert.ok(Math.abs(initial.firstPrimaryTextHeight - 14 * 1.35) < 1, 'a short primary is measured as one line');

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ showTranslation: true, showRomanization: true }));
  const mixed = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assertGeometry(mixed, 'mixed timed rows');
  assert.ok(Math.abs(mixed.firstAuxiliaryTop - mixed.firstPrimaryBottom - 2) < 1, 'visible auxiliary text follows the actual primary height');
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

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ primaryFontSizePx: 50, auxiliaryFontSizePx: 50, lineGapPx: 50 }));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(90_000));
  await page.waitForTimeout(40);
  const maximum = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(maximum.activeIndex, 90);
  assert.ok(maximum.actualRowHeights.length <= 41, 'timed DOM remains bounded at maximum font sizes');
  assertGeometry(maximum, 'maximum timed');
  assert.ok(maximum.activeRowContentHeight <= maximum.activeRowBoxHeight + 0.6, 'clamped primary and present auxiliaries fit in the row');
  assert.ok(maximum.longPrimaryHeight >= maximum.longPrimaryLineHeight * 1.8, 'a genuinely wrapped primary takes two measured lines');
  assert.ok(maximum.longPrimaryHeight <= maximum.longPrimaryLineHeight * 2.05, 'primary remains clamped to two visible lines');
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
  assert.equal(activeVisuals.inactiveTextOpacity, 0.7, 'viewport fade is layered on the configured inactive text opacity');
  assert.equal(activeVisuals.activeCentered, 'center');
  assert.ok(activeVisuals.centeredInViewport, 'auto-scroll centers the active row using variable offsets');

  const firstAndLastCue = await page.evaluate(async () => {
    const check = async (positionMs) => {
      await window.lyricsViewHarness.setPosition(positionMs);
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
      const viewport = document.querySelector('[data-testid="timed-lyrics"]');
      const active = viewport?.querySelector('[aria-current="true"]');
      const rect = active?.getBoundingClientRect();
      return {
        index: Number(document.querySelector('[data-testid="lyrics-view"]')?.dataset.activeCueIndex),
        centered: Boolean(rect && viewport && Math.abs((rect.top + rect.height / 2) - (viewport.getBoundingClientRect().top + viewport.clientHeight / 2)) < 2),
        mask: viewport ? getComputedStyle(viewport).maskImage : '',
        head: Number(viewport?.querySelector('[data-edge-spacer="head"]')?.getBoundingClientRect().height ?? 0),
        tail: Number(viewport?.querySelector('[data-edge-spacer="tail"]')?.getBoundingClientRect().height ?? 0),
      };
    };
    return [await check(0), await check(119_000)];
  });
  assert.deepEqual(firstAndLastCue.map(({ index }) => index), [0, 119]);
  assert.ok(firstAndLastCue.every((cue) => cue.centered && cue.head > 0 && cue.tail > 0), 'first and last active cues remain centered with head and tail virtual space');
  assert.ok(firstAndLastCue.every((cue) => cue.mask.includes('linear-gradient')), 'the fade mask is applied to the lyric viewport');
  assert.equal(await page.locator('[data-testid="timed-lyrics"] .lyric-line').evaluateAll((rows) => rows.some((row) => getComputedStyle(row).borderBottomWidth !== '0px')), false, 'lyric rows have no separator borders');

  await page.evaluate(() => window.lyricsViewHarness.setPosition(90_000));
  await page.waitForFunction(() => {
    const viewport = document.querySelector('[data-testid="timed-lyrics"]');
    const active = viewport?.querySelector('[aria-current="true"]');
    return active && Math.abs((active.getBoundingClientRect().top + active.getBoundingClientRect().height / 2)
      - (viewport.getBoundingClientRect().top + viewport.clientHeight / 2)) < 2;
  });
  await page.evaluate(() => {
    const viewport = document.querySelector('[data-testid="timed-lyrics"]');
    const nativeScrollTo = viewport.scrollTo.bind(viewport);
    window.__lyricsScrollCalls = [];
    viewport.scrollTo = (options) => { window.__lyricsScrollCalls.push(options); nativeScrollTo(options); };
  });
  const scrollStart = await page.evaluate(() => document.querySelector('[data-testid="timed-lyrics"]').scrollTop);
  await page.evaluate(() => window.lyricsViewHarness.setPosition(91_000));
  await page.waitForFunction(() => window.__lyricsScrollCalls?.length === 1);
  await page.waitForTimeout(35);
  const midflight = await page.evaluate(() => ({
    scrollTop: document.querySelector('[data-testid="timed-lyrics"]').scrollTop,
    target: window.__lyricsScrollCalls?.[0]?.top,
  }));
  assert.ok(midflight.scrollTop > scrollStart + 1 && midflight.scrollTop < midflight.target - 1, 'normal cue navigation has a visible in-flight scroll position');
  await page.evaluate(() => window.lyricsViewHarness.setPosition(91_250));
  await page.waitForTimeout(250);
  assert.deepEqual(await page.evaluate(() => window.__lyricsScrollCalls?.map((options) => options.behavior)), ['smooth'], '250ms same-cue playback snapshots do not restart or interrupt the normal cue animation');
  await page.evaluate(() => window.lyricsViewHarness.setPosition(92_000));
  await page.waitForFunction(() => window.__lyricsScrollCalls?.length === 2);
  await page.waitForFunction(() => {
    const viewport = document.querySelector('[data-testid="timed-lyrics"]');
    const active = viewport?.querySelector('[aria-current="true"]');
    return active && Math.abs((active.getBoundingClientRect().top + active.getBoundingClientRect().height / 2)
      - (viewport.getBoundingClientRect().top + viewport.clientHeight / 2)) < 2;
  });
  const latestNavigation = await page.evaluate(() => ({
    behaviors: window.__lyricsScrollCalls?.map((options) => options.behavior),
    cue: Number(document.querySelector('[data-testid="lyrics-view"]')?.dataset.activeCueIndex),
  }));
  assert.equal(latestNavigation.cue, 92, 'the latest cue wins after rapid updates');
  assert.deepEqual(latestNavigation.behaviors?.slice(0, 2), ['smooth', 'smooth'], 'rapid adjacent cues retarget instead of queueing');
  assert.ok((latestNavigation.behaviors?.length ?? 0) <= 3, 'at most one post-animation measured center correction is issued');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  const reducedMotionCallCount = latestNavigation.behaviors?.length ?? 0;
  await page.evaluate(() => window.lyricsViewHarness.setPosition(93_000));
  await page.waitForFunction((previousCount) => (window.__lyricsScrollCalls?.length ?? 0) > previousCount, reducedMotionCallCount);
  assert.equal(await page.evaluate(() => window.__lyricsScrollCalls?.at(-1)?.behavior), 'instant', 'reduced-motion preference disables cue animation');
  await page.emulateMedia({ reducedMotion: 'no-preference' });

  await page.evaluate(() => window.lyricsViewHarness.setPreferences({ primaryFontSizePx: 14, auxiliaryFontSizePx: 10 }));
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.waitForTimeout(80);
  const widePrimary = await page.evaluate(() => window.lyricsViewHarness.snapshot().longPrimaryHeight);
  await page.setViewportSize({ width: 360, height: 800 });
  await page.waitForTimeout(80);
  const narrowPrimary = await page.evaluate(() => window.lyricsViewHarness.snapshot().longPrimaryHeight);
  assert.ok(narrowPrimary > widePrimary + 8, 'measured height is recalculated when the primary wraps at a narrower width');

  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('paused'));
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).activeIndex, 93, 'paused playback keeps the active cue');
  await page.evaluate(() => window.lyricsViewHarness.setPlaybackState('stopped'));
  const stopped = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(stopped.activeIndex, null, 'stopped playback has no focused lyric row');
  assert.equal(stopped.timedWindowStart, 73, 'stopping keeps the held cue in the bounded window');
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
  assert.ok(soughtForward.renderedTimedRows <= 41, 'seek keeps the timed DOM bounded');
  await page.evaluate(() => window.lyricsViewHarness.setPlaying(false));
  await page.evaluate(() => window.lyricsViewHarness.setPosition(12_000));
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).activeIndex, 12, 'backward seek updates the active cue');
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).getCount, 1, 'position updates do not reload lyrics');

  await page.evaluate(() => window.lyricsViewHarness.setTrack('empty-results-track'));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().phase === 'empty');
  assert.equal(await page.locator('[data-testid="lyrics-candidates"]').count(), 1, 'empty search still shows the candidate picker panel');
  assert.equal(await page.locator('[data-testid="lyrics-remove"]').count(), 1, 'empty search still shows remove lyrics');
  assert.equal(await page.locator('[data-testid="lyrics-candidates-search"]').count(), 1, 'empty search still shows the text search field');
  assert.equal(await page.getByRole('heading', { name: '選擇歌詞' }).count(), 0, 'candidate picker has no title row');
  assert.equal(await page.getByRole('button', { name: '再次搜尋' }).count(), 0, 'candidate picker has no second search button');
  const emptyActions = await page.locator('.lyrics-candidates-actions').evaluate((actions) => {
    const form = actions.closest('form');
    const input = form?.querySelector('[data-testid="lyrics-candidates-search"]');
    const actionsRect = actions.getBoundingClientRect();
    const formRect = form?.getBoundingClientRect();
    const inputRect = input?.getBoundingClientRect();
    return {
      labels: [...actions.querySelectorAll('button')].map((button) => button.textContent?.trim()),
      flushRight: formRect ? Math.abs(actionsRect.right - formRect.right) < 2 : false,
      besideOrBelowInput: inputRect ? actionsRect.left >= inputRect.left - 1 : false,
    };
  });
  assert.deepEqual(emptyActions.labels, ['搜尋', '關閉', '移除歌詞'], 'picker chrome is search, close, and remove');
  assert.equal(emptyActions.flushRight, true, 'picker chrome sits on the right edge');
  assert.equal(emptyActions.besideOrBelowInput, true, 'picker chrome stays with the search field');
  assert.match(await page.locator('.lyrics-candidate-empty').innerText(), /找不到符合的歌詞/);
  const emptySearches = (await page.evaluate(() => window.lyricsViewHarness.snapshot())).searchCount;
  await page.locator('.lyrics-candidates-search button[type="submit"]').click();
  await page.waitForFunction((previous) => window.lyricsViewHarness.snapshot().searchCount > previous, emptySearches);
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).lastSearchQuery, null, 'an empty search box searches with the current track');
  await page.locator('[data-testid="lyrics-remove"]').click();
  await page.waitForFunction(() => document.querySelector('[data-testid="lyrics-candidates"]') === null);

  const searchCountBeforeCandidates = (await page.evaluate(() => window.lyricsViewHarness.snapshot())).searchCount;
  await page.evaluate(() => window.lyricsViewHarness.setTrack('candidate-track'));
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().phase === 'candidates');
  const candidateState = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.equal(candidateState.searchCount - searchCountBeforeCandidates, 1, 'remote search starts after a local miss');
  assert.equal(candidateState.qrcNoticeVisible, true);
  assert.equal(candidateState.candidateActionEnabled, true);
  assert.equal(await page.locator('.lyrics-candidates').evaluate((element) => Number.parseFloat(getComputedStyle(element).maxHeight)), 400, 'candidate selector grows to the bounded 50vh cap at 800px height');
  assert.ok(await page.locator('.lyrics-candidates').evaluate((element) => Number.parseFloat(getComputedStyle(element).maxHeight)) > 260, 'candidate selector is taller than its previous 260px cap');
  assert.equal(await page.locator('[data-testid="lyrics-candidates-search"]').count(), 1, 'candidate picker keeps the text search field');
  async function focusRingInsideClip(locator, label) {
    await locator.evaluate((element) => {
      if (typeof element.focus === 'function') element.focus({ focusVisible: true });
    });
    const ring = await locator.evaluate((element) => {
      const style = getComputedStyle(element);
      const outset = Math.max(0, Number.parseFloat(style.outlineOffset) + Number.parseFloat(style.outlineWidth));
      const rect = element.getBoundingClientRect();
      const ringRect = { left: rect.left - outset, right: rect.right + outset, top: rect.top - outset, bottom: rect.bottom + outset };
      const clipped = [];
      for (let node = element.parentElement; node; node = node.parentElement) {
        const nodeStyle = getComputedStyle(node);
        if (nodeStyle.overflowX === 'visible' && nodeStyle.overflowY === 'visible') continue;
        const clip = node.getBoundingClientRect();
        if (ringRect.left < clip.left - 0.5 || ringRect.right > clip.right + 0.5 || ringRect.top < clip.top - 0.5 || ringRect.bottom > clip.bottom + 0.5) {
          clipped.push(`${node.className}: ring ${JSON.stringify(ringRect)} clip ${JSON.stringify({ left: clip.left, right: clip.right, top: clip.top, bottom: clip.bottom })}`);
        }
      }
      return { focusVisible: element.matches(':focus-visible'), outlineStyle: style.outlineStyle, clipped };
    });
    assert.ok(ring.focusVisible, `${label}: focus-visible should apply`);
    assert.ok(ring.outlineStyle !== 'none', `${label}: focus ring should be visible`);
    assert.deepEqual(ring.clipped, [], `${label}: focus ring must not be clipped by overflow ancestors`);
  }
  await focusRingInsideClip(page.locator('[data-testid="lyrics-candidates-search"]'), 'candidate search input');
  await focusRingInsideClip(page.locator('.lyrics-candidates-search button[type="submit"]'), 'candidate search button');
  await focusRingInsideClip(page.locator('[data-testid="lyrics-dismiss"]'), 'candidate close button');
  assert.equal(await page.locator('[data-testid="lyrics-dismiss"]').count(), 1, 'header close remains cancel-only dismiss');
  assert.equal(await page.locator('[data-testid="lyrics-remove"]').count(), 1, 'footer always exposes remove-lyrics');
  const removeInsidePanel = await page.locator('[data-testid="lyrics-remove"]').evaluate((button) => {
    const panel = button.closest('.lyrics-candidates');
    const buttonRect = button.getBoundingClientRect();
    const panelRect = panel.getBoundingClientRect();
    return buttonRect.top >= panelRect.top - 0.5
      && buttonRect.bottom <= panelRect.bottom + 0.5
      && buttonRect.height > 0;
  });
  assert.equal(removeInsidePanel, true, 'remove-lyrics stays inside the candidate panel');
  const removeInsideShortPane = await page.evaluate(() => {
    const view = document.querySelector('.lyrics-view');
    const parent = view.parentElement;
    const pane = document.createElement('section');
    pane.className = 'now-playing-lyrics';
    pane.style.height = '240px';
    pane.style.overflow = 'hidden';
    pane.style.setProperty('--np-cover-height', '640px');
    parent.insertBefore(pane, view);
    pane.appendChild(view);
    const button = document.querySelector('[data-testid="lyrics-remove"]');
    const buttonRect = button.getBoundingClientRect();
    const clip = pane.getBoundingClientRect();
    const visible = buttonRect.height > 0 && buttonRect.top >= clip.top - 1 && buttonRect.bottom <= clip.bottom + 1;
    parent.insertBefore(view, pane);
    pane.remove();
    return visible;
  });
  assert.equal(removeInsideShortPane, true, 'remove-lyrics stays visible when the cover is taller than the lyrics pane');
  const typedSearches = (await page.evaluate(() => window.lyricsViewHarness.snapshot())).searchCount;
  await page.locator('[data-testid="lyrics-candidates-search"]').fill('拼湊的斷音');
  await page.locator('.lyrics-candidates-search button[type="submit"]').click();
  await page.waitForFunction((previous) => window.lyricsViewHarness.snapshot().searchCount > previous, typedSearches);
  assert.equal((await page.evaluate(() => window.lyricsViewHarness.snapshot())).lastSearchQuery, '拼湊的斷音', 'typed text overrides the provider query');
  await page.locator('[data-testid="lyrics-candidates-search"]').fill('');
  await page.locator('[data-testid="lyrics-dismiss"]').click();
  await page.waitForFunction(() => document.querySelector('[data-testid="lyrics-candidates"]') === null);
  await page.evaluate(async () => {
    await window.lyricsViewHarness.setTrack(null);
    await window.lyricsViewHarness.setTrack('candidate-track');
  });
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().phase === 'candidates');
  await focusRingInsideClip(page.getByRole('button', { name: '選擇' }), 'candidate select button');
  await page.getByRole('button', { name: '選擇' }).click();
  await page.waitForFunction(() => window.lyricsViewHarness.snapshot().selectedSource === 'manual');

  await page.evaluate(() => window.lyricsViewHarness.setTrack('plain-track'));
  await page.waitForFunction(() => document.querySelector('[data-testid="plain-lyrics"]') !== null);
  await page.evaluate(() => window.lyricsViewHarness.setPreferences({
    showTranslation: true,
    showRomanization: true,
    primaryFontSizePx: 50,
    auxiliaryFontSizePx: 50,
    lineGapPx: 0,
  }));
  await page.evaluate(() => window.lyricsViewHarness.scrollPlainTo(60));
  await page.waitForFunction(() => {
    const viewport = document.querySelector('[data-testid="plain-lyrics"]');
    if (!viewport) return false;
    const spacers = [...viewport.querySelectorAll('.lyrics-spacer')];
    const rows = [...viewport.querySelectorAll('.lyric-line')];
    const actual = spacers.reduce((sum, spacer) => sum + spacer.getBoundingClientRect().height, 0)
      + rows.reduce((sum, row) => sum + row.getBoundingClientRect().height + Number.parseFloat(getComputedStyle(row).marginBottom), 0);
    return Math.abs(actual - Number(viewport.dataset.totalHeight)) < 1.2;
  });
  const plain = await page.evaluate(() => window.lyricsViewHarness.snapshot());
  assert.ok(plain.renderedPlainRows <= 20, 'plain DOM remains bounded');
  assert.ok(plain.plainFirstIndex >= 45, 'plain scrolling reaches the requested distant line with bounded overscan');
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
  assert.deepEqual(settingsControls.bounds[1], ['0', '50']);
  assert.deepEqual(settingsControls.bounds[2], ['0', '50']);
  assert.deepEqual(settingsControls.bounds[3], ['0', '50']);
  assert.ok(settingsControls.lineGapLabel.includes('24px'));
  assert.equal(settingsControls.activeTab, 'true');
  await page.locator('input[aria-label="歌詞句間距"]').evaluate((input) => {
    input.value = '50';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  assert.equal(await page.locator('input[aria-label="歌詞句間距"]').inputValue(), '50', 'settings control updates the preference draft');
  await page.waitForFunction(() => document.querySelector('#lyrics-panel [role="status"]')?.textContent?.includes('歌詞設定已保存'));
  await page.reload();
  await page.waitForFunction(() => document.querySelector('.nav-link') !== null);
  await page.locator('.nav-link').filter({ hasText: '設定' }).click();
  await page.getByRole('tab', { name: '歌詞' }).click();
  await page.waitForFunction(() => document.querySelector('input[aria-label="歌詞句間距"]')?.value === '50');
  assert.equal(await page.locator('input[aria-label="歌詞句間距"]').inputValue(), '50', 'saved line gap is loaded after app reload');

  await page.goto(`${baseUrl}/tests/lyrics-view-harness.html`);
  await page.waitForFunction(() => window.lyricsViewHarness?.snapshot().phase === 'ready');
  await page.evaluate(() => window.lyricsViewHarness.setPosition(60_000));
  await page.waitForFunction(() => {
    const viewport = document.querySelector('[data-testid="timed-lyrics"]');
    const active = viewport?.querySelector('[aria-current="true"]');
    return active && Math.abs((active.getBoundingClientRect().top + active.getBoundingClientRect().height / 2)
      - (viewport.getBoundingClientRect().top + viewport.clientHeight / 2)) < 2;
  });
  await page.screenshot({ path: 'target/lyrics-spatial-fade.png', fullPage: false });

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
