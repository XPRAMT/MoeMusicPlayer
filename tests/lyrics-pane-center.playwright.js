async page => {
await page.setViewportSize({ width: 1920, height: 1080 });
await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html');
await page.waitForFunction(() => window.nowPlayingLayoutHarness && document.querySelector('[data-testid="cover-image"]')?.naturalWidth === 400);
await page.evaluate(() => window.nowPlayingLayoutHarness.setLayout('a'));
await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));

const before = await page.evaluate(() => {
  const artwork = document.querySelector('[data-layout-pane="artwork"]').getBoundingClientRect();
  const cover = document.querySelector('[data-testid="cover-stage"]').getBoundingClientRect();
  return { artworkWidth: artwork.width, coverCenter: (cover.left + cover.right) / 2, artworkCenter: (artwork.left + artwork.right) / 2 };
});
await page.getByRole('button', { name: '關閉歌詞' }).click();
await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const closed = await page.evaluate(() => {
  const artwork = document.querySelector('[data-layout-pane="artwork"]').getBoundingClientRect();
  const cover = document.querySelector('[data-testid="cover-stage"]').getBoundingClientRect();
  const lyrics = document.querySelector('[data-layout-pane="lyrics"]');
  return {
    artworkWidth: artwork.width,
    coverCenter: (cover.left + cover.right) / 2,
    artworkCenter: (artwork.left + artwork.right) / 2,
    lyricsDisplay: getComputedStyle(lyrics).display,
  };
});
if (!(closed.artworkWidth > before.artworkWidth * 1.8)) throw new Error(`hidden lyrics should release the second column: ${JSON.stringify({ before, closed })}`);
if (Math.abs(closed.coverCenter - closed.artworkCenter) > 1) throw new Error(`cover is not horizontally centered with lyrics hidden: ${JSON.stringify(closed)}`);
if (closed.lyricsDisplay !== 'none') throw new Error(`lyrics pane remained visible: ${JSON.stringify(closed)}`);

await page.getByRole('button', { name: '開啟歌詞' }).click();
await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const reopened = await page.evaluate(() => {
  const artwork = document.querySelector('[data-layout-pane="artwork"]').getBoundingClientRect();
  const cover = document.querySelector('[data-testid="cover-stage"]').getBoundingClientRect();
  const lyrics = document.querySelector('[data-layout-pane="lyrics"]');
  return { artworkWidth: artwork.width, coverCenter: (cover.left + cover.right) / 2, artworkCenter: (artwork.left + artwork.right) / 2, lyricsDisplay: getComputedStyle(lyrics).display };
});
if (!(reopened.artworkWidth < closed.artworkWidth / 1.8)) throw new Error(`reopening lyrics should restore the split layout: ${JSON.stringify(reopened)}`);
if (reopened.lyricsDisplay === 'none') throw new Error(`lyrics pane did not reopen: ${JSON.stringify(reopened)}`);
if (Math.abs(reopened.coverCenter - reopened.artworkCenter) > 1) throw new Error(`cover did not recenter in the artwork column: ${JSON.stringify(reopened)}`);

await page.evaluate(() => window.nowPlayingLayoutHarness.setLayout('b'));
await page.getByRole('button', { name: '關閉歌詞' }).click();
await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const layoutBClosed = await page.evaluate(() => {
  const artwork = document.querySelector('[data-layout-pane="artwork"]').getBoundingClientRect();
  const cover = document.querySelector('[data-testid="cover-stage"]').getBoundingClientRect();
  return Math.abs((cover.left + cover.right) / 2 - (artwork.left + artwork.right) / 2);
});
if (layoutBClosed > 1) throw new Error(`layout B cover is not centered with lyrics hidden: ${layoutBClosed}px error`);

await page.setViewportSize({ width: 360, height: 800 });
await page.evaluate(() => window.nowPlayingLayoutHarness.setLayout('a'));
await page.getByRole('button', { name: '開啟歌詞' }).click();
await page.getByRole('button', { name: '關閉歌詞' }).click();
await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
const narrowClosed = await page.evaluate(() => {
  const artwork = document.querySelector('[data-layout-pane="artwork"]').getBoundingClientRect();
  const cover = document.querySelector('[data-testid="cover-stage"]').getBoundingClientRect();
  return Math.abs((cover.left + cover.right) / 2 - (artwork.left + artwork.right) / 2);
});
if (narrowClosed > 1) throw new Error(`narrow layout cover is not horizontally centered with lyrics hidden: ${narrowClosed}px error`);

console.log(JSON.stringify({ before, closed, reopened, layoutBClosed, narrowClosed }));
}
