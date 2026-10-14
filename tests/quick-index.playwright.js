async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 960, height: 720 });
  await page.goto('http://127.0.0.1:4173/tests/quick-index-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.paged-virtual-group-row');

  const groupRows = page.locator('.paged-virtual-group-row');
  assert(await groupRows.count() >= 1, 'group heading rows are not rendered');
  assert((await groupRows.first().innerText()).trim() === 'A', 'first group heading is not A');
  const firstHeading = groupRows.first().locator('button');
  const headingGeometry = await page.evaluate(() => {
    const row = document.querySelector('.paged-virtual-group-row');
    const button = row?.querySelector('button');
    if (!row || !button) return null;
    const rowRect = row.getBoundingClientRect();
    const buttonRect = button.getBoundingClientRect();
    const textRange = document.createRange();
    textRange.selectNodeContents(button);
    const textRect = textRange.getBoundingClientRect();
    return {
      row: { x: rowRect.x, y: rowRect.y, width: rowRect.width, height: rowRect.height },
      button: { x: buttonRect.x, y: buttonRect.y, width: buttonRect.width, height: buttonRect.height },
      textCenterX: textRect.left + textRect.width / 2,
      buttonCenterX: buttonRect.left + buttonRect.width / 2,
      rowBackground: getComputedStyle(row).backgroundColor,
      buttonBackground: getComputedStyle(button).backgroundColor,
      configuredRowHeight: Number.parseFloat(getComputedStyle(document.querySelector('.paged-virtual-frame')).getPropertyValue('--paged-row-height')),
    };
  });
  assert(headingGeometry, 'group row geometry could not be read');
  assert(Math.abs(headingGeometry.button.x - headingGeometry.row.x) < 0.5 && Math.abs(headingGeometry.button.width - headingGeometry.row.width) < 0.5, `quick-index button does not fill the row width: ${JSON.stringify(headingGeometry)}`);
  assert(Math.abs(headingGeometry.button.height - headingGeometry.configuredRowHeight) < 0.5, `quick-index button does not follow configured row height: ${JSON.stringify(headingGeometry)}`);
  assert(Math.abs(headingGeometry.textCenterX - headingGeometry.buttonCenterX) < 0.5, `group label is not centered across the row: ${JSON.stringify(headingGeometry)}`);
  assert(headingGeometry.rowBackground === 'rgba(0, 0, 0, 0)' && headingGeometry.buttonBackground === 'rgba(0, 0, 0, 0)', `group row is not transparent: ${JSON.stringify(headingGeometry)}`);
  await firstHeading.click({ position: { x: headingGeometry.button.width - 4, y: headingGeometry.button.height / 2 } });
  await page.waitForSelector('[role="dialog"][aria-label="快速首字查找"]');
  const dialog = page.getByRole('dialog', { name: '快速首字查找' });
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).evaluate(button => button === document.activeElement), 'dialog did not focus its first available label');
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).isEnabled(), 'available A label is disabled');
  assert(await dialog.getByRole('button', { name: 'B', exact: true }).isEnabled(), 'available B label is disabled');
  assert(await dialog.getByRole('button', { name: 'C', exact: true }).count() === 0, 'unused C label should not be present');
  assert(await dialog.getByRole('button', { name: '關閉', exact: true }).count() === 0, 'close button should not be present');
  assert(await dialog.locator('.quick-index-popover-header').count() === 0, 'quick-index title should not be rendered');
  const popupGeometry = await page.evaluate(() => {
    const popup = document.querySelector('.quick-index-popover');
    const list = document.querySelector('.paged-virtual-viewport');
    const popupRect = popup.getBoundingClientRect();
    const listRect = list.getBoundingClientRect();
    return {
      centerX: popupRect.left + popupRect.width / 2,
      centerY: popupRect.top + popupRect.height / 2,
      listCenterX: listRect.left + listRect.width / 2,
      listCenterY: listRect.top + listRect.height / 2,
      maxHeight: Number.parseFloat(getComputedStyle(popup).maxHeight),
      listHeight: list.clientHeight,
      popupClientHeight: popup.clientHeight,
      popupScrollHeight: popup.scrollHeight,
      columnCount: getComputedStyle(document.querySelector('.quick-index-label-grid')).gridTemplateColumns.trim().split(/\s+/).length,
      rowHeight: Number.parseFloat(getComputedStyle(document.querySelector('.paged-virtual-frame')).getPropertyValue('--paged-row-height')),
      characterFontSize: Number.parseFloat(getComputedStyle(document.querySelector('.quick-index-label-grid')).fontSize),
      buttonGeometry: [...document.querySelectorAll('.quick-index-label')].map(button => {
        const rect = button.getBoundingClientRect();
        return { width: rect.width, height: rect.height };
      }),
      hasCloseButton: Boolean(popup.querySelector('button[aria-label="關閉首字索引"]')),
    };
  });
  assert(Math.abs(popupGeometry.centerX - popupGeometry.listCenterX) < 1, 'popup is not centered horizontally in the list viewport');
  assert(Math.abs(popupGeometry.centerY - popupGeometry.listCenterY) < 1, 'popup is not centered vertically in the list viewport');
  assert(popupGeometry.maxHeight <= popupGeometry.listHeight - 24 + 1, 'popup max-height is not bounded by list viewport');
  assert(popupGeometry.popupScrollHeight === popupGeometry.popupClientHeight, 'popup should not scroll when all labels fit');
  assert(popupGeometry.columnCount === 10, `index must keep exactly 10 columns: ${JSON.stringify(popupGeometry)}`);
  assert(popupGeometry.buttonGeometry.every(({ width, height }) => Math.abs(width - popupGeometry.characterFontSize * 2) < 1 && Math.abs(height - popupGeometry.characterFontSize * 2) < 1), `index character blocks are not square at two character widths: ${JSON.stringify(popupGeometry)}`);
  assert(Math.abs(popupGeometry.characterFontSize - 16) < 0.1, `index character font size does not use --font-title: ${JSON.stringify(popupGeometry)}`);
  assert(Math.abs(popupGeometry.characterFontSize * 2 - popupGeometry.rowHeight) > 1, `index character block still follows configured list row height: ${JSON.stringify(popupGeometry)}`);
  assert(!popupGeometry.hasCloseButton, 'popup contains a visible close button');
  await dialog.getByRole('button', { name: 'B', exact: true }).focus();
  await page.keyboard.press('Tab');
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).evaluate(button => button === document.activeElement), 'Tab did not wrap inside the index dialog');
  await page.keyboard.press('Shift+Tab');
  assert(await dialog.getByRole('button', { name: 'B', exact: true }).evaluate(button => button === document.activeElement), 'Shift+Tab did not wrap inside the index dialog');
  await page.keyboard.press('Escape');
  assert(await page.locator('[role="dialog"][aria-label="快速首字查找"]').count() === 0, 'Escape did not close the index popup');
  assert(await firstHeading.evaluate(button => button === document.activeElement), 'closing the index did not restore focus to its trigger');
  await firstHeading.click();
  await page.waitForSelector('[role="dialog"][aria-label="快速首字查找"]');
  await page.locator('.quick-index-backdrop').click({ position: { x: 5, y: 5 } });
  assert(await page.locator('[role="dialog"][aria-label="快速首字查找"]').count() === 0, 'clicking outside the popup did not close it');
  await firstHeading.click();
  await page.waitForSelector('[role="dialog"][aria-label="快速首字查找"]');
  await dialog.getByRole('button', { name: 'B', exact: true }).click();
  assert(await page.locator('[role="dialog"][aria-label="快速首字查找"]').count() === 0, 'index popup did not close after choosing a label');

  const viewport = page.locator('.paged-virtual-viewport');
  const scrollTop = await viewport.evaluate(element => element.scrollTop);
  assert(scrollTop > 0, 'choosing B did not jump to its group');
  await page.waitForFunction(() => [...document.querySelectorAll('.paged-virtual-row')].some(row => row.classList.contains('active') && row.textContent?.includes('Track 41')));
  await page.locator('.paged-virtual-row.activates').filter({ hasText: 'Track 41' }).click();
  const played = await page.evaluate(() => window.quickIndexHarness.played);
  assert(played.at(-1)?.id === 'entry-40' && played.at(-1)?.index === 40, 'group heading changed the underlying data row index');

  await viewport.evaluate(element => {
    element.style.height = '150px';
    element.style.flex = '0 0 150px';
    element.style.maxHeight = '150px';
    element.scrollTop = 0;
    element.dispatchEvent(new Event('scroll'));
  });
  await page.waitForFunction(() => document.querySelector('.paged-virtual-viewport')?.clientHeight === 150);
  await page.locator('.paged-virtual-group-row').first().locator('button').click();
  await page.waitForSelector('[role="dialog"][aria-label="快速首字查找"]');
  const overflowGeometry = await dialog.evaluate(element => ({
    maxHeight: Number.parseFloat(getComputedStyle(element).maxHeight),
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
    listHeight: document.querySelector('.paged-virtual-viewport').clientHeight,
  }));
  assert(overflowGeometry.maxHeight <= overflowGeometry.listHeight - 24 + 1, 'overflow popup exceeds list area');
  assert(overflowGeometry.scrollHeight > overflowGeometry.clientHeight, 'overflow popup did not enable scrolling when content exceeds list area');
  await page.locator('.quick-index-backdrop').click({ position: { x: 5, y: 5 } });

  await page.locator('[data-testid="date-sort"]').click();
  await page.waitForFunction(() => document.querySelectorAll('.paged-virtual-group-row').length === 0);
  await viewport.evaluate(element => { element.scrollTop = 1200; element.dispatchEvent(new Event('scroll')); });
  await page.waitForFunction(() => [...document.querySelectorAll('.paged-virtual-row')].some(row => row.textContent?.includes('Track 31')));
  await page.evaluate(() => window.quickIndexHarness.navigateAwayAndBack());
  await page.waitForFunction(() => document.querySelector('.paged-virtual-viewport')?.scrollTop >= 1200);
  await page.waitForFunction(() => [...document.querySelectorAll('.paged-virtual-row')].some(row => row.textContent?.includes('Track 31')));
  const restoreProof = await page.evaluate(() => ({
    scrollTop: document.querySelector('.paged-virtual-viewport')?.scrollTop ?? 0,
    requestedRanges: window.quickIndexHarness.requestedRanges,
  }));
  assert(restoreProof.requestedRanges.some(range => range.start <= 30 && range.end > 30), 'restored date/numeric sort did not request data around the saved list position');
  assert(restoreProof.scrollTop >= 1200, 'sidebar return did not restore the date/numeric sort scroll position');

  const headingCount = await groupRows.count();
  return {
    result: 'PASS',
    groupHeadings: headingCount,
    availableLabelsEnabled: true,
    unusedLabelsOmitted: true,
    chosenGroupScrollTop: scrollTop,
    playedDataIndex: played.at(-1).index,
    overflowScrollHeight: overflowGeometry.scrollHeight,
    overflowClientHeight: overflowGeometry.clientHeight,
    restoredDateSortScrollTop: restoreProof.scrollTop,
  };
}
