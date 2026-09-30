async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
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
    await page.locator('.now-playing-layout-switch').waitFor({ state: 'visible' });
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

  await rememberUnderlyingState(true);
  await openOverlay();
  await verifyAndRestore('topbar');
  await openOverlay();
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

  return { result: 'PASS', routes: ['library', 'playlists', 'queue', 'settings'], returnControls: ['top-left', 'dock-artwork'], libraryScrollPreserved: true };
}
