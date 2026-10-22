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
      const releaseDate = row?.querySelector('.now-playing-release-date-block');
      const playCount = row?.querySelector('.now-playing-play-count-block');
      const rect = element => {
        const value = element.getBoundingClientRect();
        return { x: value.x, y: value.y, width: value.width, height: value.height, right: value.right };
      };
      return {
        row: rect(row),
        quality: { ...rect(quality), text: quality.textContent.trim() },
        releaseDate: { ...rect(releaseDate), text: releaseDate.textContent.trim() },
        playCount: { ...rect(playCount), text: playCount.textContent.trim() },
      };
    });
    if (!result.quality.text.endsWith('320 kbps')) throw new Error(`${width}px: kbps was split from the quality summary`);
    if (result.quality.height >= 24) throw new Error(`${width}px: quality summary occupies multiple lines`);
    if (result.releaseDate.height >= 24 || result.playCount.height >= 24) throw new Error(`${width}px: release date or play count occupies multiple lines`);
    if (result.releaseDate.y !== result.quality.y || result.playCount.y !== result.quality.y) {
      throw new Error(`${width}px: short metadata blocks should fit beside the quality block without an early forced wrap`);
    }
    cases.push({ width, ...result });
  }
  await page.setViewportSize({ width: 360, height: 915 });
  const wrapped = await page.evaluate(() => {
    const quality = document.querySelector('.now-playing-audio-quality > span');
    const releaseDate = document.querySelector('.now-playing-release-date-block');
    const playCount = document.querySelector('.now-playing-play-count-block');
    quality.textContent = 'FLAC．384 kHz．32 bit．22.6 Mbps';
    releaseDate.textContent = '2025-08-15';
    const rect = element => {
      const value = element.getBoundingClientRect();
      return { x: value.x, y: value.y, width: value.width, height: value.height, right: value.right };
    };
    return {
      quality: { ...rect(document.querySelector('.now-playing-audio-quality')), text: quality.textContent.trim() },
      releaseDate: { ...rect(releaseDate), text: releaseDate.textContent.trim() },
      playCount: { ...rect(playCount), text: playCount.textContent.trim() },
    };
  });
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  if (!wrapped.quality.text.endsWith('22.6 Mbps') || wrapped.quality.height >= 24) {
    throw new Error('360px: long quality summary split internally instead of wrapping as one block');
  }
  if (wrapped.releaseDate.height >= 24 || wrapped.playCount.height >= 24) {
    throw new Error('360px: release date and play count should each remain one block');
  }
  if (wrapped.playCount.y === wrapped.quality.y && wrapped.releaseDate.y === wrapped.quality.y) {
    throw new Error(`360px: long metadata did not wrap naturally: ${JSON.stringify(wrapped)}`);
  }
  cases.push({ width: 360, scenario: 'natural-wrap', ...wrapped });
  return { result: 'PASS', cases };
}
