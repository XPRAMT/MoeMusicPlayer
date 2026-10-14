async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('http://127.0.0.1:1453/tests/volume-slider-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.page-title');
  await page.evaluate(() => {
    window.__themeHarness.set({
      ...window.__themeHarness.get(),
      mainBackgroundMode: 'slideshow',
      slideshowFolder: 'C:\\fixture',
      slideshowIntervalMinutes: 1,
    });
    location.reload();
  });
  await page.waitForSelector('.page-title');
  await page.getByRole('button', { name: '設定' }).click();
  const interval = page.getByRole('slider', { name: '幻燈片切換時間' });
  await page.waitForFunction(() => Boolean(window.__themeHarness));
  await page.evaluate(() => {
    const realNow = Date.now.bind(Date);
    window.__slideshowAtomicClock = { realNow, offsetMs: 60_000 };
    Date.now = () => window.__slideshowAtomicClock.realNow() + window.__slideshowAtomicClock.offsetMs;
    window.__appearanceHarness.provideSlideshowImages = true;
  });
  const setInterval = async (value) => interval.evaluate((input, next) => {
    input.value = String(next);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }, value);
  await setInterval(2);
  await setInterval(1);
  await page.waitForFunction(() => window.__appearanceHarness.slideshowRequests === 1, null, { timeout: 5000 });
  await page.waitForSelector('.main-backdrop img');
  await page.waitForFunction(() => document.querySelector('.main-backdrop img')?.complete
    && document.querySelector('.main-backdrop img')?.naturalWidth === 16);
  const displayedUrl = await page.locator('.main-backdrop img').getAttribute('src');

  await page.evaluate(() => {
    window.__appearanceHarness.holdImageDecode = true;
    window.__slideshowAtomicClock.offsetMs += 60_000;
  });
  await setInterval(2);
  await setInterval(1);
  await page.waitForFunction(() => window.__appearanceHarness.slideshowRequests === 2, null, { timeout: 5000 });
  await page.waitForFunction(() => window.__appearanceHarness.pendingImageDecodes.length === 1, null, { timeout: 5000 });
  assert(await page.locator('.main-backdrop img').getAttribute('src') === displayedUrl,
    'the previous slideshow image was cleared before the replacement finished decoding');

  await page.evaluate(() => {
    window.__appearanceHarness.holdImageDecode = false;
    window.__appearanceHarness.releaseImageDecode();
  });
  await page.waitForFunction((previous) => document.querySelector('.main-backdrop img')?.getAttribute('src') !== previous,
    displayedUrl, { timeout: 5000 });
  await page.waitForFunction(() => document.querySelector('.main-backdrop img')?.complete
    && document.querySelector('.main-backdrop img')?.naturalWidth === 16);
  await page.evaluate(() => { Date.now = window.__slideshowAtomicClock.realNow; });
  assert(await page.locator('.main-backdrop img').count() === 1, 'the background image node was remounted during a slideshow swap');
}
