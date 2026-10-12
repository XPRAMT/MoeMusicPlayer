async page => {
  await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html', {
    waitUntil: 'domcontentloaded',
    timeout: 60000,
  });
  await page.locator('.now-playing-format').waitFor({ state: 'visible', timeout: 60000 });

  const cases = [];
  for (const width of [360, 412]) {
    await page.setViewportSize({ width, height: 915 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const result = await page.evaluate(() => {
      const row = document.querySelector('.now-playing-format');
      const quality = row?.querySelector('.now-playing-audio-quality');
      const metadata = row?.querySelector('.now-playing-release-and-count');
      const rect = element => {
        const value = element.getBoundingClientRect();
        return { x: value.x, y: value.y, width: value.width, height: value.height, right: value.right };
      };
      return {
        row: rect(row),
        quality: { ...rect(quality), text: quality.textContent.trim() },
        metadata: { ...rect(metadata), text: metadata.textContent.trim() },
      };
    });
    if (!result.quality.text.endsWith('1030 kbps')) throw new Error(`${width}px: kbps was split from the quality summary`);
    if (result.quality.height >= 24) throw new Error(`${width}px: quality summary occupies multiple lines`);
    if (result.metadata.height >= 24) throw new Error(`${width}px: release date and play count split apart`);
    if (width <= 390 && result.metadata.y <= result.quality.y) throw new Error(`${width}px: release date/count should wrap below the complete quality summary`);
    if (width <= 390 && Math.abs(result.metadata.y - result.quality.y - result.quality.height) > 1) {
      throw new Error(`${width}px: wrapped rows should keep the normal single-line spacing`);
    }
    if (width > 390 && result.metadata.y !== result.quality.y) throw new Error(`${width}px: both groups should share a row when they fit`);
    cases.push({ width, ...result });
  }
  return { result: 'PASS', cases };
}
