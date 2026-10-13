async page => {
page.setDefaultTimeout(6000);
await page.setViewportSize({ width: 1120, height: 720 });
await page.goto('http://127.0.0.1:1452/tests/artist-index-harness.html');
await page.getByRole('grid', { name: /演出者統計/ }).waitFor();
await page.getByText('Artist 001', { exact: true }).first().waitFor();

const layout = await page.evaluate(() => {
  const topbar = document.querySelector('[data-testid="topbar"]');
  const scroll = document.querySelector('[data-testid="page-scroll"]');
  const content = document.querySelector('[data-testid="page-content"]');
  const section = document.querySelector('[data-testid="artist-section"]');
  const frame = document.querySelector('.paged-virtual-frame');
  const viewport = document.querySelector('.paged-virtual-viewport');
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
  };
});

const assert = (condition, message) => { if (!condition) throw new Error(message); };
assert(layout.searchControlCount === 0, 'artist page should not add a separate search field');
assert(layout.pageTopPadding === '0px', 'artist list should start directly under the topbar');
assert(Math.abs(layout.frame.top - layout.topbar.bottom) <= 1, `frame should start at the topbar edge: ${JSON.stringify(layout)}`);
assert(Math.abs(layout.frame.bottom - layout.scroll.bottom) <= 1, `frame should fill the remaining page: ${JSON.stringify(layout)}`);
assert(layout.viewport.height > 500, `viewport should use the available page height: ${JSON.stringify(layout)}`);
assert(layout.virtualRowCount < layout.totalRows, 'long artist list should render virtually');

const firstRowBefore = await page.locator('.paged-virtual-row').first().innerText();
await page.getByRole('button', { name: '演出者', exact: true }).click();
const firstRowAfter = await page.locator('.paged-virtual-row').first().innerText();
assert(firstRowAfter !== firstRowBefore, 'shared header renderer should sort artist rows');

await page.locator('.paged-virtual-row').first().click();
assert(await page.getByTestId('selected-artist').innerText() !== '尚未選擇');

return { ...layout, firstRowBefore, firstRowAfter, selectedArtist: await page.getByTestId('selected-artist').innerText() };
}
