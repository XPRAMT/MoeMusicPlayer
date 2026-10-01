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
    assert.ok(isInside(state.lyricsToolbar, state.topbar), `${viewport.width}x${viewport.height}: lyric controls are missing or clipped in the top bar`);
    assert.deepEqual(state.artworkCopyChildren, [], `${viewport.width}x${viewport.height}: no text should be rendered below the cover`);
    assert.equal(state.formatJustifyContent, 'flex-start', `${viewport.width}x${viewport.height}: audio format follows the top track information`);
    assert.deepEqual(state.lyricTextAlign, { primary: 'center', translation: 'center', romanization: 'center' }, `${viewport.width}x${viewport.height}: lyric text and auxiliary lines should be centered`);
    assert.ok(isInside(state.dockControls, state.dock), `${viewport.width}x${viewport.height}: playback controls are missing or clipped in the dock: ${JSON.stringify({ controls: state.dockControls, dock: state.dock })}`);
    assert.deepEqual(state.playerControlLabels, ['隨機播放', '上一首', '播放', '下一首', '循環播放'], `${viewport.width}x${viewport.height}: player controls must keep their Chinese accessible names`);
    assert.equal(state.playerControlIcons.length, 5, `${viewport.width}x${viewport.height}: Tabler player icons did not render for every control`);
    assert.ok(state.playerControlIcons.every(icon => icon.ariaHidden === 'true' && icon.width >= 18 && icon.height >= 18), `${viewport.width}x${viewport.height}: decorative control icons must be hidden from assistive technology and remain legible`);
    assert.equal(state.artworkCopyOverflowY, 'missing', `${viewport.width}x${viewport.height}: cover area must not contain track copy`);
    assert.equal(state.cover.objectFit, 'contain', `${viewport.width}x${viewport.height}: source image must keep its aspect ratio`);
    assert.ok(state.cover.naturalWidth > 0 && state.cover.naturalHeight > 0, `${viewport.width}x${viewport.height}: test artwork did not load at its source dimensions`);
    assert.ok(artwork && state.cover.width > 0 && state.cover.height > 0, `${viewport.width}x${viewport.height}: cover is missing`);
    const stageAspect = state.cover.width / state.cover.height;
    const sourceAspect = state.cover.naturalWidth / state.cover.naturalHeight;
    assert.ok(Math.abs(stageAspect - sourceAspect) <= 0.02, `${viewport.width}x${viewport.height}: cover frame ${state.cover.width}x${state.cover.height} must match the ${state.cover.naturalWidth}x${state.cover.naturalHeight} artwork aspect ratio`);
    assert.ok(Math.abs(state.cover.width - state.renderedImage.width - 2) <= 1.5 && Math.abs(state.cover.height - state.renderedImage.height - 2) <= 1.5,
      `${viewport.width}x${viewport.height}: frame ${state.cover.width}x${state.cover.height} must hug the visible ${state.renderedImage.width}x${state.renderedImage.height} image with only a 1px border`);
    assert.ok(Math.abs(state.renderedImage.width / state.renderedImage.height - sourceAspect) <= 0.01,
      `${viewport.width}x${viewport.height}: visible pixels ${state.renderedImage.width}x${state.renderedImage.height} must preserve source ratio ${sourceAspect}`);
    assert.ok(state.cover.left >= artwork.left - 1 && state.cover.right <= artwork.right + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane horizontally`);
    assert.ok(state.cover.top >= artwork.top - 1 && state.cover.bottom <= artwork.bottom + 1, `${viewport.width}x${viewport.height}: cover spills out of artwork pane vertically`);
    assert.ok(Math.abs((state.cover.top + state.cover.bottom) / 2 - (artwork.top + artwork.bottom) / 2) <= 2, `${viewport.width}x${viewport.height}: cover should be vertically centered in the artwork pane`);
    const heightCap = viewport.height * 0.92;
    const expectedScale = Math.min((artwork.width - 2) / state.cover.naturalWidth, (Math.min(artwork.height, heightCap) - 2) / state.cover.naturalHeight);
    assert.ok(Math.abs(state.renderedImage.width - state.cover.naturalWidth * expectedScale) <= 2
      && Math.abs(state.renderedImage.height - state.cover.naturalHeight * expectedScale) <= 2,
    `${viewport.width}x${viewport.height}: visible image ${state.renderedImage.width}x${state.renderedImage.height} does not maximize the available area without letterboxing`);
    assert.ok(state.typography.format >= 9 && state.typography.title >= 11
      && state.typography.artist >= 11 && state.typography.album >= 11,
    `${viewport.width}x${viewport.height}: topbar track information must remain readable: ${JSON.stringify(state.typography)}`);
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
    await page.evaluate(() => window.nowPlayingLayoutHarness.setArtworkVariant('square'));
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
      typography: wideState.typography,
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

  const focusedGeometry = [];
  for (const [viewport, variant] of [
    [{ width: 1920, height: 1080, wide: true }, 'square'],
    [{ width: 1920, height: 1080, wide: true }, 'portrait'],
    [{ width: 1920, height: 1080, wide: true }, 'landscape'],
    [{ width: 1366, height: 768, wide: true }, 'square'],
    [{ width: 1366, height: 768, wide: true }, 'portrait'],
    [{ width: 1366, height: 768, wide: true }, 'landscape'],
    [{ width: 412, height: 915, wide: false }, 'square'],
    [{ width: 412, height: 915, wide: false }, 'portrait'],
    [{ width: 412, height: 915, wide: false }, 'landscape'],
  ]) {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.evaluate((next) => window.nowPlayingLayoutHarness.setArtworkVariant(next), variant);
    await page.evaluate(() => window.nowPlayingLayoutHarness.setLayout('a'));
    const state = await readLayout();
    verifyShell(state, viewport, 'a');
    await page.evaluate(() => window.nowPlayingLayoutHarness.setLayout('b'));
    const stateB = await readLayout();
    verifyShell(stateB, viewport, 'b');
    focusedGeometry.push({ viewport: `${viewport.width}x${viewport.height}`, variant, frame: `${state.cover.width.toFixed(1)}x${state.cover.height.toFixed(1)}`, image: `${state.renderedImage.width.toFixed(1)}x${state.renderedImage.height.toFixed(1)}` });
  }

  const androidInsetsGeometry = [];
  for (const testCase of [
    { name: 'gesture-nav-hidden', safeLeft: 0, safeTop: 42, safeRight: 0, safeBottom: 0 },
    { name: 'three-button-nav-visible', safeLeft: 0, safeTop: 42, safeRight: 0, safeBottom: 24 },
    { name: 'cutout-and-bars', safeLeft: 14, safeTop: 36, safeRight: 10, safeBottom: 0 },
  ]) {
    await page.setViewportSize({ width: 412, height: 915 });
    await page.evaluate((insets) => {
      const root = document.documentElement;
      root.style.setProperty('--android-safe-left', `${insets.safeLeft}px`);
      root.style.setProperty('--android-safe-top', `${insets.safeTop}px`);
      root.style.setProperty('--android-safe-right', `${insets.safeRight}px`);
      root.style.setProperty('--android-safe-bottom', `${insets.safeBottom}px`);
    }, testCase);
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const geometry = await page.evaluate(() => {
      const bounds = (selector) => {
        const rect = document.querySelector(selector).getBoundingClientRect();
        return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, width: rect.width, height: rect.height };
      };
      return {
        viewport: { width: window.innerWidth, height: window.innerHeight },
        shell: bounds('[data-testid="now-playing-shell"]'),
        backdrop: bounds('[data-testid="now-playing-backdrop"]'),
        topbar: bounds('[data-testid="layout-topbar"]'),
        dock: bounds('[data-testid="layout-dock"]'),
      };
    });
    assert.ok(Math.abs(geometry.shell.left) <= 1 && Math.abs(geometry.shell.top) <= 1
      && Math.abs(geometry.shell.right - geometry.viewport.width) <= 1
      && Math.abs(geometry.shell.bottom - geometry.viewport.height) <= 1,
    `${testCase.name}: app shell must fill the full viewport while its children use safe padding`);
    assert.ok(geometry.backdrop.left <= 0 && geometry.backdrop.top <= 0
      && geometry.backdrop.right >= geometry.viewport.width
      && geometry.backdrop.bottom >= geometry.viewport.height,
    `${testCase.name}: Now Playing backdrop must reach behind every system bar`);
    assert.ok(geometry.topbar.top >= testCase.safeTop - 1
      && geometry.topbar.left >= testCase.safeLeft - 1
      && geometry.topbar.right <= geometry.viewport.width - testCase.safeRight + 1,
    `${testCase.name}: top controls must stay within status/cutout safe bounds`);
    assert.ok(geometry.dock.bottom <= geometry.viewport.height - testCase.safeBottom + 1,
      `${testCase.name}: dock must avoid visible tappable navigation UI`);
    if (testCase.safeBottom === 0) {
      assert.ok(Math.abs(geometry.dock.bottom - geometry.viewport.height) <= 1,
        `${testCase.name}: hidden gesture navigation must not leave a reserved bottom gap`);
    }
    androidInsetsGeometry.push({ name: testCase.name, backdrop: geometry.backdrop, topbar: geometry.topbar, dock: geometry.dock });
  }

  return { result: 'PASS', viewports: results, focusedGeometry, androidInsetsGeometry };
}
