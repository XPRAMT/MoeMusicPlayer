async page => {
  await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html', {
    waitUntil: 'domcontentloaded',
    timeout: 60000,
  });
  await page.locator('.now-playing-track-info').waitFor({ state: 'visible', timeout: 60000 });
  await page.evaluate(() => window.nowPlayingLayoutHarness.setTrackInfo({
    artists: ['hanser', 'yousa', '鹿乃', '花譜', '星街すいせい', '風真いろは', '白上フブキ', '博衣こより'],
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
        artistUnits: [...block.querySelectorAll('.now-playing-artist-unit')].map(unit => ({
          ...bounds(unit),
          text: unit.textContent.trim(),
          whiteSpace: getComputedStyle(unit).whiteSpace,
        })),
        trailingSeparator: {
          content: getComputedStyle(block, '::after').content,
          visibility: getComputedStyle(block, '::after').visibility,
          hiddenByLayout: block.classList.contains('hide-trailing-separator'),
        },
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
      if (block.field === 'artist') {
        if (block.artistUnits.length !== 8) throw new Error(`${width}px: each artist should have its own wrapping unit`);
        if (new Set(block.artistUnits.map(unit => unit.top)).size < 2) throw new Error(`${width}px: artists should wrap between names when the row fills: ${JSON.stringify(block.artistUnits)}`);
        if (block.artistUnits.some(unit => unit.whiteSpace !== 'nowrap' || unit.height >= 24)) {
          throw new Error(`${width}px: an individual artist name wrapped internally: ${JSON.stringify(block.artistUnits)}`);
        }
      }
      if (block.field !== 'title') {
        const next = result.blocks[result.blocks.indexOf(block) + 1];
        const wrapsBeforeNext = next.top > block.top + 1 || block.height > 30;
        if (block.trailingSeparator.content !== '"．"' || block.trailingSeparator.hiddenByLayout !== wrapsBeforeNext || (wrapsBeforeNext && block.trailingSeparator.visibility !== 'hidden')) {
          throw new Error(`${width}px: a line-ending separator must be hidden in place: ${JSON.stringify(block)}`);
        }
      }
      for (const child of block.children) {
        const expectedNoWrap = block.field === 'artist';
        if (child.overflow === 'hidden' || child.whiteSpace === (expectedNoWrap ? 'normal' : 'nowrap') || child.textOverflow === 'ellipsis') {
          throw new Error(`${width}px: ${block.field} text is hidden or truncated: ${JSON.stringify(child)}`);
        }
        if (expectedNoWrap && child.scrollWidth > child.clientWidth) {
          throw new Error(`${width}px: an individual artist name is clipped: ${JSON.stringify(child)}`);
        }
      }
    }
    if (result.nav.bottom > result.topbar.bottom + 1) throw new Error(`${width}px: wrapped track information is clipped by the topbar`);
    cases.push({ width, ...result });
  }
  await page.evaluate(() => window.nowPlayingLayoutHarness.setTrackInfo({ artists: ['hanser'], album: '專輯', title: '曲名' }));
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const compact = await page.evaluate(() => [...document.querySelectorAll('.now-playing-track-info > [data-track-block]')].map(block => ({
    field: block.dataset.trackBlock,
    top: block.getBoundingClientRect().top,
    visibility: getComputedStyle(block, '::after').visibility,
    content: getComputedStyle(block, '::after').content,
  })));
  if (compact.length !== 3 || compact.some((block, index) => index < 2 && block.visibility !== 'visible') || compact[2].content !== 'none') {
    throw new Error(`360px: separators should remain when the next field shares the line: ${JSON.stringify(compact)}`);
  }
  return { result: 'PASS', cases };
}
