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
    const tops = cells
      .filter((cell) => !cell.classList.contains('column-index'))
      .map((cell) => Math.round(cell.getBoundingClientRect().top));
    const headerBottom = Math.round(header.getBoundingClientRect().bottom);
    const row = document.querySelector('.track-list-viewport .paged-virtual-row');
    const rowTop = Math.round(row.getBoundingClientRect().top);
    const rowCells = [...row.querySelectorAll(':scope > .track-index, :scope > .list-column')];
    const boundaries = cells.map((cell, index) => {
      const headerRect = cell.getBoundingClientRect();
      const rowRect = rowCells[index]?.getBoundingClientRect();
      return rowRect ? {
        headerLeft: headerRect.left,
        rowLeft: rowRect.left,
        headerRight: headerRect.right,
        rowRight: rowRect.right,
      } : null;
    });
    return { tops, headerBottom, rowTop, cellCount: cells.length, rowCellCount: rowCells.length, boundaries };
  });
  assert(geometry.cellCount >= 8, 'header cells collapsed: ' + JSON.stringify(geometry));
  assert(Math.max(...geometry.tops) - Math.min(...geometry.tops) <= 2, 'header cells are not on one line: ' + JSON.stringify(geometry));
  assert(geometry.rowTop >= geometry.headerBottom - 1, 'first row overlaps the header: ' + JSON.stringify(geometry));
  assert(geometry.rowCellCount === geometry.cellCount, 'header and row cell counts differ: ' + JSON.stringify(geometry));
  assert(geometry.boundaries.every((boundary) => boundary
    && Math.abs(boundary.headerLeft - boundary.rowLeft) <= 1
    && Math.abs(boundary.headerRight - boundary.rowRight) <= 1), 'header and row column boundaries differ: ' + JSON.stringify(geometry));
  return { result: 'PASS', geometry };
}
