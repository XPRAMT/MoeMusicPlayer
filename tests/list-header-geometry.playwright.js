async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('http://127.0.0.1:4173/tests/configurable-columns-harness.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => Boolean(window.configurableColumnsHarness));
  await page.evaluate(() => window.configurableColumnsHarness.releasePages());
  await page.waitForSelector('.track-list-viewport .paged-virtual-row .list-column-title');
  const geometry = await page.evaluate(() => {
    const header = document.querySelector('.paged-virtual-header [role="row"]');
    const cells = [...header.querySelectorAll(':scope > .list-column, :scope > .column-index')];
    const tops = cells.map((cell) => Math.round(cell.getBoundingClientRect().top));
    const headerBottom = Math.round(header.getBoundingClientRect().bottom);
    const row = document.querySelector('.track-list-viewport .paged-virtual-row');
    const rowTop = Math.round(row.getBoundingClientRect().top);
    return { tops, headerBottom, rowTop, cellCount: cells.length };
  });
  assert(geometry.cellCount >= 8, 'header cells collapsed: ' + JSON.stringify(geometry));
  assert(Math.max(...geometry.tops) - Math.min(...geometry.tops) <= 2, 'header cells are not on one line: ' + JSON.stringify(geometry));
  assert(geometry.rowTop >= geometry.headerBottom - 1, 'first row overlaps the header: ' + JSON.stringify(geometry));
  return { result: 'PASS', geometry };
}
