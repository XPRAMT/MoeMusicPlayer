async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
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
    { width: 1920, height: 1080, wide: true },
    { width: 1366, height: 768, wide: true },
    { width: 900, height: 900, wide: true },
    { width: 412, height: 915, wide: false },
    { width: 360, height: 800, wide: false },
  ];
  await page.goto('http://127.0.0.1:4173/tests/now-playing-layout-harness.html');
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
    assert.equal(state.sidebarDisplay, 'none', `${viewport.width}x${viewport.height}: side navigation should be hidden`);
    assert.equal(state.outerOverflow, 'hidden', `${viewport.width}x${viewport.height}: Now Playing page scroll must be disabled`);
    assert.ok(state.documentWidth <= viewport.width + 1, `${viewport.width}x${viewport.height}: document width overflows`);
    assert.ok(state.documentHeight <= viewport.height + 1, `${viewport.width}x${viewport.height}: document height overflows`);
    assert.ok(state.outerScrollHeight <= state.outerClientHeight + 1, `${viewport.width}x${viewport.height}: page content exceeds its non-scroll region`);
    assert.equal(state.outerScrollTop, 0, `${viewport.width}x${viewport.height}: outer page accepted a scroll`);
    assert.ok(Math.abs(state.shellHeight - viewport.height) <= 1.1, `${viewport.width}x${viewport.height}: shell does not fill viewport height`);
    assert.ok(state.topbar.height > 0 && state.topbar.top >= 0, `${viewport.width}x${viewport.height}: top bar is missing`);
    assert.ok(state.dock.height > 0 && state.dock.bottom <= viewport.height + 1, `${viewport.width}x${viewport.height}: player dock is clipped`);
    assert.ok(state.card.height > 0 && state.card.top >= state.topbar.bottom - 1, `${viewport.width}x${viewport.height}: arrangement overlaps the top bar`);
    assert.ok(state.card.bottom <= state.dock.top + 1, `${viewport.width}x${viewport.height}: arrangement overlaps the dock`);
    assert.ok(isInside(state.card, state.nowPlaying), `${viewport.width}x${viewport.height}: arrangement exceeds the Now Playing region`);
    assert.ok(isInside(state.returnButton, artwork), `${viewport.width}x${viewport.height}: return-to-library control is missing or clipped in the artwork pane`);
    assert.ok(isInside(state.returnButton, artworkCopy), `${viewport.width}x${viewport.height}: long track title clipped the return-to-library control`);
    assert.ok(isInside(state.layoutSwitch, state.nowPlaying), `${viewport.width}x${viewport.height}: A/B layout switch is missing or outside Now Playing`);
    assert.ok(isInside(state.lyricsToolbar, lyrics), `${viewport.width}x${viewport.height}: lyric controls are missing or clipped in the lyrics pane`);
    assert.ok(isInside(state.dockControls, state.dock), `${viewport.width}x${viewport.height}: playback controls are missing or clipped in the dock: ${JSON.stringify({ controls: state.dockControls, dock: state.dock })}`);
    assert.ok(!['auto', 'scroll'].includes(state.artworkCopyOverflowY), `${viewport.width}x${viewport.height}: track information should not introduce a second scrollable window`);
    assert.equal(state.cover.objectFit, 'contain', `${viewport.width}x${viewport.height}: source image must keep its aspect ratio`);
    assert.deepEqual([state.cover.naturalWidth, state.cover.naturalHeight], [400, 300], `${viewport.width}x${viewport.height}: test artwork did not load at its source dimensions`);
    assert.ok(artwork && state.cover.width > 0 && state.cover.height > 0, `${viewport.width}x${viewport.height}: cover is missing`);
    assert.ok(state.cover.left >= artwork.left - 1 && state.cover.right <= artwork.right + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane horizontally`);
    assert.ok(state.cover.top >= artwork.top - 1 && state.cover.bottom <= artwork.bottom + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane vertically`);
    assert.ok(state.cover.width >= artwork.width * 0.34, `${viewport.width}x${viewport.height}: cover is unexpectedly small for its available pane`);
    if (viewport.width >= 1024 && viewport.height <= 820) {
      assert.ok(state.cover.width >= artwork.width * 0.5, `${viewport.width}x${viewport.height}: short wide layout should allocate at least half the artwork pane width to cover`);
    }
    if (viewport.width <= 620) {
      assert.ok(state.cover.width >= artwork.width * 0.45, `${viewport.width}x${viewport.height}: stacked layout should preserve enough pane width for artwork`);
    }
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
