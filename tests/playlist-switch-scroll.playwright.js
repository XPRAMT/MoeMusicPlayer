async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto('http://127.0.0.1:4173/tests/playback-queue-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => Boolean(window.playbackQueueHarness));
  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-live'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('現場演出'));

  const viewport = page.locator('.playlist-detail .paged-virtual-viewport');
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '1285');
  const positionSort = page.locator('.playlist-detail .track-list-header button[aria-label="依播放清單中的順序排序"]');
  await positionSort.click();
  assert((await positionSort.innerText()).includes('▼'), 'playlist sort should toggle before navigating away');
  await viewport.evaluate(element => { element.scrollTop = 2200; element.dispatchEvent(new Event('scroll', { bubbles: true })); });
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.scrollTop === 2200);

  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-hanser'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('Hanser 精選'));
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '3');
  const firstVisitTop = await viewport.evaluate(element => element.scrollTop);
  const otherPlaylistPositionSort = page.locator('.playlist-detail .track-list-header button[aria-label="依播放清單中的順序排序"]');
  assert((await otherPlaylistPositionSort.innerText()).includes('▲'), 'each playlist should keep its own sort direction');

  await page.evaluate(() => window.playbackQueueHarness.setPlaylist('playlist-live'));
  await page.waitForFunction(() => document.querySelector('#selected-playlist-heading')?.textContent?.includes('現場演出'));
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.getAttribute('aria-rowcount') === '1285');
  const restoredTop = await viewport.evaluate(element => element.scrollTop);
  assert((await positionSort.innerText()).includes('▼'), 'switching back should restore the playlist sort direction');
  assert(firstVisitTop === 0, `first visit to another playlist should start at top, got ${firstVisitTop}`);
  assert(Math.abs(restoredTop - 2200) <= 1, `switching back should restore playlist position 2200, got ${restoredTop}`);

  const requestsBeforeNavigation = await page.evaluate(() => window.playbackQueueHarness.snapshot().playlistRequestCount);
  await page.evaluate(() => window.playbackQueueHarness.setView('queue'));
  await page.waitForSelector('.playback-queue-viewport');
  assert(await page.locator('.playlist-detail').count() === 0, 'leaving playlists should destroy the playlist list view');
  await page.evaluate(() => window.playbackQueueHarness.setView('playlists'));
  await page.waitForFunction(() => document.querySelector('.playlist-detail')?.getAttribute('aria-labelledby') === 'selected-playlist-heading');
  await page.waitForFunction(() => document.querySelector('.playlist-detail .paged-virtual-viewport')?.scrollTop === 2200);
  const restoredAfterNavigation = await viewport.evaluate(element => element.scrollTop);
  const requestsAfterNavigation = await page.evaluate(() => window.playbackQueueHarness.snapshot().playlistRequestCount);
  assert(requestsAfterNavigation > requestsBeforeNavigation, 'returning through the sidebar should reload playlist data');
  assert(Math.abs(restoredAfterNavigation - 2200) <= 1, `returning through navigation should restore playlist position 2200, got ${restoredAfterNavigation}`);
  return { firstVisitTop, restoredTop, restoredAfterNavigation, requestsBeforeNavigation, requestsAfterNavigation };
}
