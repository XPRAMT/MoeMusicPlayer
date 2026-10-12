async page => {
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
await page.goto('http://127.0.0.1:4175/tests/favorite-dock-harness.html');
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

await page.getByTestId('next-track').click();
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

await page.locator('[aria-label="隨機播放"]').hover();
const hoverColor = await page.locator('[aria-label="隨機播放"]').evaluate((element) => getComputedStyle(element).color);
if (hoverColor !== 'rgb(230, 230, 230)') throw new Error(`Shuffle hover used theme color ${hoverColor}`);
if (errors.length) throw new Error(`Browser errors: ${errors.join(' | ')}`);
console.log('PASS: live per-track membership, stale-response fencing, solid heart, and neutral dock active/hover colors.');
}
