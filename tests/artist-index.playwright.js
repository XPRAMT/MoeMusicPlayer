async page => {
const assert = (condition, message) => { if (!condition) throw new Error(message); };
page.setDefaultTimeout(6000);
await page.setViewportSize({ width: 1120, height: 720 });
await page.goto('http://127.0.0.1:1452/tests/artist-index-harness.html', { waitUntil: 'commit', timeout: 30000 });
await page.evaluate(() => sessionStorage.removeItem('artist-list-sort'));
await page.reload();
const artistCountLabel = page.getByTestId('artist-count');
await artistCountLabel.waitFor();
assert(await artistCountLabel.innerText() === '位演出者', 'artist count label should be visible without a placeholder dash while loading');
const countLabelBefore = await artistCountLabel.boundingBox();
await page.getByRole('grid', { name: /演出者統計/ }).waitFor();
await page.getByText('Artist 001', { exact: true }).first().waitFor();
await page.getByTestId('artist-count').getByText('80 位演出者', { exact: true }).waitFor();
const countLabelAfter = await artistCountLabel.boundingBox();
assert(countLabelBefore.height === countLabelAfter.height && countLabelBefore.y === countLabelAfter.y, 'artist subtitle should stay in place when count is filled in');

const layout = await page.evaluate(() => {
  const topbar = document.querySelector('[data-testid="topbar"]');
  const scroll = document.querySelector('[data-testid="page-scroll"]');
  const content = document.querySelector('[data-testid="page-content"]');
  const section = document.querySelector('[data-testid="artist-section"]');
  const frame = document.querySelector('.paged-virtual-frame');
  const viewport = document.querySelector('.paged-virtual-viewport');
  const headerRow = document.querySelector('.track-table-head');
  const dataRow = document.querySelector('.artist-index-row');
  const rect = (element) => {
    const { top, bottom, height } = element.getBoundingClientRect();
    return { top, bottom, height };
  };
  return {
    topbar: rect(topbar),
    scroll: rect(scroll),
    content: rect(content),
    section: rect(section),
    frame: rect(frame),
    viewport: rect(viewport),
    pageTopPadding: getComputedStyle(content).paddingTop,
    searchControlCount: document.querySelectorAll('input[type="search"]').length,
    virtualRowCount: document.querySelectorAll('.paged-virtual-row').length,
    totalRows: Number(document.querySelector('[role="grid"]')?.getAttribute('aria-rowcount')),
    headerGridColumns: getComputedStyle(headerRow).gridTemplateColumns.split(' ').map(Number.parseFloat),
    dataGridColumns: getComputedStyle(dataRow).gridTemplateColumns.split(' ').map(Number.parseFloat),
    dataFontSize: Number.parseFloat(getComputedStyle(dataRow).fontSize),
  };
});

assert(layout.searchControlCount === 0, 'artist page should not add a separate search field');
assert(layout.pageTopPadding === '0px', 'artist list should start directly under the topbar');
assert(Math.abs(layout.frame.top - layout.topbar.bottom) <= 1, `frame should start at the topbar edge: ${JSON.stringify(layout)}`);
assert(Math.abs(layout.frame.bottom - layout.scroll.bottom) <= 1, `frame should fill the remaining page: ${JSON.stringify(layout)}`);
assert(layout.viewport.height > 500, `viewport should use the available page height: ${JSON.stringify(layout)}`);
assert(layout.virtualRowCount < layout.totalRows, 'long artist list should render virtually');
for (let column = 2; column < 6; column += 1) {
  assert(
    Math.abs(layout.headerGridColumns[column] - layout.dataGridColumns[column]) < 0.1,
    `artist header and data column ${column} should have equal widths: ${JSON.stringify(layout)}`,
  );
  assert(
    Math.abs(layout.dataGridColumns[column] - layout.dataFontSize * 6) < 0.1,
    `artist summary columns should be six fullwidth characters: ${JSON.stringify(layout)}`,
  );
}
for (const label of ['歌曲數量', '專輯數量', '累積播放', '播放時長']) {
  await page.getByRole('button', { name: new RegExp(label) }).waitFor();
}

await page.getByRole('button', { name: /累積播放/ }).click();
const cumulativeSortedFirstRow = await page.locator('.paged-virtual-row').first().innerText();
assert(cumulativeSortedFirstRow.includes('Artist 001'), 'the cumulative play column should sort actual listening time independently');
await page.getByRole('button', { name: /播放時長/ }).click();
const durationSortedFirstRow = await page.locator('.paged-virtual-row').first().innerText();
assert(durationSortedFirstRow.includes('Artist 080'), 'the new track duration column should sort by summed song length');
await page.getByTestId('artist-section').getByRole('button', { name: '演出者', exact: true }).click();
const firstRowBefore = await page.locator('.paged-virtual-row').first().innerText();
await page.getByTestId('artist-section').getByRole('button', { name: '演出者', exact: true }).click();
const firstRowAfter = await page.locator('.paged-virtual-row').first().innerText();
assert(firstRowAfter !== firstRowBefore, 'shared header renderer should sort artist rows');
assert(firstRowAfter.includes('Artist 080'), 'clicking the active title sort should reverse its order');

const viewport = page.locator('.paged-virtual-viewport');
await viewport.evaluate(element => { element.scrollTop = 1200; element.dispatchEvent(new Event('scroll')); });
const preservedScrollTop = await viewport.evaluate(element => element.scrollTop);
const preservedFirstVisibleRow = await page.locator('.paged-virtual-row').first().innerText();
const artistHomeElement = await page.locator('[data-testid="artist-section"]').evaluate(element => {
  window.__artistHomeElement = element;
  return true;
});
assert(artistHomeElement, 'artist home should be mounted before opening a detail');
const visibleRowIndex = await page.locator('.paged-virtual-row').evaluateAll((rows, selector) => {
  const viewport = document.querySelector(selector);
  const viewportRect = viewport.getBoundingClientRect();
  return rows.findIndex(row => {
    const rect = row.getBoundingClientRect();
    return rect.top >= viewportRect.top && rect.bottom <= viewportRect.bottom;
  });
}, '.paged-virtual-viewport');
assert(visibleRowIndex >= 0, 'a fully visible artist row should be available to open its detail page');
await page.locator('.paged-virtual-row').nth(visibleRowIndex).click();
assert(await page.getByTestId('selected-artist').innerText() !== '尚未選擇');
await page.getByTestId('artist-detail').waitFor();
const detailCount = page.getByTestId('artist-track-count');
assert(await detailCount.innerText() === '首曲目', 'artist detail should keep the track count label visible while loading');
const detailCountBefore = await detailCount.boundingBox();
await detailCount.getByText('160 首曲目', { exact: true }).waitFor();
const detailCountAfter = await detailCount.boundingBox();
assert(detailCountBefore.height === detailCountAfter.height && detailCountBefore.y === detailCountAfter.y, 'artist detail subtitle should stay in place when count is filled in');
assert(await page.getByTestId('artist-section').evaluate(element => element.hidden), 'artist index should stay mounted but hidden while viewing detail');
assert(await page.getByTestId('artist-section').evaluate(element => getComputedStyle(element).display === 'none'), 'artist index should not render while viewing detail');
assert(await page.locator('[data-testid="artist-section"]').evaluate(element => element === window.__artistHomeElement), 'opening an artist detail should preserve the artist home instance');
const selectedArtistName = await page.getByTestId('selected-artist').innerText();
await page.getByTestId('nav-library').click();
assert(await page.getByTestId('artist-detail').count() === 0, 'artist detail should be destroyed while viewing the library');
assert(await page.getByTestId('artist-section').count() === 0, 'artist home should be destroyed when leaving the artist section');
await page.getByTestId('nav-artists').click();
await page.getByTestId('artist-detail').waitFor();
assert((await page.getByTestId('artist-detail').innerText()).includes(selectedArtistName), 'artist navigation should restore the same artist');
assert(await page.getByTestId('artist-section').count() === 0, 'artist home should not remain mounted after leaving the artist section');
await page.getByRole('button', { name: '返回演出者首頁' }).click();
await page.getByTestId('artist-section').waitFor();
assert(await page.getByTestId('artist-detail').count() === 0, 'the explicit artist breadcrumb should destroy the detail list');
const restoredScrollTop = await viewport.evaluate(element => element.scrollTop);
const restoredFirstVisibleRow = await page.locator('.paged-virtual-row').first().innerText();
assert(restoredScrollTop === preservedScrollTop, `artist list should recover its scroll position after remount: ${preservedScrollTop} -> ${restoredScrollTop}`);
assert(restoredFirstVisibleRow === preservedFirstVisibleRow, 'artist sort and visible rows should survive detail navigation');

await page.reload();
await page.getByRole('grid', { name: /演出者統計/ }).waitFor();
await page.getByText('Artist 080', { exact: true }).first().waitFor();
const restoredSortAfterRestart = await page.locator('.paged-virtual-row').first().innerText();
assert(restoredSortAfterRestart.includes('Artist 080'), 'artist sort field and direction should restore after app startup');

return { ...layout, artistCount: await page.getByTestId('artist-count').innerText(), firstRowBefore, firstRowAfter, preservedScrollTop, restoredScrollTop, preservedFirstVisibleRow, restoredFirstVisibleRow, restoredSortAfterRestart, selectedArtist: await page.getByTestId('selected-artist').innerText() };
}
