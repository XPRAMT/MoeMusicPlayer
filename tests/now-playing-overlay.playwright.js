async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    deepEqual(actual, expected, message) {
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
    },
    ok(value, message) {
      if (!value) throw new Error(message);
    },
  };

  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('http://127.0.0.1:4173/tests/volume-slider-harness.html');
  await page.waitForFunction(() => document.querySelector('.dock-art')?.disabled === false);
  await page.waitForFunction(() => document.querySelectorAll('.track-list-viewport .paged-virtual-row').length > 0);

  async function rememberUnderlyingState(includeListScroll = false) {
    await page.evaluate((saveScroll) => {
      const list = document.querySelector('.track-list-viewport');
      if (list && saveScroll) {
        list.scrollTop = Math.min(550, list.scrollHeight - list.clientHeight);
        list.dispatchEvent(new Event('scroll', { bubbles: true }));
      }
      window.__nowPlayingOrigin = {
        view: document.querySelector('.app-shell')?.getAttribute('data-active-view'),
        page: document.querySelector('.workspace .page-content'),
        list,
        listScrollTop: list?.scrollTop ?? null,
        selectedLyricsTab: document.querySelector('#lyrics-tab')?.getAttribute('aria-selected') ?? null,
      };
    }, includeListScroll);
    await page.waitForTimeout(35);
  }

  async function openOverlay() {
    await page.locator('.dock-art').click();
    await page.waitForFunction(() => document.querySelector('.now-playing-overlay')?.classList.contains('is-open'));
    await page.locator('.now-playing-card').waitFor({ state: 'visible' });
    assert.equal(await page.locator('.now-playing-overlay .now-playing-layout-switch').count(), 0, 'Now Playing page must not contain an A/B switch');
    assert.equal(await page.locator('.now-playing-overlay .runtime-pill').count(), 0, 'Now Playing header must not show desktop service status');
  }

  async function selectLayoutInSettings(layout) {
    await page.getByRole('button', { name: '設定', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('.app-shell')?.getAttribute('data-active-view') === 'settings');
    await page.getByRole('tab', { name: '正在播放' }).click();
    await page.getByRole('button', { name: layout === 'a' ? '排列 A：封面在前，歌詞在後' : '排列 B：歌詞在前，封面在後' }).click();
    await page.waitForFunction((expected) => document.querySelector('#now-playing-layout-panel > [role="status"]')?.textContent?.includes(`排列 ${expected.toUpperCase()} 已保存`), layout);
  }

  async function readOverlayLayout() {
    return await page.evaluate(() => {
      const rect = (element) => {
        const value = element.getBoundingClientRect();
        return { left: Math.round(value.left), top: Math.round(value.top), right: Math.round(value.right), bottom: Math.round(value.bottom), width: Math.round(value.width), height: Math.round(value.height) };
      };
      const artwork = document.querySelector('.now-playing-card [data-layout-pane="artwork"]');
      const lyrics = document.querySelector('.now-playing-card [data-layout-pane="lyrics"]');
      const stage = document.querySelector('.now-playing-copy')?.previousElementSibling;
      const copy = document.querySelector('.now-playing-copy');
      return {
        layout: document.querySelector('.now-playing-card')?.dataset.layout,
        panes: [...document.querySelectorAll('.now-playing-card > [data-layout-pane]')].map((pane) => pane.dataset.layoutPane),
        artwork: rect(artwork),
        lyrics: rect(lyrics),
        cover: rect(stage),
        copy: rect(copy),
        coverFit: getComputedStyle(document.querySelector('.cover-stage-image')).objectFit,
        coverSource: [document.querySelector('.cover-stage-image').naturalWidth, document.querySelector('.cover-stage-image').naturalHeight],
        copyChildren: [...copy.children].map((child) => child.className.toString() || child.tagName.toLowerCase()),
        formatText: copy.querySelector('.now-playing-format')?.textContent?.trim() ?? '',
        hiResBadge: (() => {
          const image = copy.querySelector('.now-playing-format img');
          return image ? { alt: image.alt, complete: image.complete, naturalWidth: image.naturalWidth } : null;
        })(),
        errorText: copy.querySelector('.error-note')?.textContent ?? null,
        lyricsViewport: rect(lyrics.querySelector('.timed-lyrics-viewport, .plain-lyrics-viewport')),
        lyricsOverflow: getComputedStyle(lyrics.querySelector('.timed-lyrics-viewport, .plain-lyrics-viewport')).overflowY,
        pageOverflow: getComputedStyle(document.querySelector('.now-playing-overlay-body')).overflowY,
        return: rect(document.querySelector('.now-playing-overlay-return')),
        topbar: rect(document.querySelector('.now-playing-overlay-topbar')),
        dock: rect(document.querySelector('.player-dock')),
      };
    });
  }

  async function verifyAndRestore(returnMethod) {
    const overlayState = await page.evaluate(() => {
      const shell = document.querySelector('.app-shell');
      const overlay = document.querySelector('.now-playing-overlay');
      const sidebar = document.querySelector('.sidebar');
      const workspace = document.querySelector('.workspace');
      const dock = document.querySelector('.player-dock');
      const origin = window.__nowPlayingOrigin;
      return {
        activeView: shell?.dataset.activeView,
        overlayOpen: overlay?.classList.contains('is-open'),
        overlayVisible: overlay ? getComputedStyle(overlay).display !== 'none' : false,
        sidebarVisible: sidebar ? getComputedStyle(sidebar).display !== 'none' : false,
        sidebarCovered: Boolean(sidebar && overlay && overlay.contains(document.elementFromPoint(
          sidebar.getBoundingClientRect().left + Math.min(100, sidebar.getBoundingClientRect().width / 2),
          Math.min(window.innerHeight - 120, Math.max(90, sidebar.getBoundingClientRect().top + 100)),
        ))),
        workspaceInert: workspace?.inert,
        dockVisible: dock ? dock.getBoundingClientRect().height > 0 : false,
        nowPlayingLayout: document.querySelector('.now-playing-card')?.dataset.layout,
        playState: window.__volumeHarness.snapshot().isPlaying,
        originPageRetained: origin?.page === document.querySelector('.workspace .page-content'),
        listRetained: origin?.list ? origin.list === document.querySelector('.track-list-viewport') : null,
        listScrollTop: document.querySelector('.track-list-viewport')?.scrollTop ?? null,
      };
    });

    assert.equal(overlayState.overlayOpen, true, 'Now Playing overlay should open');
    assert.equal(overlayState.overlayVisible, true, 'Now Playing overlay should be visible');
    assert.equal(overlayState.activeView, await page.evaluate(() => window.__nowPlayingOrigin.view), 'opening the overlay should preserve active route');
    assert.equal(overlayState.sidebarVisible, true, 'sidebar should remain mounted under the overlay');
    assert.equal(overlayState.sidebarCovered, true, 'overlay should visually cover the sidebar');
    assert.equal(overlayState.workspaceInert, true, 'underlying workspace should be unavailable to keyboard interaction');
    assert.equal(overlayState.dockVisible, true, 'fixed playback dock should remain visible');
    assert.equal(overlayState.playState, true, 'opening the overlay must not change playback state');
    assert.equal(overlayState.nowPlayingLayout, await page.evaluate(() => localStorage.getItem('__nowPlayingLayout') ?? 'a'), 'opening the overlay must use the saved settings layout');
    assert.equal(overlayState.originPageRetained, true, 'originating page DOM should remain mounted');
    if (overlayState.listRetained !== null) {
      assert.equal(overlayState.listRetained, true, 'library virtual list DOM should remain mounted');
      assert.equal(overlayState.listScrollTop, await page.evaluate(() => window.__nowPlayingOrigin.listScrollTop), 'library list scroll position should remain unchanged');
    }

    if (returnMethod === 'topbar') {
      await page.locator('.now-playing-overlay-return').click();
    } else {
      await page.locator('.dock-art').click();
    }
    await page.waitForFunction(() => !document.querySelector('.now-playing-overlay')?.classList.contains('is-open'));
    const returned = await page.evaluate(() => ({
      view: document.querySelector('.app-shell')?.getAttribute('data-active-view'),
      pageRetained: window.__nowPlayingOrigin.page === document.querySelector('.workspace .page-content'),
      listRetained: window.__nowPlayingOrigin.list
        ? window.__nowPlayingOrigin.list === document.querySelector('.track-list-viewport')
        : null,
      listScrollTop: document.querySelector('.track-list-viewport')?.scrollTop ?? null,
      lyricsTab: document.querySelector('#lyrics-tab')?.getAttribute('aria-selected') ?? null,
      workspaceInert: document.querySelector('.workspace')?.inert,
    }));
    assert.equal(returned.view, await page.evaluate(() => window.__nowPlayingOrigin.view), 'return should restore the originating route');
    assert.equal(returned.pageRetained, true, 'return should preserve the originating page DOM');
    assert.equal(returned.workspaceInert, false, 'return should re-enable the underlying workspace');
    if (returned.listRetained !== null) {
      assert.equal(returned.listRetained, true, 'return should preserve the library virtual list DOM');
      assert.equal(returned.listScrollTop, await page.evaluate(() => window.__nowPlayingOrigin.listScrollTop), 'return should preserve the library list scroll position');
    }
    assert.equal(returned.lyricsTab, await page.evaluate(() => window.__nowPlayingOrigin.selectedLyricsTab), 'return should preserve selected settings section');
  }

  await selectLayoutInSettings('a');
  await page.getByRole('button', { name: '曲庫', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.app-shell')?.getAttribute('data-active-view') === 'library');
  await rememberUnderlyingState(true);
  await openOverlay();
  const layoutA1920 = await (async () => {
    await page.setViewportSize({ width: 1920, height: 1080 });
    await page.waitForTimeout(35);
    const state = await readOverlayLayout();
    assert.equal(state.layout, 'a', 'layout A selected in Settings should appear in Now Playing');
    assert.equal(state.coverFit, 'contain', 'cover art must preserve its original aspect ratio');
    const stageAspect = state.cover.width / state.cover.height;
    const sourceAspect = state.coverSource[0] / state.coverSource[1];
    assert.ok(Math.abs(stageAspect - sourceAspect) <= 0.02, `1920x1080: cover frame ${state.cover.width}x${state.cover.height} must match ${state.coverSource[0]}x${state.coverSource[1]} artwork`);
    assert.deepEqual(state.copyChildren, ['now-playing-format', 'h2', 'now-playing-artist', 'now-playing-album'], 'track copy should show format, title, artist and album');
    assert.ok(state.formatText.includes('FLAC 48 kHz 24-bit'), 'format summary should use the shared audio format formatter');
    assert.equal(state.hiResBadge?.alt, 'Hi-Res', 'Hi-Res badge should have accessible alternative text');
    assert.ok(state.hiResBadge?.complete && state.hiResBadge.naturalWidth > 0, 'Hi-Res badge image should load for qualifying source depth');
    assert.ok(state.copy.top >= state.cover.bottom, 'title and artist must be below the cover');
    assert.ok(state.cover.width > 628, `1920x1080: cover should use more space than the previous 628px layout (${state.cover.width}px)`);
    assert.ok(state.lyricsViewport.height >= 64 && ['auto', 'scroll'].includes(state.lyricsOverflow), 'lyrics must keep an independently scrollable viewport');
    assert.equal(state.pageOverflow, 'hidden', 'the Now Playing page itself must not scroll');
    assert.ok(state.return.left >= 0 && state.return.right <= 1920 && state.topbar.bottom <= state.dock.top, 'return control must fit above the dock');
    assert.ok(state.dock.bottom <= 1080, 'bottom playback controls must fit in the viewport');
    return state;
  })();
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-layout-a-1920x1080.png' });
  await verifyAndRestore('topbar');

  await selectLayoutInSettings('b');
  await rememberUnderlyingState();
  await openOverlay();
  await page.setViewportSize({ width: 1366, height: 768 });
  await page.waitForTimeout(35);
  const layoutB1366 = await readOverlayLayout();
  assert.equal(layoutB1366.layout, 'b', 'layout B selected in Settings should appear in Now Playing');
  assert.deepEqual(layoutB1366.panes, ['lyrics', 'artwork'], 'layout B should place lyrics before cover');
  assert.ok(layoutB1366.cover.width > 367, `1366x768: cover should use more space than the previous 367px layout (${layoutB1366.cover.width}px)`);
  assert.ok(layoutB1366.copy.top >= layoutB1366.cover.bottom, 'layout B title and artist must stay below the cover');
  assert.ok(layoutB1366.lyricsViewport.height >= 64 && ['auto', 'scroll'].includes(layoutB1366.lyricsOverflow), 'layout B lyrics must keep an independently scrollable viewport');
  assert.equal(layoutB1366.pageOverflow, 'hidden', 'layout B Now Playing page itself must not scroll');
  assert.ok(layoutB1366.return.left >= 0 && layoutB1366.return.right <= 1366 && layoutB1366.topbar.bottom <= layoutB1366.dock.top, 'layout B return control must fit above the dock');
  assert.ok(layoutB1366.dock.bottom <= 768, 'layout B bottom playback controls must fit in the viewport');
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-layout-b-1366x768.png' });
  await verifyAndRestore('dock');

  await page.locator('.playlist-tree-open').click();
  await page.waitForFunction(() => document.querySelector('.app-shell')?.getAttribute('data-active-view') === 'playlists');
  await rememberUnderlyingState();
  await openOverlay();
  await verifyAndRestore('dock');

  await page.getByRole('button', { name: /播放佇列/ }).click();
  await page.waitForFunction(() => document.querySelector('.app-shell')?.getAttribute('data-active-view') === 'queue');
  await rememberUnderlyingState();
  await openOverlay();
  await verifyAndRestore('topbar');

  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('.app-shell')?.getAttribute('data-active-view') === 'settings');
  await page.getByRole('tab', { name: '歌詞' }).click();
  await rememberUnderlyingState();
  await openOverlay();
  await verifyAndRestore('dock');

  return {
    result: 'PASS',
    routes: ['library', 'playlists', 'queue', 'settings'],
    returnControls: ['top-left', 'dock-artwork'],
    layoutSelectedInSettings: ['a', 'b'],
    measurements: {
      '1920x1080-A': { cover: [layoutA1920.cover.width, layoutA1920.cover.height], artwork: [layoutA1920.artwork.width, layoutA1920.artwork.height] },
      '1366x768-B': { cover: [layoutB1366.cover.width, layoutB1366.cover.height], artwork: [layoutB1366.artwork.width, layoutB1366.artwork.height] },
    },
    screenshots: ['target/now-playing-layout-a-1920x1080.png', 'target/now-playing-layout-b-1366x768.png'],
    libraryScrollPreserved: true,
  };
}
