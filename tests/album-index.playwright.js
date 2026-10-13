async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 900, height: 720 });
  await page.goto('http://127.0.0.1:1450/tests/album-index-harness.html', { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.getByText('Album 1', { exact: true }).waitFor();
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

  const firstCard = page.getByRole('button', { name: /Album 1/ });
  await firstCard.locator('img').waitFor();
  const firstAlbumReads = await page.evaluate(() => window.__albumIndexStats.trackCacheReadsByTrack['track-Album-1'] ?? 0);
  assert(firstAlbumReads === 1, `the first card should resolve its thumbnail once: ${firstAlbumReads}`);
  await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 3000;
    element.dispatchEvent(new Event('scroll'));
  });
  await firstCard.waitFor({ state: 'detached' });
  await page.locator('.album-grid-viewport').evaluate((element) => {
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await firstCard.locator('img').waitFor();
  const firstAlbumReadsAfterRemount = await page.evaluate(() => window.__albumIndexStats.trackCacheReadsByTrack['track-Album-1'] ?? 0);
  assert(firstAlbumReadsAfterRemount === firstAlbumReads, `returning to a virtualized row must reuse the in-memory cover: ${firstAlbumReads} -> ${firstAlbumReadsAfterRemount}`);

  return { loadingStats, initial, afterResize, afterScroll, firstAlbumReadsAfterRemount };
}
