async page => {
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
await page.goto('http://127.0.0.1:4176/tests/favorite-dock-harness.html');
await page.setViewportSize({ width: 1280, height: 800 });
await page.getByRole('button', { name: '隨機播放' }).waitFor();

const heart = page.locator('.dock-favorite');
await page.waitForFunction(() => document.querySelector('.dock-favorite')?.classList.contains('has-favorites'));
const firstTrack = await page.evaluate(() => {
  const heartButton = document.querySelector('.dock-favorite');
  const shuffleButton = document.querySelector('[aria-label="隨機播放"]');
  const heartSvg = heartButton.querySelector('svg');
  return {
    favoriteColor: getComputedStyle(heartButton).color,
    shuffleColor: getComputedStyle(shuffleButton).color,
    heartFill: getComputedStyle(heartSvg).fill,
    heartPressed: heartButton.getAttribute('aria-pressed'),
    calls: [...window.__favoriteDockHarness.calls],
  };
});
if (firstTrack.heartPressed !== 'true' || firstTrack.heartFill === 'none') {
  throw new Error(`Favorited track did not show a solid heart: ${JSON.stringify(firstTrack)}`);
}
if (firstTrack.favoriteColor !== 'rgb(230, 230, 230)' || firstTrack.shuffleColor !== 'rgb(230, 230, 230)') {
  throw new Error(`Dock active controls used a non-neutral color: ${JSON.stringify(firstTrack)}`);
}

await page.evaluate(() => {
  window.__favoriteStateChanges = [];
  const heartButton = document.querySelector('.dock-favorite');
  new MutationObserver(() => window.__favoriteStateChanges.push(heartButton.getAttribute('aria-pressed')))
    .observe(heartButton, { attributes: true, attributeFilter: ['aria-pressed'] });
});
await page.getByTestId('next-track').click();
await page.waitForFunction(() => document.querySelector('[data-testid="current-track"]')?.textContent === 'track-two');
await page.waitForFunction(() => window.__favoriteDockHarness.calls.includes('track-two'));
await page.waitForTimeout(80);
const duringFavoriteLookup = await page.locator('.dock-favorite').getAttribute('aria-pressed');
if (duringFavoriteLookup !== 'true' || (await page.evaluate(() => window.__favoriteStateChanges)).includes('false')) {
  throw new Error(`Favorite state flickered while looking up another playlist member: ${JSON.stringify({
    duringFavoriteLookup,
    changes: await page.evaluate(() => window.__favoriteStateChanges),
  })}`);
}
await page.getByTestId('next-track').click();
await page.waitForFunction(() => document.querySelector('[data-testid="current-track"]')?.textContent === 'track-three');
await page.waitForFunction(() => window.__favoriteDockHarness.calls.includes('track-three'));
await page.waitForTimeout(350);
const afterStaleReply = await page.evaluate(() => ({
  currentTrack: document.querySelector('[data-testid="current-track"]').textContent,
  favoritePressed: document.querySelector('.dock-favorite').getAttribute('aria-pressed'),
  heartFill: getComputedStyle(document.querySelector('.dock-favorite svg')).fill,
  calls: [...window.__favoriteDockHarness.calls],
}));
if (afterStaleReply.currentTrack !== 'track-three' || afterStaleReply.favoritePressed !== 'false' || afterStaleReply.heartFill !== 'none') {
  throw new Error(`A stale playlist response replaced the current track state: ${JSON.stringify(afterStaleReply)}`);
}
if (!afterStaleReply.calls.includes('track-two') || !afterStaleReply.calls.includes('track-three')) {
  throw new Error(`Track change did not trigger fresh membership checks: ${JSON.stringify(afterStaleReply)}`);
}

await heart.click();
await page.getByRole('dialog', { name: '收藏到播放清單' }).waitFor();
await page.getByTestId('next-track').click();
await page.waitForFunction(() => !document.querySelector('.favorite-popover'));
await heart.click();
await page.getByRole('dialog', { name: '收藏到播放清單' }).waitFor();
await page.setViewportSize({ width: 721, height: 800 });
await page.waitForFunction(() => {
  const panel = document.querySelector('.favorite-popover')?.getBoundingClientRect();
  const anchor = document.querySelector('.favorite-control')?.getBoundingClientRect();
  return panel && anchor && panel.left >= 8 && panel.right <= innerWidth - 8 && Math.abs(anchor.top - panel.bottom - 14) <= 1;
});
const panelGeometry = await page.evaluate(() => {
  const panel = document.querySelector('.favorite-popover').getBoundingClientRect();
  const anchor = document.querySelector('.favorite-control').getBoundingClientRect();
  return {
    left: panel.left,
    right: panel.right,
    bottomGap: anchor.top - panel.bottom,
    viewportWidth: innerWidth,
  };
});
if (panelGeometry.left < 8 || panelGeometry.right > panelGeometry.viewportWidth - 8 || Math.abs(panelGeometry.bottomGap - 14) > 1) {
  throw new Error(`Favorite panel did not keep the volume panel spacing and edge inset: ${JSON.stringify(panelGeometry)}`);
}

await page.setViewportSize({ width: 720, height: 800 });
await page.waitForFunction(() => {
  const button = document.querySelector('.dock-favorite');
  return button && getComputedStyle(button).display === 'none' && !document.querySelector('.favorite-popover');
});
await page.evaluate(() => document.querySelector('.app-shell').classList.add('has-now-playing-backdrop'));
await page.waitForFunction(() => getComputedStyle(document.querySelector('.dock-favorite')).display !== 'none');
await heart.click();
await page.getByRole('dialog', { name: '收藏到播放清單' }).waitFor();
await page.setViewportSize({ width: 620, height: 800 });
await page.waitForFunction(() => document.querySelector('.favorite-popover') !== null);
await heart.click();
await page.waitForFunction(() => !document.querySelector('.favorite-popover'));

await page.locator('[aria-label="隨機播放"]').hover();
const hoverColor = await page.locator('[aria-label="隨機播放"]').evaluate((element) => getComputedStyle(element).color);
if (hoverColor !== 'rgb(230, 230, 230)') throw new Error(`Shuffle hover used theme color ${hoverColor}`);
if (errors.length) throw new Error(`Browser errors: ${errors.join(' | ')}`);
await page.evaluate(() => { document.title = 'PASS: favorite close parity and responsive alignment'; });
console.log('PASS: live membership, stale-response fencing, outside-click close, non-Now-Playing responsive close, Now Playing visibility, edge alignment, and neutral dock colors.');
}
