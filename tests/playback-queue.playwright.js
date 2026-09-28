async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const waitForQueue = async () => page.waitForFunction(() => Boolean(window.playbackQueueHarness));

  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto('http://127.0.0.1:4173/tests/playback-queue-harness.html');
  await waitForQueue();

  const treeHost = page.locator('.sidebar-playlist-tree-host');
  assert(await treeHost.locator('.playlist-tree').count() === 1, 'playlist tree is missing from the sidebar');
  assert(await page.locator('.playlist-browser .playlist-tree').count() === 0, 'playlist page retained a second tree column');
  await page.waitForSelector('.playlist-detail');
  let state = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(state.treeHostParent?.includes('sidebar'), 'playlist tree is not a child of the primary sidebar');
  assert(Math.abs(state.playlistDetailWidth - state.pageContentClientWidth) <= 2, 'playlist content is not using the full main content width');
  assert(state.documentWidth <= state.viewportWidth, 'desktop playlist page horizontally overflows');
  assert(await page.locator('.primary-nav').getByText('播放清單', { exact: true }).count() === 0, 'primary navigation retained the duplicate playlist entry');

  const treeToggle = treeHost.locator('.playlist-tree-toggle');
  assert(await treeToggle.getAttribute('aria-expanded') === 'true', 'desktop playlist group is not expanded by default');
  await treeToggle.click();
  assert(await treeToggle.getAttribute('aria-expanded') === 'false', 'playlist group did not collapse');
  assert(await treeHost.locator('.playlist-tree-item').count() === 0, 'collapsed playlist group still renders items');
  await treeToggle.click();
  const selected = treeHost.locator('.playlist-tree-item', { hasText: '現場演出' });
  await selected.click();
  state = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(state.selectedPlaylistId === 'playlist-live', 'sidebar playlist item did not select its playlist');
  assert(state.activeView === 'playlists', 'sidebar playlist item did not open playlist content');
  assert(Math.abs(state.playlistDetailWidth - state.pageContentClientWidth) <= 2, 'selected playlist content lost full width');

  const queueNavigation = page.locator('.primary-nav .nav-link', { hasText: '播放佇列' });
  await queueNavigation.click();
  await page.waitForSelector('.playback-queue-row');
  state = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(state.activeView === 'queue', 'clicking the playback queue navigation did not open the queue page');
  assert(await page.locator('.playback-queue-row').count() > 0, 'clicking the playback queue navigation did not display queue entries');
  await treeHost.locator('.playlist-tree-open').click();
  state = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(state.activeView === 'playlists', 'playlist tree heading did not open the playlist page');
  await page.evaluate(() => window.playbackQueueHarness.setView('queue'));
  await page.waitForSelector('.playback-queue-row');
  let beforeCursor = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(beforeCursor.queueCurrentRows.includes('1'), 'initial current queue entry is not marked');
  assert(Math.abs(beforeCursor.queueListWidth - beforeCursor.pageContentClientWidth) <= 2, 'queue list is not using the full main content width');
  const beforeProbeCalls = beforeCursor.queueRequestLog.length;
  await page.evaluate(() => window.playbackQueueHarness.setCurrentEntry(1));
  await page.waitForFunction(() => window.playbackQueueHarness.snapshot().queueCurrentRows.includes('2'));
  let afterCursor = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(afterCursor.queueCurrentRows.length === 1 && afterCursor.queueCurrentRows[0] === '2', 'cursor probe did not move the current marker between duplicate-track entries');
  assert(afterCursor.queueRequestLog.length - beforeProbeCalls === 1, 'cursor update reloaded queue pages instead of requesting one cursor item');
  assert(afterCursor.queueRequestLog.at(-1).offset === 0 && afterCursor.queueRequestLog.at(-1).limit === 1, 'cursor update did not use the one-item metadata request');

  const beforeReplacementCalls = afterCursor.queueRequestLog.length;
  await page.evaluate(() => window.playbackQueueHarness.replaceQueue());
  await page.waitForFunction(() => window.playbackQueueHarness.snapshot().queueCurrentRows.includes('4'));
  afterCursor = await page.evaluate(() => window.playbackQueueHarness.snapshot());
  assert(afterCursor.queueRequestLog.length - beforeReplacementCalls >= 2, 'queue replacement did not reload paged data');
  assert(afterCursor.documentWidth <= afterCursor.viewportWidth, 'desktop queue page horizontally overflows');

  const sweep = await page.evaluate(() => window.playbackQueueHarness.scrollSweep(2_500));
  assert(sweep.total === 100_000, 'synthetic queue did not expose all entries');
  assert(sweep.maxDomRows <= 30, 'queue virtualization rendered too many rows');
  assert(sweep.generatedItems < sweep.total / 10, 'queue paging generated too much of the queue');

  const responsive = [];
  for (const width of [1366, 1024, 800, 412, 360]) {
    await page.setViewportSize({ width, height: width <= 620 ? 915 : 768 });
    await page.evaluate(() => window.playbackQueueHarness.setView('playlists'));
    await page.waitForFunction(() => window.playbackQueueHarness.snapshot().activeView === 'playlists');
    await page.waitForTimeout(35);
    const metrics = await page.evaluate(() => {
      const state = window.playbackQueueHarness.snapshot();
      const host = document.querySelector('.sidebar-playlist-tree-host');
      const tree = host?.querySelector('.playlist-tree');
      const detail = document.querySelector('.playlist-detail');
      const content = document.querySelector('.page-content');
      const contentStyle = content ? getComputedStyle(content) : null;
      return {
        ...state,
        hostLeft: host ? Math.round(host.getBoundingClientRect().left) : -1,
        treeWidth: tree ? Math.round(tree.getBoundingClientRect().width) : 0,
        detailWidth: detail ? Math.round(detail.getBoundingClientRect().width) : 0,
        contentWidth: content && contentStyle
          ? content.clientWidth - parseFloat(contentStyle.paddingLeft) - parseFloat(contentStyle.paddingRight)
          : 0,
      };
    });
    assert(metrics.documentWidth <= metrics.viewportWidth, `document horizontally overflows at ${width}px`);
    assert(metrics.treeVisible, `sidebar playlist tree is unavailable at ${width}px`);
    assert(Math.abs(metrics.detailWidth - metrics.contentWidth) <= 2, `playlist list is not full width at ${width}px`);
    if (width <= 820) {
      assert(metrics.hostLeft < metrics.viewportWidth / 2, `compact playlist tree is not anchored to the sidebar at ${width}px`);
    }
    responsive.push({ width, detailWidth: metrics.detailWidth, contentWidth: metrics.contentWidth, documentWidth: metrics.documentWidth });
  }

  return {
    result: 'PASS',
    queue: { total: sweep.total, maximumVirtualRows: sweep.maxDomRows, metadataProbeItems: 1, duplicateCurrentSlotUpdated: true },
    playlistTree: { sidebarOwned: true, collapsible: true, secondMainColumn: false },
    responsive,
  };
}
