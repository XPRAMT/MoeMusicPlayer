async page => {
  page.setDefaultTimeout(6000);
  const setRange = async (locator, value) => locator.evaluate((element, next) => {
    element.value = String(next);
    element.dispatchEvent(new Event('input', { bubbles: true }));
    element.dispatchEvent(new Event('change', { bubbles: true }));
  }, value);

  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto('http://127.0.0.1:1452/tests/volume-slider-harness.html');
  await page.waitForFunction(() => Boolean(window.__themeHarness));
  await page.evaluate(() => window.__themeHarness.set({
    colorMode: 'autoBackgroundOpacity',
    mainBackgroundMode: 'albumCover',
    mainBackgroundClarityPercent: 24,
    coverBackgroundClarityPercent: 76,
  }));
  await page.reload();
  await page.waitForFunction(() => Boolean(document.querySelector('.player-dock')));

  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '主介面' }).click();
  const mainClarity = page.getByRole('slider', { name: '主介面背景清晰度' });
  await page.waitForFunction(() => document.querySelector('[aria-label="主介面背景清晰度"]')?.value === '24');
  const mainLabels = await page.locator('.main-interface-panel .lyrics-preference-range').evaluateAll(items => items.map(item => item.textContent));
  const mainBlur = mainLabels.findIndex(text => text.includes('背景模糊'));
  const mainClarityIndex = mainLabels.findIndex(text => text.includes('清晰度'));
  const mainOpacity = mainLabels.findIndex(text => text.includes('背景透明度'));
  if (!(mainBlur >= 0 && mainClarityIndex === mainBlur + 1 && mainOpacity === mainClarityIndex + 1)) {
    throw new Error(`主介面清晰度順序錯誤：${JSON.stringify(mainLabels)}`);
  }

  await page.getByRole('tab', { name: '播放頁' }).click();
  const coverClarity = page.getByRole('slider', { name: '封面背景清晰度' });
  await page.waitForFunction(() => document.querySelector('[aria-label="封面背景清晰度"]')?.value === '76');
  const coverLabels = await page.locator('.quick-settings-group[aria-label="封面與歌詞排列"] .lyrics-preference-range').evaluateAll(items => items.map(item => item.textContent));
  const coverBlur = coverLabels.findIndex(text => text.includes('封面背景模糊'));
  const coverClarityIndex = coverLabels.findIndex(text => text.includes('清晰度'));
  const coverOpacity = coverLabels.findIndex(text => text.includes('封面背景透明度'));
  if (!(coverBlur >= 0 && coverClarityIndex === coverBlur + 1 && coverOpacity === coverClarityIndex + 1)) {
    throw new Error(`播放頁清晰度順序錯誤：${JSON.stringify(coverLabels)}`);
  }

  await setRange(coverClarity, 43);
  await page.waitForFunction(() => window.__themeHarness.get().coverBackgroundClarityPercent === 43);
  if (await page.evaluate(() => window.__themeHarness.get().mainBackgroundClarityPercent) !== 24) {
    throw new Error('調整播放頁清晰度時改變了主介面清晰度');
  }
  await page.getByRole('tab', { name: '主介面' }).click();
  await setRange(mainClarity, 31);
  await page.waitForFunction(() => window.__themeHarness.get().mainBackgroundClarityPercent === 31);
  if (await page.evaluate(() => window.__themeHarness.get().coverBackgroundClarityPercent) !== 43) {
    throw new Error('調整主介面清晰度時改變了播放頁清晰度');
  }
  await page.reload();
  await page.waitForFunction(() => document.querySelector('[aria-label="主要導覽"]') !== null);
  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '主介面' }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="主介面背景清晰度"]')?.value === '31');
  await page.getByRole('tab', { name: '播放頁' }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="封面背景清晰度"]')?.value === '43');
  return { result: 'PASS', mainClarity: 31, coverClarity: 43, mainOrder: mainLabels, coverOrder: coverLabels };
}
