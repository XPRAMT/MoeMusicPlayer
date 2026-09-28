async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const defaultOrder = ['title', 'artist', 'album', 'year', 'audioFormat', 'duration'];
  const groupName = '正在播放頁排列';
  await page.setViewportSize({ width: 360, height: 800 });
  await page.goto('http://127.0.0.1:4173/tests/configurable-columns-harness.html');
  await page.waitForFunction(() => Boolean(window.configurableColumnsHarness));
  await page.waitForSelector('.track-list-viewport .paged-virtual-row');

  let state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  const mobileMetrics = { ...state.page };
  assert(JSON.stringify(state.library.header) === JSON.stringify(defaultOrder), 'library default header order differs');
  assert(JSON.stringify(state.playlist.header) === JSON.stringify(defaultOrder), 'playlist default header order differs');
  for (const [name, list] of [['library', state.library], ['playlist', state.playlist]]) {
    assert(JSON.stringify(list.skeleton) === JSON.stringify(defaultOrder), name + ' skeleton columns differ');
    assert(list.headerCellCount === 8 && list.rowCellCount === 8, name + ' fixed/skeleton cells not synchronized');
    assert(list.ariaColumnCount === '8', name + ' aria-colcount does not match default grid');
    assert(list.scrollWidth > list.clientWidth, name + ' list does not contain its own horizontal scrolling');
  }
  assert(mobileMetrics.documentWidth <= mobileMetrics.viewportWidth, 'outer page horizontally overflows at 360px');
  assert(state.switches.length === 2 && state.switches.every(group => group.role === 'group' && JSON.stringify(group.pressed) === '["true","false"]'), 'A/B controls lack accessible toggle state');
  assert(await page.locator('[role="radio"]').count() === 0, 'incomplete radio semantics remain');

  const moved = [
    { id: 'artist', visible: true }, { id: 'title', visible: true }, { id: 'album', visible: true },
    { id: 'year', visible: true }, { id: 'duration', visible: true }, { id: 'audioFormat', visible: true },
  ];
  await page.evaluate(columns => window.configurableColumnsHarness.setColumns(columns), moved);
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  const movedOrder = moved.map(column => column.id);
  for (const [name, list] of [['library', state.library], ['playlist', state.playlist]]) {
    assert(JSON.stringify(list.header) === JSON.stringify(movedOrder), name + ' moved header order not applied');
    assert(JSON.stringify(list.skeleton) === JSON.stringify(movedOrder), name + ' moved skeleton order not applied');
  }

  await page.evaluate(() => window.configurableColumnsHarness.releasePages());
  await page.waitForSelector('.track-row .list-column-title');
  await page.waitForSelector('.playlist-entry-row .list-column-title');
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  for (const [name, list] of [['library', state.library], ['playlist', state.playlist]]) {
    assert(JSON.stringify(list.header) === JSON.stringify(movedOrder), name + ' loaded header order not applied');
    assert(JSON.stringify(list.row) === JSON.stringify(movedOrder), name + ' loaded row order differs from header');
    assert(list.headerCellCount === 8 && list.rowCellCount === 8, name + ' loaded cell count mismatch');
    assert(list.ariaColumnCount === '8', name + ' loaded aria-colcount mismatch');
  }

  const missing = await page.locator('.playlist-entry-row').filter({ hasText: 'M3U 項目' }).evaluate(row => ({
    cells: [...row.querySelectorAll('.list-column')].map(cell => ({ className: cell.className, text: cell.textContent.trim() })),
    artistSubtitles: row.querySelectorAll('.playlist-entry-title small').length,
  }));
  assert(missing.artistSubtitles === 0, 'playlist title duplicates artist as a subtitle');
  for (const id of ['artist', 'album', 'year', 'audioFormat', 'duration']) {
    assert(missing.cells.find(cell => cell.className.includes('list-column-' + id))?.text === '—', 'unmatched playlist ' + id + ' lacks placeholder');
  }

  for (const hiddenId of defaultOrder) {
    const preferences = defaultOrder.map(id => ({ id, visible: id !== hiddenId }));
    await page.evaluate(value => window.configurableColumnsHarness.setColumns(value), preferences);
    state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
    const expectedVisible = defaultOrder.filter(id => id !== hiddenId);
    for (const [name, list] of [['library', state.library], ['playlist', state.playlist]]) {
      assert(JSON.stringify(list.header) === JSON.stringify(expectedVisible), name + ' did not hide ' + hiddenId + ' in header');
      assert(JSON.stringify(list.row) === JSON.stringify(expectedVisible), name + ' did not hide ' + hiddenId + ' in row');
      assert(list.headerCellCount === expectedVisible.length + 2, name + ' header has stale cells after hiding ' + hiddenId);
      assert(list.rowCellCount === expectedVisible.length + 2, name + ' row has stale cells after hiding ' + hiddenId);
      assert(list.ariaColumnCount === String(expectedVisible.length + 2), name + ' aria-colcount is stale after hiding ' + hiddenId);
    }
  }

  await page.evaluate(columns => window.configurableColumnsHarness.setColumns(columns), moved);
  const b = await page.evaluate(() => window.configurableColumnsHarness.setLayout('b'));
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  assert(JSON.stringify(b.paneOrder) === '["lyrics","artwork"]', 'B layout DOM order is not lyrics then artwork');
  assert(b.nodesPreserved, 'layout toggle recreated artwork/lyrics DOM nodes');
  assert(b.lyricsText.includes('尚無可顯示歌詞'), 'lyrics placeholder disappeared after layout toggle');
  assert(state.layout.paneTops[0] < state.layout.paneTops[1], 'narrow B layout does not stack lyrics before artwork');
  const a = await page.evaluate(() => window.configurableColumnsHarness.setLayout('a'));
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  assert(JSON.stringify(a.paneOrder) === '["artwork","lyrics"]', 'A layout DOM order is not artwork then lyrics');
  assert(a.nodesPreserved, 'A/B switch recreated artwork/lyrics DOM nodes');
  assert(state.layout.paneTops[0] < state.layout.paneTops[1], 'narrow A layout does not stack artwork before lyrics');

  const groups = page.getByRole('group', { name: groupName });
  await groups.nth(0).getByRole('button', { name: '排列 B：歌詞在前，封面在後' }).click();
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  assert(state.layout.paneOrder.join(',') === 'lyrics,artwork', 'Now Playing A/B button did not update layout');
  assert(state.switches.every(group => JSON.stringify(group.pressed) === '["false","true"]'), 'compact and settings toggle states diverged');
  await groups.nth(1).getByRole('button', { name: '排列 A：封面在前，歌詞在後' }).click();
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  assert(state.layout.paneOrder.join(',') === 'artwork,lyrics', 'settings A/B button did not update layout');
  assert(state.switches.every(group => JSON.stringify(group.pressed) === '["true","false"]'), 'settings toggle did not update the Now Playing control');

  await page.setViewportSize({ width: 1280, height: 900 });
  await page.waitForTimeout(50);
  state = await page.evaluate(() => window.configurableColumnsHarness.snapshot());
  assert(state.layout.paneTops[0] === state.layout.paneTops[1], 'wide A layout unexpectedly stacked');
  await page.evaluate(() => window.configurableColumnsHarness.setLayout('b'));
  const wideB = await page.evaluate(() => {
    const panes = [...document.querySelectorAll('.now-playing-card > [data-layout-pane]')];
    return {
      order: panes.map(pane => pane.dataset.layoutPane),
      lefts: panes.map(pane => Math.round(pane.getBoundingClientRect().left)),
      tops: panes.map(pane => Math.round(pane.getBoundingClientRect().top)),
    };
  });
  assert(wideB.order.join(',') === 'lyrics,artwork', 'wide B DOM order mismatch');
  assert(wideB.lefts[0] < wideB.lefts[1] && wideB.tops[0] === wideB.tops[1], 'wide B did not put lyrics before artwork horizontally');
  return {
    result: 'PASS',
    viewport360: { documentWidth: mobileMetrics.documentWidth, viewportWidth: mobileMetrics.viewportWidth },
    defaultColumns: defaultOrder,
    hiddenColumnCasesPerList: defaultOrder.length,
    narrowLayouts: { a: ['artwork', 'lyrics'], b: ['lyrics', 'artwork'] },
    ariaPressedControls: 'both controls stay synchronized',
    nowPlayingChildrenPreserved: true,
    wideB: wideB.order,
  };
}