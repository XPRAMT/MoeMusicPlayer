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
  await page.waitForSelector('.page-title');
  const libraryTitle = await page.locator('#page-heading').innerText();
  assert(libraryTitle === '我的曲庫', 'library top bar title: ' + libraryTitle);
  assert(await page.locator('.track-list-count').count() === 0, 'library still shows the bottom range label');
  await page.waitForSelector('.page-title-count');
  const libraryCount = await page.locator('.page-title-count').innerText();
  assert(/^\d[\d,]* 首曲目$/.test(libraryCount), 'library top-bar track count: ' + libraryCount);
  assert(await page.locator('.page-scroll.is-list-page').count() === 1, 'library missing list-page scroll class');
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
  assert(brand.includes('MOE') && brand.includes('Music Otaku Elite'), 'brand slogan: ' + brand);
  const brandSrc = await page.locator('.brand-mark img').getAttribute('src');
  assert(Boolean(brandSrc && brandSrc.includes('SilverWolfIcon')), 'brand icon source: ' + brandSrc);

  assert(await page.locator('.library-section .section-heading-toolbar').count() === 0, 'library still has a nested toolbar heading');
  assert(await page.locator('.library-toolbar .outline-button').count() === 1, 'library refresh is not in the search toolbar');

  await page.getByRole('button', { name: '播放佇列' }).click();
  assert(await page.locator('#page-heading').innerText() === '播放佇列', 'queue title missing');
  assert(await page.getByRole('heading', { name: '播放佇列', exact: true }).count() === 1, 'queue title is duplicated');
  assert(await page.locator('.track-list-count').count() === 0, 'queue still shows the bottom range label');
  await page.waitForSelector('.page-title-count');
  const queueCount = await page.locator('.page-title-count').innerText();
  assert(/^\d[\d,]* 首曲目$/.test(queueCount), 'queue top-bar track count: ' + queueCount);
  await assertSingleListScroll('queue', false);

  await page.locator('.playlist-tree-open').click();
  assert(await page.locator('#page-heading').innerText() === '我的播放清單', 'playlist title missing');
  assert(await page.locator('.playlist-section .section-heading-toolbar').count() === 0, 'playlist still has a nested import toolbar');
  assert(await page.getByRole('button', { name: '匯入 M3U/M3U8' }).count() === 1, 'playlist import action missing');
  const playlistChrome = await page.evaluate(() => {
    const detail = document.querySelector('.playlist-detail');
    if (!detail) return { hasDetail: false };
    const style = getComputedStyle(detail);
    return {
      hasDetail: true,
      borderTopWidth: style.borderTopWidth,
      backgroundImage: style.backgroundImage,
      backgroundColor: style.backgroundColor,
      paddingTop: style.paddingTop,
    };
  });
  if (playlistChrome.hasDetail) {
    assert(playlistChrome.borderTopWidth === '0px', 'playlist detail still has a nested border: ' + JSON.stringify(playlistChrome));
    assert(playlistChrome.paddingTop === '0px', 'playlist detail still has nested padding: ' + JSON.stringify(playlistChrome));
  }

  const brandBox = await page.locator('.brand-mark img').boundingBox();
  assert(brandBox && brandBox.width >= 47 && brandBox.height >= 47, 'brand icon size: ' + JSON.stringify(brandBox));
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
  assert(await page.getByRole('button', { name: '選擇背景圖片' }).count() === 1, 'background picker missing');
  assert(await page.getByRole('slider', { name: '主介面背景模糊程度' }).count() === 1, 'background blur missing');
  assert(await page.getByRole('slider', { name: '主介面背景透明度' }).count() === 1, 'background opacity missing');
  await page.getByRole('tab', { name: '音樂來源' }).click();
  assert(await page.getByRole('heading', { name: '管理音樂來源', exact: true }).count() === 1, 'sources heading missing');
  assert(await page.locator('.section-kicker').count() === 0, 'english kicker remains in sources');
  return { result: 'PASS', libraryTitle, libraryCount, queueCount };
}
