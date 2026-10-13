async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://127.0.0.1:4173/tests/playback-queue-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => Boolean(window.playbackQueueHarness));
  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-live'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('現場演出'));

  const viewport = page.locator('.playlist-detail .paged-virtual-viewport');
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '1285');
  await viewport.evaluate(element => { element.scrollTop = 2200; });
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.scrollTop === 2200);

  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-hanser'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('Hanser 精選'));
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '3');
  const firstVisitTop = await viewport.evaluate(element => element.scrollTop);

  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-live'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('現場演出'));
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '1285');
  const restoredTop = await viewport.evaluate(element => element.scrollTop);
  assert(firstVisitTop === 0, `first visit to another playlist should start at top, got ${firstVisitTop}`);
  assert(Math.abs(restoredTop - 2200) <= 1, `switching back should restore playlist position 2200, got ${restoredTop}`);
  return { firstVisitTop, restoredTop };
}
