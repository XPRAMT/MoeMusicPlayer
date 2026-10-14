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
  await dialog.getByRole('button', { name: '關閉', exact: true }).focus();
  await page.keyboard.press('Tab');
  assert(await dialog.getByRole('button', { name: 'A', exact: true }).evaluate(button => button === document.activeElement), 'Tab did not wrap inside the index dialog');
  await page.keyboard.press('Shift+Tab');
  assert(await dialog.getByRole('button', { name: '關閉', exact: true }).evaluate(button => button === document.activeElement), 'Shift+Tab did not wrap inside the index dialog');
  await page.keyboard.press('Escape');
  assert(await page.locator('[role="dialog"][aria-label="快速首字查找"]').count() === 0, 'Escape did not close the index popup');
  assert(await firstHeading.evaluate(button => button === document.activeElement), 'closing the index did not restore focus to its trigger');
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

  return {
    result: 'PASS',
    groupHeadings: await groupRows.count(),
    availableLabelsEnabled: true,
    unavailableLabelsDisabled: true,
    chosenGroupScrollTop: scrollTop,
    playedDataIndex: played.at(-1).index,
  };
}
