async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    notEqual(actual, expected, message) {
      if (actual === expected) throw new Error(`${message}: expected a value other than ${expected}`);
    },
    ok(value, message) {
      if (!value) throw new Error(message);
    },
    deepEqual(actual, expected, message) {
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
    },
  };
  const viewports = [
    { width: 3840, height: 2160, wide: true },
    { width: 2560, height: 1440, wide: true },
    { width: 1920, height: 1080, wide: true },
    { width: 1600, height: 900, wide: true },
    { width: 1366, height: 768, wide: true },
    { width: 1280, height: 1024, wide: true },
    { width: 1024, height: 768, wide: true },
    { width: 900, height: 900, wide: true },
    { width: 800, height: 1200, wide: true },
    { width: 720, height: 1280, wide: false },
    { width: 412, height: 915, wide: false },
    { width: 360, height: 800, wide: false },
  ];
  await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html');
  await page.waitForFunction(() => window.nowPlayingLayoutHarness && document.querySelector('[data-testid="timed-lyrics"]'));
  await page.waitForFunction(() => document.querySelector('[data-testid="cover-image"]')?.naturalWidth === 400);

  async function readLayout() {
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    return page.evaluate(() => window.nowPlayingLayoutHarness.snapshot());
  }

  function verifyShell(state, viewport, layout) {
    const expectedPaneOrder = layout === 'a' ? ['artwork', 'lyrics'] : ['lyrics', 'artwork'];
    const artwork = state.panes.find(pane => pane.name === 'artwork');
    const lyrics = state.panes.find(pane => pane.name === 'lyrics');
    const artworkCopy = state.artworkChildren.find(child => child.className === 'now-playing-copy');
    const isInside = (child, parent) => child && parent
      && child.width > 0 && child.height > 0
      && child.left >= parent.left - 1 && child.right <= parent.right + 1
      && child.top >= parent.top - 1 && child.bottom <= parent.bottom + 1;
    assert.notEqual(state.sidebarDisplay, 'none', `${viewport.width}x${viewport.height}: underlying side navigation should remain mounted and measurable`);
    assert.equal(state.outerOverflow, 'hidden', `${viewport.width}x${viewport.height}: Now Playing page scroll must be disabled`);
    assert.ok(state.documentWidth <= viewport.width + 1, `${viewport.width}x${viewport.height}: document width overflows`);
    assert.ok(state.documentHeight <= viewport.height + 1, `${viewport.width}x${viewport.height}: document height overflows`);
    assert.ok(state.outerScrollHeight <= state.outerClientHeight + 1, `${viewport.width}x${viewport.height}: page content exceeds its non-scroll region`);
    assert.equal(state.outerScrollTop, 0, `${viewport.width}x${viewport.height}: outer page accepted a scroll`);
    assert.ok(Math.abs(state.shellHeight - viewport.height) <= 1.1, `${viewport.width}x${viewport.height}: shell does not fill viewport height`);
    assert.ok(state.overlay.left <= state.sidebar.left + 1, `${viewport.width}x${viewport.height}: overlay does not cover the left navigation`);
    assert.ok(state.overlay.right >= state.dock.right - 1, `${viewport.width}x${viewport.height}: overlay does not span the full app width`);
    assert.ok(state.overlay.top <= 1 && state.overlay.bottom <= state.dock.top + 1, `${viewport.width}x${viewport.height}: overlay must cover the app content above the dock`);
    assert.ok(state.topbar.height > 0 && state.topbar.top >= 0, `${viewport.width}x${viewport.height}: top bar is missing`);
    assert.ok(state.dock.height > 0 && state.dock.bottom <= viewport.height + 1, `${viewport.width}x${viewport.height}: player dock is clipped`);
    assert.ok(state.card.height > 0 && state.card.top >= state.topbar.bottom - 1, `${viewport.width}x${viewport.height}: arrangement overlaps the top bar`);
    assert.ok(state.card.bottom <= state.dock.top + 1, `${viewport.width}x${viewport.height}: arrangement overlaps the dock`);
    assert.ok(isInside(state.card, state.nowPlaying), `${viewport.width}x${viewport.height}: arrangement exceeds the Now Playing region`);
    assert.ok(state.nowPlaying.bottom - state.card.bottom <= 1, `${viewport.width}x${viewport.height}: artwork and lyrics should fill the available area above the dock`);
    assert.ok(isInside(state.returnButton, state.topbar), `${viewport.width}x${viewport.height}: top-left return control is missing or clipped`);
    assert.ok(state.returnButton.left <= state.topbar.left + Math.max(80, viewport.width * 0.05), `${viewport.width}x${viewport.height}: return control should remain at the top-left of the overlay`);
    assert.ok(isInside(state.layoutSwitch, state.nowPlaying), `${viewport.width}x${viewport.height}: A/B layout switch is missing or outside Now Playing`);
    assert.ok(isInside(state.lyricsToolbar, lyrics), `${viewport.width}x${viewport.height}: lyric controls are missing or clipped in the lyrics pane`);
    assert.deepEqual(state.artworkCopyChildren, ['now-playing-format', 'h2', 'now-playing-artist', 'now-playing-album'], `${viewport.width}x${viewport.height}: artwork copy should show audio format, title, artist and album`);
    assert.equal(state.artworkCopyTextAlign, 'center', `${viewport.width}x${viewport.height}: all four artwork copy rows should be centered`);
    assert.equal(state.formatJustifyContent, 'center', `${viewport.width}x${viewport.height}: audio format and badge should be centered together`);
    assert.deepEqual(state.lyricTextAlign, { primary: 'center', translation: 'center', romanization: 'center' }, `${viewport.width}x${viewport.height}: lyric text and auxiliary lines should be centered`);
    assert.ok(artworkCopy && artworkCopy.top >= state.cover.bottom - 1, `${viewport.width}x${viewport.height}: title and artist must stay below the cover`);
    assert.ok(isInside(state.dockControls, state.dock), `${viewport.width}x${viewport.height}: playback controls are missing or clipped in the dock: ${JSON.stringify({ controls: state.dockControls, dock: state.dock })}`);
    assert.deepEqual(state.playerControlLabels, ['隨機播放', '上一首', '播放', '下一首', '循環播放'], `${viewport.width}x${viewport.height}: player controls must keep their Chinese accessible names`);
    assert.equal(state.playerControlIcons.length, 5, `${viewport.width}x${viewport.height}: Tabler player icons did not render for every control`);
    assert.ok(state.playerControlIcons.every(icon => icon.ariaHidden === 'true' && icon.width >= 18 && icon.height >= 18), `${viewport.width}x${viewport.height}: decorative control icons must be hidden from assistive technology and remain legible`);
    assert.ok(!['auto', 'scroll'].includes(state.artworkCopyOverflowY), `${viewport.width}x${viewport.height}: track information should not introduce a second scrollable window`);
    assert.equal(state.cover.objectFit, 'contain', `${viewport.width}x${viewport.height}: source image must keep its aspect ratio`);
    assert.deepEqual([state.cover.naturalWidth, state.cover.naturalHeight], [400, 300], `${viewport.width}x${viewport.height}: test artwork did not load at its source dimensions`);
    assert.ok(artwork && state.cover.width > 0 && state.cover.height > 0, `${viewport.width}x${viewport.height}: cover is missing`);
    assert.ok(state.cover.left >= artwork.left - 1 && state.cover.right <= artwork.right + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane horizontally`);
    assert.ok(state.cover.top >= artwork.top - 1 && state.cover.bottom <= artwork.bottom + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane vertically`);
    const copyGap = Math.max(4, artworkCopy.top - state.cover.bottom);
    const verticalBudget = artwork.height - artworkCopy.height - copyGap;
    const heightCap = viewport.width <= 720 ? viewport.height * 0.4 : viewport.height * 0.78;
    const expectedMaximum = Math.max(1, Math.min(artwork.width, verticalBudget, heightCap));
    assert.ok(state.cover.width >= expectedMaximum * 0.9, `${viewport.width}x${viewport.height}: cover does not use most of its available square area (${state.cover.width} of ${expectedMaximum})`);
    assert.deepEqual(state.panes.map(pane => pane.name), expectedPaneOrder, `${viewport.width}x${viewport.height}: pane order differs for layout ${layout}`);
    if (viewport.wide) {
      assert.ok(Math.abs(state.panes[0].top - state.panes[1].top) <= 2, `${viewport.width}x${viewport.height}: wide layout should remain side by side`);
      assert.ok(state.panes[0].left < state.panes[1].left, `${viewport.width}x${viewport.height}: A/B should switch the left-right order`);
    } else {
      assert.ok(state.panes[0].top < state.panes[1].top, `${viewport.width}x${viewport.height}: narrow layout should stack in A/B order`);
    }
    assert.ok(lyrics && state.lyricsViewport.clientHeight >= 64, `${viewport.width}x${viewport.height}: lyric viewport has no usable height`);
    assert.ok(state.lyricsViewport.scrollHeight > state.lyricsViewport.clientHeight, `${viewport.width}x${viewport.height}: long lyrics should scroll inside their own window`);
    assert.ok(['auto', 'scroll'].includes(state.lyricsViewport.overflowY), `${viewport.width}x${viewport.height}: lyric window is not scrollable`);
  }

  const results = [];
  for (const viewport of viewports) {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.waitForTimeout(35);
    await page.getByRole('button', { name: '排列 A：封面在前，歌詞在後' }).click();
    const wideState = await readLayout();
    verifyShell(wideState, viewport, 'a');
    const scrolled = await page.evaluate(() => window.nowPlayingLayoutHarness.scrollLyrics());
    assert.ok(scrolled, `${viewport.width}x${viewport.height}: lyrics failed to scroll independently`);
    const outerScrollTop = await page.evaluate(() => window.nowPlayingLayoutHarness.attemptOuterScroll());
    assert.equal(outerScrollTop, 0, `${viewport.width}x${viewport.height}: outer page changed scroll position`);

    await page.evaluate(() => {
      window.__nowPlayingLayoutNodes = [...document.querySelectorAll('.now-playing-card > [data-layout-pane]')];
    });
    await page.getByRole('button', { name: '排列 B：歌詞在前，封面在後' }).click();
    const layoutB = await readLayout();
    verifyShell(layoutB, viewport, 'b');
    const preserved = await page.evaluate(() => window.__nowPlayingLayoutNodes.every(node => document.querySelector('.now-playing-card > [data-layout-pane="' + node.dataset.layoutPane + '"]') === node));
    assert.ok(preserved, `${viewport.width}x${viewport.height}: changing A/B recreated a pane`);
    results.push({
      viewport: `${viewport.width}x${viewport.height}`,
      cover: `${Math.round(wideState.cover.width)}x${Math.round(wideState.cover.height)}`,
      artworkPane: `${Math.round(wideState.panes.find(pane => pane.name === 'artwork').width)}x${Math.round(wideState.panes.find(pane => pane.name === 'artwork').height)}`,
      artworkContent: wideState.artworkChildren.map(child => `${child.className || 'unnamed'}=${Math.round(child.width)}x${Math.round(child.height)}`),
      coverPaneRatio: [
        Number((wideState.cover.width / wideState.panes.find(pane => pane.name === 'artwork').width).toFixed(2)),
        Number((wideState.cover.height / wideState.panes.find(pane => pane.name === 'artwork').height).toFixed(2)),
      ],
      lyricsViewportHeight: Math.round(wideState.lyricsViewport.clientHeight),
      outerHeight: wideState.documentHeight,
      paneOrderA: wideState.panes.map(pane => pane.name),
      paneOrderB: layoutB.panes.map(pane => pane.name),
      lyricsScrollable: scrolled,
    });
  }

  return { result: 'PASS', viewports: results };
}
