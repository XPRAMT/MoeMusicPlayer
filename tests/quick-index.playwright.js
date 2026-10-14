async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 960, height: 720 });
  await page.goto('http://127.0.0.1:4173/tests/quick-index-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('.paged-virtual-group-row');

  const groupRows = page.locator('.paged-virtual-group-row');
  assert(await groupRows.count() >= 1, 'group heading rows are not rendered');
  assert((await groupRows.first().innerText()).trim() === 'A', 'first group heading is not A');
  const firstHeading = groupRows.first().locator('button');
  await firstHeading.click();
  await page.waitForSelector('[role="dialog"][aria-label="快速首字查找"]');
  const dialog = page.getByRole('dialog', { name: '快速首字查找' });
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).evaluate(button => button === document.activeElement), 'dialog did not focus its first available label');
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).isEnabled(), 'available A label is disabled');
  assert(await dialog.getByRole('button', { name: 'B', exact: true }).isEnabled(), 'available B label is disabled');
  assert(await dialog.getByRole('button', { name: 'C', exact: true }).isDisabled(), 'missing C label is not disabled');
  assert(await dialog.getByRole('button', { name: '關閉', exact: true }).count() === 0, 'close button should not be present');
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
      columnWidths: [...document.querySelectorAll('.quick-index-label')].map(button => button.getBoundingClientRect().width),
      expectedColumnWidth: (() => {
        const probe = document.createElement('span');
        probe.style.display = 'block';
        probe.style.font = getComputedStyle(list).font;
        probe.style.width = '10ch';
        probe.style.boxSizing = 'border-box';
        document.body.append(probe);
        const width = probe.getBoundingClientRect().width;
        probe.remove();
        return width;
      })(),
      hasCloseButton: Boolean(popup.querySelector('button[aria-label="關閉首字索引"]')),
    };
  });
  assert(Math.abs(popupGeometry.centerX - popupGeometry.listCenterX) < 1, 'popup is not centered horizontally in the list viewport');
  assert(Math.abs(popupGeometry.centerY - popupGeometry.listCenterY) < 1, 'popup is not centered vertically in the list viewport');
  assert(popupGeometry.maxHeight <= popupGeometry.listHeight - 24 + 1, 'popup max-height is not bounded by list viewport');
  assert(popupGeometry.popupScrollHeight === popupGeometry.popupClientHeight, 'popup should not scroll when all labels fit');
  assert(popupGeometry.columnWidths.every(width => Math.abs(width - popupGeometry.expectedColumnWidth) < 1), `index columns are not fixed at 10ch: ${JSON.stringify(popupGeometry)}`);
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
    unavailableLabelsDisabled: true,
    chosenGroupScrollTop: scrollTop,
    playedDataIndex: played.at(-1).index,
    overflowScrollHeight: overflowGeometry.scrollHeight,
    overflowClientHeight: overflowGeometry.clientHeight,
    restoredDateSortScrollTop: restoreProof.scrollTop,
  };
}
