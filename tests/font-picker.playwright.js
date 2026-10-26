async page => {
  // The font picker (ADR 0042) in its harness (tests/font-picker-harness.html, `npx vite --port 4173`): the library's
  // group rows and quick index panel, 「預設字型」 first, keyboard choice, and rows drawn without the real families.
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 960, height: 720 });
  await page.goto('http://127.0.0.1:4173/tests/font-picker-harness.html', { waitUntil: 'domcontentloaded' });
  await page.click('#harness-font-picker-button');
  await page.waitForSelector('.paged-virtual-group-row');

  const headings = await page.locator('.paged-virtual-group-row').allInnerTexts();
  assert(JSON.stringify(headings.slice(0, 3)) === JSON.stringify(['繁體中文', '簡體中文', '日文']), `headings: ${headings}`);
  const firstRow = await page.locator('.font-picker-row').first().innerText();
  assert(firstRow.startsWith('預設'), `first row: ${firstRow}`);
  const geometry = await page.evaluate(() => {
    const group = document.querySelector('.paged-virtual-group-row');
    const row = document.querySelector('.paged-virtual-row.font-picker-row');
    const surface = group?.querySelector('.list-group-row-surface');
    return {
      group: group?.getBoundingClientRect().height,
      row: row?.getBoundingClientRect().height,
      line: surface ? getComputedStyle(surface).borderBottomWidth : null,
      top: surface ? getComputedStyle(surface).borderTopWidth : null,
      rowFonts: [...document.querySelectorAll('.font-picker-row-name')].map((name) => name.style.fontFamily),
    };
  });
  assert(geometry.row === 36 && geometry.group === 30, `heights ${geometry.row} / ${geometry.group}`);
  assert(geometry.line === '1px' && geometry.top === '0px', `separator ${geometry.top} / ${geometry.line}`);
  assert(geometry.rowFonts.every((font) => !font.includes('微軟正黑體') && !font.includes('Latin')), `rows never use the real family: ${geometry.rowFonts}`);

  // Group heading → quick index panel → jump.
  await page.locator('.paged-virtual-group-button').first().click();
  await page.waitForSelector('.quick-index-popover');
  const labels = await page.locator('.quick-index-label').allInnerTexts();
  assert(labels.join('|') === '繁體中文|簡體中文|日文|英文', `labels: ${labels}`);
  await page.locator('.quick-index-label', { hasText: '英文' }).click();
  await page.waitForFunction(() => document.querySelector('.paged-virtual-viewport').scrollTop > 0);

  // Search hides the empty groups; 「預設字型」 stays first.
  await page.fill('.font-picker-search', 'latin 1');
  await page.waitForFunction(() => document.querySelectorAll('.paged-virtual-group-row').length === 1);
  assert((await page.locator('.paged-virtual-group-row').innerText()) === '英文', 'only 英文 has matches');
  assert((await page.locator('.font-picker-row').first().innerText()).startsWith('預設'), '預設字型 while searching');

  // Keyboard: focus the list, move down, Enter chooses.
  await page.focus('.paged-virtual-viewport');
  await page.keyboard.press('ArrowDown');
  await page.keyboard.press('Enter');
  const chosen = await page.evaluate(() => globalThis.__FONT_PICKER_CHOSEN__);
  assert(chosen.length === 1 && chosen[0] === 'Latin 1', `chosen: ${JSON.stringify(chosen)}`);
  return { headings, labels, chosen };
}
