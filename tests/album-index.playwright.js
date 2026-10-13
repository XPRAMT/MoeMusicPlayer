async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 900, height: 720 });
  await page.goto(`http://127.0.0.1:1450/tests/album-index-harness.html?run=${Date.now()}`, { waitUntil: 'commit', timeout: 30000 });
  await page.locator('.album-grid-viewport').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('[data-testid="album-count"]')?.textContent?.includes('80'));
  await page.waitForFunction(() => document.querySelector('.album-grid-viewport')?.getAttribute('data-scroll-restored') === 'true');
  await page.locator('.album-card').first().waitFor();
  const mosaicLayout = await page.evaluate(() => Object.fromEntries([1, 2, 3, 4].map((count) => {
    const images = [...document.querySelectorAll(`[data-testid="mosaic-${['zero', 'one', 'two', 'three', 'four'][count]}"] img`)];
    return [count, images.map((image) => { const rect = image.getBoundingClientRect(); return { width: rect.width, height: rect.height }; })];
  })));
  assert(mosaicLayout[1].length === 1 && Math.abs(mosaicLayout[1][0].width - mosaicLayout[1][0].height) < 0.1, `one-cover layout should fill the square: ${JSON.stringify(mosaicLayout[1])}`);
  assert(mosaicLayout[2].length === 2 && mosaicLayout[2].every((image) => Math.abs(image.width - 84) < 0.1 && Math.abs(image.height - 168) < 0.1), `two covers should use vertical strips: ${JSON.stringify(mosaicLayout[2])}`);
  assert(mosaicLayout[3].length === 3 && mosaicLayout[3].every((image) => Math.abs(image.width - 56) < 0.1 && Math.abs(image.height - 168) < 0.1), `three covers should use vertical strips: ${JSON.stringify(mosaicLayout[3])}`);
  assert(mosaicLayout[4].length === 4 && mosaicLayout[4].every((image) => Math.abs(image.width - 84) < 0.1 && Math.abs(image.height - 84) < 0.1), `four covers should use a two-by-two grid: ${JSON.stringify(mosaicLayout[4])}`);
  await page.waitForFunction(() => window.__albumIndexStats.trackCacheReads + window.__albumIndexStats.legacyCacheReads > 0);
  const loadingStats = await page.evaluate(() => ({ ...window.__albumIndexStats }));
  assert(loadingStats.trackCacheReads > 0, `cached covers should use the per-track fast path: ${JSON.stringify(loadingStats)}`);
  assert(loadingStats.rawArtworkReads === 0, `a thumbnail cache hit must not transfer original artwork bytes: ${JSON.stringify(loadingStats)}`);
  assert(loadingStats.legacyCacheReads === 0, `thumbnail file I/O must use the asynchronous cache command: ${JSON.stringify(loadingStats)}`);

  const initial = await page.evaluate(() => {
    const viewport = document.querySelector('.album-grid-viewport');
    const card = document.querySelector('.album-card');
    const art = document.querySelector('.album-card-art');
    const cardRect = card.getBoundingClientRect();
    const artRect = art.getBoundingClientRect();
    return {
      viewportWidth: viewport.clientWidth,
      cardWidth: cardRect.width,
      cardHeight: cardRect.height,
      artWidth: artRect.width,
      artHeight: artRect.height,
      columns: getComputedStyle(document.querySelector('.album-grid')).gridTemplateColumns,
    };
  });
  assert(initial.cardWidth <= 220, `first card should retain its fixed width before any scroll: ${JSON.stringify(initial)}`);
  assert(Math.abs(initial.cardWidth - 168) <= 0.1, `album cards should use their fixed 168 CSS px width: ${JSON.stringify(initial)}`);
  assert(Math.abs(initial.artWidth - initial.artHeight) <= 1, `first card artwork should be square before any scroll: ${JSON.stringify(initial)}`);
  assert(initial.columns.split(' ').length === 3, `initial column count should be derived from the content width: ${JSON.stringify(initial)}`);

  await page.locator('[data-testid="workspace"]').evaluate((element) => { element.style.width = '900px'; });
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.album-grid')).gridTemplateColumns.split(' ').length === 4);
  const afterResize = await page.locator('.album-card').first().evaluate((element) => element.getBoundingClientRect().width);
  assert(Math.abs(afterResize - initial.cardWidth) <= 0.1, `resizing should change columns without changing the fixed card width: ${initial.cardWidth} -> ${afterResize}`);

  await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 300;
    element.dispatchEvent(new Event('scroll'));
  });
  assert(await page.locator('.album-grid-viewport').evaluate((element) => element.scrollTop) === 300, 'album grid should remain scrollable while thumbnail cache requests are pending');
  await page.waitForTimeout(50);
  const afterScroll = await page.locator('.album-card-art').first().evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { width: rect.width, height: rect.height };
  });
  assert(Math.abs(afterScroll.width - afterScroll.height) <= 1, `card artwork should remain square after scroll: ${JSON.stringify(afterScroll)}`);
  assert(Math.abs(afterResize - afterScroll.width) <= 1, `scrolling must not change card size: ${afterResize} -> ${afterScroll.width}`);

  const firstCard = page.getByRole('button', { name: '開啟專輯「Album 1」，4 首曲目' });
  await firstCard.locator('.album-card-art img').first().waitFor();
  await page.waitForFunction(() => document.querySelectorAll('.album-card:first-of-type .album-card-art img').length > 0);
  const firstAlbumReads = await page.evaluate(() => [1, 2, 3, 4].reduce((sum, index) => sum + (window.__albumIndexStats.trackCacheReadsByTrack[`track-Album-1-${index}`] ?? 0), 0));
  assert(firstAlbumReads === 1, `the homepage should initially load just the first cover, not four serial thumbnail requests: ${firstAlbumReads}`);
  const noCoverCard = page.getByRole('button', { name: '開啟專輯「Album 2」，1 首曲目' });
  const noCoverImage = noCoverCard.locator('.album-card-art img');
  await noCoverImage.waitFor();
  const fallbackImage = await noCoverImage.evaluate((image) => ({ source: image.getAttribute('src'), width: image.naturalWidth }));
  assert(fallbackImage.source?.toLowerCase().includes('defaultcover') && fallbackImage.width > 0,
    `albums without artwork should display the bundled default cover: ${JSON.stringify(fallbackImage)}`);
  const noFrame = await page.locator('.album-card-art').first().evaluate((element) => getComputedStyle(element).borderWidth);
  assert(noFrame === '0px', `album cover thumbnails should not have a border: ${noFrame}`);
  await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 3000;
    element.dispatchEvent(new Event('scroll'));
  });
  await firstCard.waitFor({ state: 'detached' });
  await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await firstCard.locator('.album-card-art img').first().waitFor();
  const firstAlbumReadsAfterRemount = await page.evaluate(() => [1, 2, 3, 4].reduce((sum, index) => sum + (window.__albumIndexStats.trackCacheReadsByTrack[`track-Album-1-${index}`] ?? 0), 0));
  assert(firstAlbumReadsAfterRemount === firstAlbumReads, `returning to a virtualized row must reuse the in-memory cover: ${firstAlbumReads} -> ${firstAlbumReadsAfterRemount}`);

  await firstCard.click();
  await page.waitForFunction(() => {
    const summary = document.querySelector('.album-detail-summary')?.textContent ?? '';
    return !summary.includes('包含的演出者') && summary.includes('hanser') && summary.includes('yousa') && summary.includes('other artist');
  });
  await page.getByText('4 首曲目', { exact: true }).waitFor();
  assert(await page.getByText(/張代表封面/).count() === 0, 'album detail should no longer display representative-cover count text');
  const detailFrame = await page.locator('.album-detail-art').evaluate((element) => getComputedStyle(element).borderWidth);
  assert(detailFrame === '0px', `album detail artwork should not have a border: ${detailFrame}`);
  await page.waitForFunction(() => [...window.__albumArtworkSelections.values()].some((selection) => selection.representative));
  const rawReadsAfterFirstDetail = await page.evaluate(() => window.__albumIndexStats.rawArtworkReads);
  assert(rawReadsAfterFirstDetail === 4, `first album-detail visit should hash each source cover once: ${rawReadsAfterFirstDetail}`);
  await page.getByTestId('album-detail-back').click();
  await page.waitForFunction(() => document.querySelectorAll('.album-card:first-of-type .album-card-art img').length === 1);
  assert(await page.locator('.album-card:first-of-type .album-card-art img').count() === 1, 'the homepage must reuse the detail page representative selection; identical covers should remain one tile');
  await firstCard.click();
  await page.getByText('4 首曲目', { exact: true }).waitFor();
  await page.locator('.album-detail-art img').waitFor();
  const rawReadsAfterCachedDetail = await page.evaluate(() => window.__albumIndexStats.rawArtworkReads);
  assert(rawReadsAfterCachedDetail === rawReadsAfterFirstDetail, `saved representative-cover selection should prevent rereading large artwork: ${rawReadsAfterFirstDetail} -> ${rawReadsAfterCachedDetail}`);

  await page.getByTestId('album-detail-back').click();
  await page.locator('.album-grid-viewport').waitFor({ state: 'visible' });
  const restoredTarget = await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 900;
    element.dispatchEvent(new Event('scroll'));
    return element.scrollTop;
  });
  assert(restoredTarget > 0, `test must scroll the album homepage before navigation: ${restoredTarget}`);
  await page.getByTestId('leave-albums').click();
  await page.locator('.album-grid-viewport').waitFor({ state: 'detached' });
  await page.getByTestId('enter-albums').click();
  await page.locator('.album-grid-viewport').waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('.album-grid-viewport')?.getAttribute('data-scroll-restored') === 'true');
  await page.locator('.album-card').first().waitFor();
  await page.waitForTimeout(100);
  const restoredAfterNavigation = await page.locator('.album-grid-viewport').evaluate((element) => element.scrollTop);
  assert(Math.abs(restoredAfterNavigation - restoredTarget) <= 1,
    `leaving and reopening the album homepage should restore scrollTop: ${restoredTarget} -> ${restoredAfterNavigation}`);

  return { loadingStats, mosaicLayout, initial, afterResize, afterScroll, firstAlbumReadsAfterRemount, rawReadsAfterCachedDetail, restoredTarget, restoredAfterNavigation };
}
