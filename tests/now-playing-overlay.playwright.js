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
  const harnessBaseUrl = page.url().split('/').slice(0, 3).join('/');
  await page.goto(`${harnessBaseUrl}/tests/volume-slider-harness.html`);
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
    assert.equal(await page.locator('#now-playing-layout-panel input[aria-label="背景圖片亮度"]').count(), 1, 'main settings exposes background brightness');
    assert.equal(await page.locator('#now-playing-layout-panel input[aria-label="元件底色透明度"]').count(), 0, 'main settings removes surface transparency');
    await page.getByRole('button', { name: layout === 'a' ? '排列 A：封面在前，歌詞在後' : '排列 B：歌詞在前，封面在後' }).click();
    await page.waitForFunction((expected) => document.querySelector('#now-playing-layout-panel .quick-settings-group [role="status"]')?.textContent?.includes(`排列 ${expected.toUpperCase()} 已保存`), layout);
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
  const trackIdBeforeQuickSettings = await page.evaluate(() => window.__volumeHarness.snapshot().currentTrack.id);
  const lyricsFetchCountBeforeQuickSettings = await page.evaluate(() => window.__volumeHarness.lyricsGetCount);
  await page.locator('.timed-lyrics-viewport, .plain-lyrics-viewport').evaluate((element) => { element.scrollTop = 96; });
  const quickSettingsBeforeOpen = await page.evaluate(() => {
    const rect = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height };
    };
    return {
      windowScroll: [window.scrollX, window.scrollY],
      pageScroll: document.scrollingElement?.scrollTop ?? null,
      lyricsScroll: document.querySelector('.timed-lyrics-viewport, .plain-lyrics-viewport')?.scrollTop ?? null,
      topbar: rect('.now-playing-overlay-topbar'),
      cover: rect('.cover-stage'),
      dock: rect('.player-dock'),
    };
  });
  await page.getByRole('button', { name: '開啟快速設定' }).click();
  const quickSettings = page.getByRole('dialog', { name: '快速設定' });
  await quickSettings.waitFor({ state: 'visible' });
  for (const label of ['封面背景模糊程度', '背景圖片亮度', '非目前歌詞透明度', '原文字級', '譯文與羅馬拼音字級', '歌詞句間距']) {
    assert.equal(await quickSettings.locator(`input[aria-label="${label}"]`).count(), 1, `drawer exposes ${label}`);
  }
  assert.equal(await page.locator('.now-playing-topbar-tools .lyrics-topbar-status span').count(), 2, 'provider and sync labels are shown in the page toolbar');
  assert.equal(await page.locator('.now-playing-overlay .lyrics-panel-heading').count(), 0, 'LyricsView has no heading');
  await page.keyboard.press('Shift+Tab');
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), '歌詞句間距', 'Shift+Tab stays inside the settings dialog');
  await page.keyboard.press('Tab');
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), '關閉快速設定', 'Tab wraps focus to the close button');
  assert.equal(await page.locator('.now-playing-overlay-body').evaluate((element) => element.inert), true, 'quick settings modal makes the covered playback view inert');
  assert.equal(await page.locator('.player-dock').evaluate((element) => element.inert), true, 'modal drawer blocks dock controls while open');
  assert.equal(await quickSettings.locator('input[aria-label="元件底色透明度"]').count(), 0, 'surface transparency is not configurable');
  const drawerGeometry = await page.evaluate(() => {
    const rect = (element) => {
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height };
    };
    const drawer = document.querySelector('.now-playing-quick-settings-drawer');
    const scrim = document.querySelector('.now-playing-quick-settings-scrim');
    const ancestors = [];
    for (let element = drawer?.parentElement; element; element = element.parentElement) {
      ancestors.push({
        name: `${element.tagName.toLowerCase()}.${typeof element.className === 'string' ? element.className : ''}`,
        scrollTop: element.scrollTop,
        scrollLeft: element.scrollLeft,
        rect: rect(element),
      });
    }
    return {
      windowScroll: [window.scrollX, window.scrollY],
      pageScroll: document.scrollingElement?.scrollTop ?? null,
      lyricsScroll: document.querySelector('.timed-lyrics-viewport, .plain-lyrics-viewport')?.scrollTop ?? null,
      ancestors,
      topbar: rect(document.querySelector('.now-playing-overlay-topbar')),
      cover: rect(document.querySelector('.cover-stage')),
      dock: rect(document.querySelector('.player-dock')),
      header: rect(document.querySelector('.quick-settings-drawer-header')),
      nav: rect(document.querySelector('.quick-settings-nav')),
      close: rect(document.querySelector('.quick-settings-drawer-header button')),
      drawer: rect(drawer),
      scrim: rect(scrim),
      scrimBackground: getComputedStyle(scrim).backgroundColor,
    };
  });
  assert.deepEqual(drawerGeometry.windowScroll, quickSettingsBeforeOpen.windowScroll, 'opening the drawer must not move the page');
  assert.equal(drawerGeometry.pageScroll, quickSettingsBeforeOpen.pageScroll, 'opening the drawer must preserve document scroll');
  assert.equal(drawerGeometry.lyricsScroll, quickSettingsBeforeOpen.lyricsScroll, 'opening the drawer must preserve the independent lyrics scroll position');
  assert.deepEqual(drawerGeometry.topbar, quickSettingsBeforeOpen.topbar, 'opening the drawer must not move the Now Playing toolbar');
  assert.deepEqual(drawerGeometry.cover, quickSettingsBeforeOpen.cover, 'opening the drawer must not move or crop the cover');
  assert.deepEqual(drawerGeometry.dock, quickSettingsBeforeOpen.dock, 'opening the drawer must not move the dock');
  assert.equal(drawerGeometry.scrimBackground, 'rgba(0, 0, 0, 0)', 'click-outside layer has no darkening overlay');
  assert.equal(drawerGeometry.scrim.x, 0, 'click-outside layer still covers the playback area from the left edge');
  assert.equal(drawerGeometry.scrim.width, 1280, 'click-outside layer still spans the playback area');
  assert.ok(drawerGeometry.drawer.x >= 0 && drawerGeometry.drawer.x + drawerGeometry.drawer.width <= 1280, 'drawer stays within the right edge');
  assert.ok(drawerGeometry.drawer.x >= drawerGeometry.scrim.x && drawerGeometry.drawer.x + drawerGeometry.drawer.width <= drawerGeometry.scrim.x + drawerGeometry.scrim.width, 'drawer remains inside the click-outside layer geometry');
  assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector('.now-playing-quick-settings-drawer')).boxShadow), 'none', 'drawer has no left-side black shadow');

  const captureDrawerScrollState = () => page.evaluate(() => {
    const rect = (element) => {
      const bounds = element.getBoundingClientRect();
      return { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height };
    };
    const drawer = document.querySelector('.now-playing-quick-settings-drawer');
    const ancestors = [];
    for (let element = drawer?.parentElement; element; element = element.parentElement) {
      ancestors.push({
        name: `${element.tagName.toLowerCase()}.${typeof element.className === 'string' ? element.className : ''}`,
        scrollTop: element.scrollTop,
        scrollLeft: element.scrollLeft,
        rect: rect(element),
      });
    }
    const scroller = drawer.querySelector('.quick-settings-drawer-scroll');
    return {
      windowScroll: [window.scrollX, window.scrollY],
      pageScroll: document.scrollingElement?.scrollTop ?? null,
      lyricsScroll: document.querySelector('.timed-lyrics-viewport, .plain-lyrics-viewport')?.scrollTop ?? null,
      ancestors,
      ownScroll: scroller.scrollTop,
      ownMaxScroll: Math.max(0, scroller.scrollHeight - scroller.clientHeight),
      topbar: rect(document.querySelector('.now-playing-overlay-topbar')),
      cover: rect(document.querySelector('.cover-stage')),
      dock: rect(document.querySelector('.player-dock')),
      header: rect(document.querySelector('.quick-settings-drawer-header')),
      nav: rect(document.querySelector('.quick-settings-nav')),
      close: rect(document.querySelector('.quick-settings-drawer-header button')),
      drawer: rect(drawer),
      drawerScroll: rect(scroller),
      lyricsHeading: rect(document.querySelector('#quick-settings-lyrics-heading')),
    };
  });
  const assertOnlyDrawerMoved = (before, after, label) => {
    for (const key of ['windowScroll', 'pageScroll', 'lyricsScroll', 'ancestors', 'topbar', 'cover', 'dock', 'header', 'nav', 'close', 'drawer', 'drawerScroll']) {
      assert.deepEqual(after[key], before[key], `${label}: ${key} must stay fixed while drawer contents scroll`);
    }
  };
  const quickSettingsScrollMeasurements = [];
  for (const viewport of [
    { width: 3840, height: 2160 },
    { width: 1920, height: 1080 },
    { width: 1366, height: 768 },
    { width: 360, height: 800 },
  ]) {
    await page.setViewportSize(viewport);
    await page.waitForTimeout(60);
    const beforeNavigation = await captureDrawerScrollState();
    await quickSettings.getByRole('button', { name: '歌詞外觀', exact: true }).click();
    await page.waitForTimeout(320);
    const afterNavigation = await captureDrawerScrollState();
    assertOnlyDrawerMoved(beforeNavigation, afterNavigation, `${viewport.width}x${viewport.height} navigation`);
    if (viewport.height <= 800) {
      assert.ok(afterNavigation.ownMaxScroll > 0, `${viewport.width}x${viewport.height}: drawer contents need an independent scroll range`);
      assert.ok(afterNavigation.ownScroll > beforeNavigation.ownScroll, `${viewport.width}x${viewport.height}: section navigation scrolls only the drawer contents`);
    }
    assert.ok(afterNavigation.lyricsHeading.y >= afterNavigation.drawerScroll.y - 1, `${viewport.width}x${viewport.height}: lyrics settings remain in their own scroll viewport`);
    assert.ok(afterNavigation.lyricsHeading.y < afterNavigation.drawerScroll.y + afterNavigation.drawerScroll.height, `${viewport.width}x${viewport.height}: lyrics section is reachable`);
    assert.ok(afterNavigation.header.y >= 0 && afterNavigation.close.y >= afterNavigation.header.y, `${viewport.width}x${viewport.height}: drawer header and close button stay visible`);
    assert.ok(afterNavigation.nav.y >= afterNavigation.header.y + afterNavigation.header.height - 1, `${viewport.width}x${viewport.height}: drawer navigation stays below its fixed header`);
    assert.ok(afterNavigation.dock.y + afterNavigation.dock.height <= viewport.height + 1, `${viewport.width}x${viewport.height}: playback dock stays visible`);

    const scrollerBounds = await quickSettings.locator('.quick-settings-drawer-scroll').boundingBox();
    assert.ok(scrollerBounds, 'drawer has a bounded scroll viewport');
    await page.mouse.move(scrollerBounds.x + scrollerBounds.width / 2, scrollerBounds.y + scrollerBounds.height / 2);
    await page.mouse.wheel(0, 12000);
    await page.waitForTimeout(120);
    const afterWheel = await captureDrawerScrollState();
    assertOnlyDrawerMoved(afterNavigation, afterWheel, `${viewport.width}x${viewport.height} wheel`);
    assert.ok(afterWheel.ownMaxScroll === 0 || afterWheel.ownScroll >= afterWheel.ownMaxScroll - 1, `${viewport.width}x${viewport.height}: wheel reaches the drawer's own end`);
    await page.mouse.wheel(0, 12000);
    await page.waitForTimeout(100);
    const afterOverscroll = await captureDrawerScrollState();
    assertOnlyDrawerMoved(afterWheel, afterOverscroll, `${viewport.width}x${viewport.height} bottom overscroll`);
    assert.ok(afterOverscroll.ownScroll === afterOverscroll.ownMaxScroll || afterOverscroll.ownMaxScroll === 0, `${viewport.width}x${viewport.height}: overscroll does not chain to the playback page`);

    await quickSettings.getByRole('button', { name: '播放頁', exact: true }).click();
    await page.waitForTimeout(320);
    const afterReturnNavigation = await captureDrawerScrollState();
    assertOnlyDrawerMoved(afterOverscroll, afterReturnNavigation, `${viewport.width}x${viewport.height} return navigation`);
    assert.equal(await page.locator('.quick-settings-drawer-header').isVisible(), true, 'drawer close controls remain visible after internal navigation');
    quickSettingsScrollMeasurements.push({
      viewport: `${viewport.width}x${viewport.height}`,
      drawer: [Math.round(afterNavigation.drawer.width), Math.round(afterNavigation.drawer.height)],
      scrollerHeight: Math.round(afterNavigation.drawerScroll.height),
      lyricsNavigationScrollTop: Math.round(afterNavigation.ownScroll),
      maximumScrollTop: Math.round(afterNavigation.ownMaxScroll),
      pageScrollTop: afterOverscroll.pageScroll,
      dockBottom: Math.round(afterOverscroll.dock.y + afterOverscroll.dock.height),
    });
  }
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-quick-settings-scroll-regression.png' });
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-quick-settings-desktop.png' });
  const drawerBounds = await quickSettings.boundingBox();
  assert.ok(drawerBounds && drawerBounds.x >= 0 && drawerBounds.width <= 1280, 'desktop drawer stays within viewport bounds');
  const brightnessSlider = quickSettings.locator('input[aria-label="背景圖片亮度"]');
  const expectedOverlayAlpha = new Map([[0, '1'], [40, '0.6'], [100, '0']]);
  for (const brightness of [0, 40, 100]) {
    await brightnessSlider.evaluate((input, value) => {
      input.value = String(value);
      input.dispatchEvent(new Event('input', { bubbles: true }));
      input.dispatchEvent(new Event('change', { bubbles: true }));
    }, brightness);
    const overlayAlpha = await page.evaluate(() => getComputedStyle(document.querySelector('.now-playing-backdrop'), '::after').opacity);
    assert.equal(overlayAlpha, expectedOverlayAlpha.get(brightness), `brightness ${brightness}% maps to the expected theme overlay alpha`);
    await page.waitForFunction((expected) => JSON.parse(localStorage.getItem('__appearancePreferences') || '{}').backgroundBrightnessPercent === expected, brightness);
  }
  assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector('.now-playing-overlay.is-open .now-playing-overlay-topbar')).backgroundColor), 'rgba(0, 0, 0, 0)', 'Now Playing topbar surface remains transparent');
  assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector('.app-shell.has-now-playing-backdrop .player-dock')).backgroundColor), 'rgba(0, 0, 0, 0)', 'Now Playing dock surface remains transparent');
  assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector('.now-playing-overlay .lyrics-toggle')).backgroundColor), 'rgba(0, 0, 0, 0)', 'Now Playing inactive lyric toggle surface remains transparent');
  await brightnessSlider.evaluate((input) => {
    input.value = '40';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__appearancePreferences') || '{}').backgroundBrightnessPercent === 40);
  await quickSettings.getByRole('button', { name: '排列 B：歌詞在前，封面在後' }).click();
  await page.waitForFunction(() => localStorage.getItem('__nowPlayingLayout') === 'b');
  const blurSlider = quickSettings.locator('input[aria-label="封面背景模糊程度"]');
  await blurSlider.evaluate((input) => {
    input.value = '25';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__appearancePreferences') || '{}').backgroundBlurPx === 25);
  await blurSlider.evaluate((input) => {
    input.value = '20';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__appearancePreferences') || '{}').backgroundBlurPx === 20);
  const drawerGap = quickSettings.locator('input[aria-label="歌詞句間距"]');
  await drawerGap.evaluate((input) => {
    input.value = '40';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').lineGapPx === 40);
  const drawerTranslation = quickSettings.getByRole('checkbox', { name: /顯示譯文/ });
  await drawerTranslation.check();
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').showTranslation === true);
  await drawerTranslation.uncheck();
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').showTranslation === false);
  await quickSettings.getByRole('button', { name: '排列 A：封面在前，歌詞在後' }).click();
  await page.waitForFunction(() => localStorage.getItem('__nowPlayingLayout') === 'a');
  await page.setViewportSize({ width: 360, height: 800 });
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-quick-settings-narrow.png' });
  const narrowDrawerBounds = await quickSettings.boundingBox();
  assert.ok(narrowDrawerBounds && narrowDrawerBounds.x >= 0 && narrowDrawerBounds.x + narrowDrawerBounds.width <= 360, 'narrow drawer stays within viewport bounds');
  await page.keyboard.press('Escape');
  await page.waitForFunction(() => !document.querySelector('[data-testid="now-playing-quick-settings"]'));
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), '開啟快速設定', 'Escape returns focus to its trigger');
  assert.equal(await page.locator('.now-playing-overlay-body').evaluate((element) => element.inert), false, 'closing drawer restores playback view interaction');
  assert.equal(await page.locator('.player-dock').evaluate((element) => element.inert), false, 'closing drawer restores dock controls');
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.getByRole('button', { name: '開啟快速設定' }).click();
  await page.waitForFunction(() => document.querySelector('[data-testid="now-playing-quick-settings"]'));
  assert.equal(await page.locator('.now-playing-quick-settings-drawer input[aria-label="背景圖片亮度"]').inputValue(), '40', 'reopened drawer restores saved brightness');
  await page.locator('.now-playing-quick-settings-scrim').click({ position: { x: 12, y: 360 } });
  await page.waitForFunction(() => !document.querySelector('[data-testid="now-playing-quick-settings"]'));
  assert.equal(await page.evaluate(() => document.activeElement?.getAttribute('aria-label')), '開啟快速設定', 'click-outside close returns focus to its trigger');
  assert.equal(await page.evaluate(() => window.__volumeHarness.snapshot().currentTrack.id), trackIdBeforeQuickSettings, 'drawer preference changes retain the current track');
  assert.equal(await page.evaluate(() => window.__volumeHarness.lyricsGetCount), lyricsFetchCountBeforeQuickSettings, 'drawer interactions do not reload or reset lyrics');
  const translationToggle = page.locator('.now-playing-topbar-tools button[aria-label="切換譯文顯示"]');
  await translationToggle.click();
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').showTranslation === true);
  await translationToggle.click();
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').showTranslation === false);
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').lineGapPx === 40);
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.waitForTimeout(50);
  await page.getByRole('button', { name: '開啟快速設定' }).click();
  await page.locator('.now-playing-quick-settings-drawer input[aria-label="歌詞句間距"]').evaluate((input) => {
    input.value = '24';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    input.dispatchEvent(new Event('change', { bubbles: true }));
  });
  await page.waitForFunction(() => JSON.parse(localStorage.getItem('__lyricsPreferences') || '{}').lineGapPx === 24);
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('.now-playing-overlay .lyrics-panel-heading').count(), 0, 'lyrics title is removed from the page');
  assert.equal(await page.locator('.now-playing-overlay .lyrics-toggle').count(), 2, 'translation controls moved to the top toolbar');
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
    quickSettingsScrollMeasurements,
    libraryScrollPreserved: true,
  };
}
