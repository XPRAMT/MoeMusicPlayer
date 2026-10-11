async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const assertSingleListScroll = async (label, requireViewport = true) => {
    const geometry = await page.evaluate(() => {
      const pageScroll = document.querySelector('.page-scroll');
      const viewport = document.querySelector('.paged-virtual-viewport');
      if (!pageScroll) return null;
      return {
        hasViewport: Boolean(viewport),
        pageScrollOverflowY: getComputedStyle(pageScroll).overflowY,
        pageCanScroll: pageScroll.scrollHeight > pageScroll.clientHeight + 1,
        viewportCanScroll: viewport ? viewport.scrollHeight > viewport.clientHeight + 1 : false,
      };
    });
    assert(geometry !== null, label + ' missing page-scroll');
    assert(geometry.pageScrollOverflowY === 'hidden', label + ' page-scroll overflow: ' + geometry.pageScrollOverflowY);
    assert(!geometry.pageCanScroll, label + ' still has outer page scroll: ' + JSON.stringify(geometry));
    if (requireViewport) assert(geometry.hasViewport, label + ' missing list viewport');
  };

  await page.setViewportSize({ width: 1280, height: 800 });
  page.on('pageerror', (error) => { throw error; });
  await page.goto('http://127.0.0.1:1453/tests/volume-slider-harness.html', { waitUntil: 'domcontentloaded' });
  await page.evaluate(() => localStorage.removeItem('__platform'));
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.page-title');
  const libraryTitle = await page.locator('#page-heading').innerText();
  assert(libraryTitle === '我的曲庫', 'library top bar title: ' + libraryTitle);
  assert(await page.locator('.track-list-count').count() === 0, 'library still shows the bottom range label');
  await page.waitForSelector('.page-title-count');
  const libraryCount = await page.locator('.page-title-count').innerText();
  assert(/^\d[\d,]* 首曲目$/.test(libraryCount), 'library top-bar track count: ' + libraryCount);
  assert(await page.locator('.page-scroll.is-list-page').count() === 1, 'library missing list-page scroll class');
  const listFlush = await page.evaluate(() => {
    const workspace = document.querySelector('.workspace');
    const frame = document.querySelector('.paged-virtual-frame.fill-available');
    if (!workspace || !frame) return null;
    const workspaceBox = workspace.getBoundingClientRect();
    const frameBox = frame.getBoundingClientRect();
    return {
      leftDelta: Math.round(frameBox.left - workspaceBox.left),
      rightDelta: Math.round(workspaceBox.right - frameBox.right),
    };
  });
  assert(listFlush && Math.abs(listFlush.leftDelta) <= 1 && Math.abs(listFlush.rightDelta) <= 1, 'library list is not edge-flush: ' + JSON.stringify(listFlush));
  await page.waitForSelector('.paged-virtual-viewport');
  await assertSingleListScroll('library');
  assert(/^MoeMusicPlayer \d+\.\d+\.\d+$/.test(await page.title()), 'window title is not the build date: ' + await page.title());
  assert(await page.locator('.section-kicker').count() === 0, 'english section kickers remain on the library page');
  assert(await page.getByRole('heading', { name: '我的曲庫', exact: true }).count() === 1, 'library title is duplicated');
  const sidebar = await page.locator('.sidebar').innerText();
  for (const removed of ['你的音樂空間', '本機曲庫', 'LOCAL FIRST', 'MoeMusicPlayer']) {
    assert(!sidebar.includes(removed), 'sidebar still shows ' + removed);
  }
  assert(await page.locator('[data-testid="sync-progress-banner"]').count() === 0, 'sync banner is on the library page');
  const brand = await page.locator('.brand-lockup').innerText();
  assert(brand.includes('MOE') && brand.includes('Music Otaku Edge'), 'brand slogan: ' + brand);
  const brandSrc = await page.locator('.brand-mark img').getAttribute('src');
  assert(Boolean(brandSrc && brandSrc.includes('SilverWolfIcon')), 'brand icon source: ' + brandSrc);

  assert(await page.locator('.library-section .section-heading-toolbar').count() === 0, 'library still has a nested toolbar heading');
  assert(await page.locator('.library-toolbar .outline-button').count() === 0, 'library search toolbar still has a refresh button');
  assert(await page.locator('.topbar-refresh-button').count() === 1, 'top-bar refresh control missing');

  await page.getByRole('button', { name: '播放佇列' }).click();
  assert(await page.locator('#page-heading').innerText() === '播放佇列', 'queue title missing');
  assert(await page.getByRole('heading', { name: '播放佇列', exact: true }).count() === 1, 'queue title is duplicated');
  assert(await page.locator('.track-list-count').count() === 0, 'queue still shows the bottom range label');
  await page.waitForSelector('.page-title-count');
  const queueCount = await page.locator('.page-title-count').innerText();
  assert(/^\d[\d,]* 首曲目$/.test(queueCount), 'queue top-bar track count: ' + queueCount);
  await assertSingleListScroll('queue', false);

  await page.locator('.playlist-tree-open').click();
  await page.waitForFunction(() => {
    const heading = document.querySelector('#page-heading');
    return heading && heading.textContent === '我的播放清單．hanser';
  });
  assert(await page.locator('#page-heading').innerText() === '我的播放清單．hanser', 'playlist title with name missing');
  assert(await page.locator('.playlist-section .section-heading-toolbar').count() === 0, 'playlist still has a nested import toolbar');
  assert(await page.getByRole('button', { name: '匯入 M3U/M3U8' }).count() === 0, 'playlist page still shows import');
  assert(await page.getByRole('button', { name: /匯出 M3U/i }).count() === 0, 'playlist page still shows export');
  const playlistChrome = await page.evaluate(() => {
    const page = document.querySelector('.page-content.wide-list-page');
    const detail = document.querySelector('.playlist-detail');
    if (!page) return { hasPage: false };
    const pageStyle = getComputedStyle(page);
    if (!detail) {
      return { hasPage: true, hasDetail: false, pagePaddingTop: pageStyle.paddingTop };
    }
    const style = getComputedStyle(detail);
    return {
      hasPage: true,
      hasDetail: true,
      pagePaddingTop: pageStyle.paddingTop,
      borderTopWidth: style.borderTopWidth,
      backgroundImage: style.backgroundImage,
      backgroundColor: style.backgroundColor,
      paddingTop: style.paddingTop,
    };
  });
  assert(playlistChrome.hasPage, 'playlist wide list page missing');
  assert(playlistChrome.pagePaddingTop === '0px', 'playlist page still has top padding: ' + JSON.stringify(playlistChrome));
  if (playlistChrome.hasDetail) {
    assert(playlistChrome.borderTopWidth === '0px', 'playlist detail still has a nested border: ' + JSON.stringify(playlistChrome));
    assert(playlistChrome.paddingTop === '0px', 'playlist detail still has nested padding: ' + JSON.stringify(playlistChrome));
  }

  const brandBox = await page.locator('.brand-mark img').boundingBox();
  assert(brandBox && brandBox.width >= 63 && brandBox.height >= 63, 'brand icon size: ' + JSON.stringify(brandBox));
  const brandFit = await page.evaluate(() => {
    const sidebar = document.querySelector('.sidebar').getBoundingClientRect();
    const lockup = document.querySelector('.brand-lockup').getBoundingClientRect();
    return { sidebarRight: Math.round(sidebar.right), lockupRight: Math.round(lockup.right) };
  });
  assert(brandFit.lockupRight <= brandFit.sidebarRight + 1, 'brand overflows the sidebar: ' + JSON.stringify(brandFit));

  await page.getByRole('button', { name: '設定', exact: true }).click();
  assert(await page.locator('#page-heading').innerText() === '設定', 'settings title missing');
  assert(await page.locator('.page-title-count').count() === 0, 'settings still shows track count');
  assert(await page.locator('.page-scroll.is-list-page').count() === 0, 'settings kept list-page scroll class');
  assert(await page.getByRole('tab', { name: '主介面', exact: true }).count() === 1, 'main interface settings tab missing');
  const gaps = await page.locator('.main-interface-panel > *').evaluateAll((nodes) => {
    const boxes = nodes.map((node) => node.getBoundingClientRect()).filter((box) => box.height > 0);
    const spaces = [];
    for (let index = 1; index < boxes.length; index += 1) spaces.push(Math.round(boxes[index].top - boxes[index - 1].bottom));
    return spaces;
  });
  assert(gaps.length >= 4 && gaps.every((gap) => gap === 8), 'main interface settings spacing: ' + JSON.stringify(gaps));
  const modeGroup = page.getByRole('group', { name: '主介面背景模式' });
  assert(await modeGroup.getByRole('button').count() === 5, 'main background mode count');
  assert(await page.getByRole('button', { name: '純色', exact: true }).getAttribute('aria-pressed') === 'true', 'solid should be selected by default');
  assert(await page.getByRole('slider', { name: '主介面背景模糊程度' }).count() === 0, 'solid mode must not show blur');
  assert(await page.getByRole('slider', { name: '主介面背景透明度' }).count() === 0, 'solid mode must not show opacity');
  await page.getByRole('button', { name: '單圖', exact: true }).click();
  assert(await page.getByRole('button', { name: '選擇圖片' }).count() === 1, 'single-image picker missing');
  assert(await page.getByRole('slider', { name: '主介面背景模糊程度' }).count() === 1, 'single-image blur missing');
  assert(await page.getByRole('slider', { name: '主介面背景透明度' }).count() === 1, 'single-image opacity missing');
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'single-image mode must not show background zoom');
  await page.getByRole('button', { name: '幻燈片', exact: true }).click();
  assert(await page.getByRole('button', { name: '選擇資料夾' }).count() === 1, 'slideshow folder picker missing');
  const slideshowOrder = page.getByRole('group', { name: '幻燈片播放順序' });
  assert(await slideshowOrder.getByRole('button', { name: '順序播放' }).getAttribute('aria-pressed') === 'true', 'sequential should be the default order');
  assert(await slideshowOrder.getByRole('button', { name: '隨機播放' }).count() === 1, 'random slideshow choice missing');
  assert(await page.getByRole('slider', { name: '幻燈片切換時間' }).count() === 1, 'slideshow interval missing');
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'slideshow mode must not show background zoom');
  await page.getByRole('button', { name: '專輯封面', exact: true }).click();
  assert((await page.getByText(/沒有可用封面時暫時顯示純色/).count()) === 1, 'album-cover fallback explanation missing');
  await page.waitForSelector('.main-backdrop img');
  assert(await page.locator('.main-backdrop img').count() === 1, 'album cover is not used as the main-window background');
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'album-cover mode must not show background zoom');
  const mainBlurSlider = page.getByRole('slider', { name: '主介面背景模糊程度' });
  await mainBlurSlider.evaluate((input) => {
    input.value = '0';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const zeroBlurGeometry = await page.locator('.main-backdrop img').evaluate((image) => {
    const imageBox = image.getBoundingClientRect();
    return {
      transform: getComputedStyle(image).transform,
      imageBox: { left: imageBox.left, top: imageBox.top, width: imageBox.width, height: imageBox.height },
      viewport: { width: innerWidth, height: innerHeight },
    };
  });
  assert(zeroBlurGeometry.transform === 'none', 'background image must not apply a separate zoom transform: ' + JSON.stringify(zeroBlurGeometry));
  assert(Math.abs(zeroBlurGeometry.imageBox.left) < 0.1
    && Math.abs(zeroBlurGeometry.imageBox.top) < 0.1
    && Math.abs(zeroBlurGeometry.imageBox.width - zeroBlurGeometry.viewport.width) < 0.1
    && Math.abs(zeroBlurGeometry.imageBox.height - zeroBlurGeometry.viewport.height) < 0.1,
  'zero blur should size the background image exactly to the viewport: ' + JSON.stringify(zeroBlurGeometry));
  await mainBlurSlider.evaluate((input) => {
    input.value = '10';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const tenPixelBlurGeometry = await page.locator('.main-backdrop img').evaluate((image) => {
    const imageBox = image.getBoundingClientRect();
    return {
      imageBox: { left: imageBox.left, top: imageBox.top, width: imageBox.width, height: imageBox.height },
      viewport: { width: innerWidth, height: innerHeight },
    };
  });
  assert(Math.abs(tenPixelBlurGeometry.imageBox.left + 10) < 0.1
    && Math.abs(tenPixelBlurGeometry.imageBox.top + 10) < 0.1
    && Math.abs(tenPixelBlurGeometry.imageBox.width - tenPixelBlurGeometry.viewport.width - 20) < 0.1
    && Math.abs(tenPixelBlurGeometry.imageBox.height - tenPixelBlurGeometry.viewport.height - 20) < 0.1,
  '10px blur should extend the image by 10px on each edge: ' + JSON.stringify(tenPixelBlurGeometry));
  await mainBlurSlider.evaluate((input) => {
    input.value = '20';
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'background zoom control should be removed');
  const backgroundLabels = await page.locator('.main-interface-panel .lyrics-preference-range').evaluateAll((labels) => labels.map((label) => label.textContent));
  const opacityPosition = backgroundLabels.findIndex((text) => text.includes('背景透明度'));
  assert(opacityPosition >= 0 && !backgroundLabels.some((text) => text.includes('背景縮放')), 'background zoom control remains in settings: ' + JSON.stringify(backgroundLabels));
  const mainBackdropStyle = await page.locator('.main-backdrop img').evaluate((image) => {
    const style = getComputedStyle(image);
    return {
      transform: style.transform,
      objectFit: style.objectFit,
      src: image.getAttribute('src'),
      naturalWidth: image.naturalWidth,
      naturalHeight: image.naturalHeight,
      box: (() => {
        const rect = image.getBoundingClientRect();
        return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
      })(),
      container: (() => {
        const box = image.parentElement.getBoundingClientRect();
        return { width: box.width, height: box.height };
      })(),
    };
  });
  await page.getByRole('button', { name: /開啟正在播放/ }).click();
  await page.waitForSelector('.now-playing-backdrop img');
  const nowPlayingBackdropStyle = await page.locator('.now-playing-backdrop img').evaluate((image) => {
    const style = getComputedStyle(image);
    return {
      transform: style.transform,
      objectFit: style.objectFit,
      src: image.getAttribute('src'),
      naturalWidth: image.naturalWidth,
      naturalHeight: image.naturalHeight,
      box: (() => {
        const rect = image.getBoundingClientRect();
        return { left: rect.left, top: rect.top, width: rect.width, height: rect.height };
      })(),
      container: (() => {
        const box = image.parentElement.getBoundingClientRect();
        return { width: box.width, height: box.height };
      })(),
    };
  });
  assert(mainBackdropStyle.transform === 'none' && nowPlayingBackdropStyle.transform === 'none', 'backgrounds must use blur compensation without a separate zoom transform: ' + JSON.stringify({ mainBackdropStyle, nowPlayingBackdropStyle }));
  assert(JSON.stringify(mainBackdropStyle.box) === JSON.stringify(nowPlayingBackdropStyle.box), 'main album-cover and Now Playing artwork must use the same automatic sizing: ' + JSON.stringify({ mainBackdropStyle, nowPlayingBackdropStyle }));
  assert(mainBackdropStyle.objectFit === nowPlayingBackdropStyle.objectFit, 'album-cover and playback backdrops use different image fit: ' + JSON.stringify({ mainBackdropStyle, nowPlayingBackdropStyle }));
  assert(mainBackdropStyle.src === nowPlayingBackdropStyle.src, 'album-cover and playback backdrops use different artwork: ' + JSON.stringify({ mainBackdropStyle, nowPlayingBackdropStyle }));
  assert(mainBackdropStyle.naturalWidth === nowPlayingBackdropStyle.naturalWidth && mainBackdropStyle.naturalHeight === nowPlayingBackdropStyle.naturalHeight, 'album-cover and playback backdrops decoded different image sizes');
  assert(Math.abs(mainBackdropStyle.container.width - nowPlayingBackdropStyle.container.width) < 0.1 && Math.abs(mainBackdropStyle.container.height - nowPlayingBackdropStyle.container.height) < 0.1, 'album-cover and playback backdrops use different container sizes: ' + JSON.stringify({ mainBackdropStyle, nowPlayingBackdropStyle }));
  await page.locator('.now-playing-overlay-return').click();
  assert(await page.getByRole('slider', { name: '主介面背景模糊程度' }).count() === 1, 'album-cover blur missing');
  await page.getByRole('button', { name: '系統桌布', exact: true }).click();
  assert((await page.getByText(/讀取 Windows 目前的系統桌布/).count()) === 1, 'system wallpaper explanation missing');
  assert(await page.getByRole('slider', { name: '主介面背景透明度' }).count() === 1, 'system wallpaper opacity missing');
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'system wallpaper must not show background zoom');
  await page.getByRole('button', { name: '純色', exact: true }).click();
  assert(await page.getByRole('slider', { name: '快速設定面板透明度' }).count() === 0, 'quick settings opacity stays out of main interface');
  await page.getByRole('tab', { name: '播放頁', exact: true }).click();
  assert(await page.getByRole('slider', { name: '快速設定面板透明度' }).count() === 1, 'quick settings opacity missing on playback page');
  assert(await page.getByRole('slider', { name: '快速設定面板模糊程度' }).count() === 1, 'quick settings blur missing on playback page');
  assert(await page.getByRole('button', { name: '重新整理音樂來源' }).count() === 1, 'top bar refresh is always available');
  await page.getByRole('tab', { name: '音樂來源' }).click();
  assert(await page.getByRole('heading', { name: '管理音樂來源', exact: true }).count() === 1, 'sources heading missing');
  assert(await page.locator('#sources-panel').getByRole('button', { name: /重新載入|重新整理/ }).count() === 0, 'sources panel must not keep a local reload button');
  assert(await page.locator('.configured-source-state').count() === 0, 'added sources must not show sync status');
  assert(await page.locator('.runtime-pill').count() === 0, 'desktop service status pill is removed');
  assert(await page.locator('.section-kicker').count() === 0, 'english kicker remains in sources');
  await page.getByRole('tab', { name: '歌詞' }).click();
  assert(await page.getByRole('button', { name: '恢復預設' }).count() === 0, 'lyrics settings must not show reset defaults');
  await page.evaluate(() => localStorage.setItem('__platform', 'android'));
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.page-title');
  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '主介面', exact: true }).click();
  for (const mode of ['單圖', '幻燈片', '系統桌布']) {
    assert(await page.getByRole('button', { name: mode, exact: true }).isDisabled(), `Android must disable ${mode}`);
  }
  await page.getByRole('button', { name: '專輯封面', exact: true }).click();
  assert(await page.getByRole('slider', { name: '主介面背景縮放' }).count() === 0, 'Android album-cover must not show background zoom');
  assert(await page.getByText(/Android 暫不支援本機圖片選取/).count() === 1, 'Android background limitations are not explained');
  return { result: 'PASS', libraryTitle, libraryCount, queueCount };
}
