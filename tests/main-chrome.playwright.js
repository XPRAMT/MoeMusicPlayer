async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 800 });
  page.on('pageerror', (error) => { throw error; });
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.page-title');
  const libraryTitle = await page.locator('#page-heading').innerText();
  assert(libraryTitle === '我的曲庫', 'library top bar title: ' + libraryTitle);
  assert(/^MoeMusicPlayer \d+\.\d+\.\d+$/.test(await page.title()), 'window title is not the build date: ' + await page.title());
  assert(await page.locator('.section-kicker').count() === 0, 'english section kickers remain on the library page');
  assert(await page.getByRole('heading', { name: '我的曲庫', exact: true }).count() === 1, 'library title is duplicated');
  const sidebar = await page.locator('.sidebar').innerText();
  for (const removed of ['你的音樂空間', '本機曲庫', 'LOCAL FIRST', 'MoeMusicPlayer']) {
    assert(!sidebar.includes(removed), 'sidebar still shows ' + removed);
  }
  assert(await page.locator('[data-testid="sync-progress-banner"]').count() === 0, 'sync banner is on the library page');

  await page.getByRole('button', { name: '播放佇列' }).click();
  assert(await page.locator('#page-heading').innerText() === '播放佇列', 'queue title missing');
  assert(await page.getByRole('heading', { name: '播放佇列', exact: true }).count() === 1, 'queue title is duplicated');

  await page.getByRole('button', { name: '設定', exact: true }).click();
  assert(await page.locator('#page-heading').innerText() === '設定', 'settings title missing');
  await page.getByRole('tab', { name: '音樂來源' }).click();
  assert(await page.getByRole('heading', { name: '管理音樂來源', exact: true }).count() === 1, 'sources heading missing');
  assert(await page.locator('.section-kicker').count() === 0, 'english kicker remains in sources');
  return { result: 'PASS', libraryTitle };
}
