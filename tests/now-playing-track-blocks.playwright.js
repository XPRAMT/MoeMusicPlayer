async page => {
  await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html', {
    waitUntil: 'domcontentloaded',
    timeout: 60000,
  });
  await page.locator('.now-playing-track-info').waitFor({ state: 'visible', timeout: 60000 });
  await page.evaluate(() => window.nowPlayingLayoutHarness.setTrackInfo({
    artists: ['hanser - ultra-long collaborative performer collection', '協力演出者名稱非常長'],
    album: 'Live at the 2025 International Concert Hall Anniversary Edition',
    title: 'さよならスーヴェニア：窄版完整標題測試あいうえお',
  }));
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));

  const cases = [];
  for (const width of [320, 360]) {
    await page.setViewportSize({ width, height: 915 });
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    const result = await page.evaluate(() => {
    const nav = document.querySelector('.now-playing-track-info');
    const topbar = document.querySelector('.now-playing-overlay-topbar');
    const bounds = element => {
      const rect = element.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom, width: rect.width, height: rect.height };
    };
    return {
      nav: { ...bounds(nav), clientWidth: nav.clientWidth, scrollWidth: nav.scrollWidth, overflow: getComputedStyle(nav).overflow, whiteSpace: getComputedStyle(nav).whiteSpace },
      topbar: bounds(topbar),
      blocks: [...nav.querySelectorAll(':scope > [data-track-block]')].map(block => ({
        field: block.dataset.trackBlock,
        ...bounds(block),
        text: block.textContent.trim(),
        children: [...block.querySelectorAll('[data-track-field]')].map(child => ({
          field: child.dataset.trackField,
          text: child.textContent.trim(),
          overflow: getComputedStyle(child).overflow,
          whiteSpace: getComputedStyle(child).whiteSpace,
          textOverflow: getComputedStyle(child).textOverflow,
          scrollWidth: child.scrollWidth,
          clientWidth: child.clientWidth,
        })),
      })),
    };
    });

    if (result.blocks.map(block => block.field).join(',') !== 'artist,album,title') throw new Error(`${width}px: artist, album, and title must remain three ordered blocks`);
    if (result.nav.scrollWidth > result.nav.clientWidth + 1) throw new Error(`${width}px: track blocks overflow horizontally: ${JSON.stringify(result.nav)}`);
    if (result.nav.overflow === 'hidden') throw new Error(`${width}px: track information must not be clipped by its container`);
    if (result.blocks.some(block => block.left < result.nav.left - 1 || block.right > result.nav.right + 1)) {
      throw new Error(`${width}px: a track block exceeds the available width: ${JSON.stringify(result.blocks)}`);
    }
    for (const block of result.blocks) {
      for (const child of block.children) {
        if (child.overflow === 'hidden' || child.whiteSpace === 'nowrap' || child.textOverflow === 'ellipsis') {
          throw new Error(`${width}px: ${block.field} text is hidden or truncated: ${JSON.stringify(child)}`);
        }
      }
    }
    if (result.nav.bottom > result.topbar.bottom + 1) throw new Error(`${width}px: wrapped track information is clipped by the topbar`);
    cases.push({ width, ...result });
  }
  return { result: 'PASS', cases };
}
