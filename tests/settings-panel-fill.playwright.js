async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('http://127.0.0.1:4173/', { waitUntil: 'domcontentloaded' });
  await page.getByRole('button', { name: '設定', exact: true }).click();
  const panel = await page.locator('.settings-panel').first().evaluate((element) => getComputedStyle(element).backgroundColor);
  assert(panel === 'rgba(0, 0, 0, 0)' || panel === 'transparent', 'settings panel still has a fill: ' + panel);
  assert(await page.locator('.theme-preview').count() === 0, 'main interface color preview remains');
  return { result: 'PASS', panel };
}
