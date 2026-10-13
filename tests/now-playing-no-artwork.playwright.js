async page => {
await page.goto('http://127.0.0.1:4174/tests/now-playing-layout-harness.html');
await page.waitForFunction(() => window.nowPlayingLayoutHarness && document.querySelector('[data-testid="timed-lyrics"]'));
const result = [];
for (const viewport of [{width:1366,height:768},{width:412,height:915}]) {
  await page.setViewportSize(viewport);
  await page.evaluate(() => window.nowPlayingLayoutHarness.setArtworkVisible(false));
  for (const layout of ['a','b']) {
    await page.evaluate(value => window.nowPlayingLayoutHarness.setLayout(value),layout);
    const state = await page.evaluate(() => {
      const card = document.querySelector('.now-playing-card');
      const lyrics = card?.querySelector('[data-layout-pane="lyrics"]');
      const cr = card.getBoundingClientRect(), lr = lyrics.getBoundingClientRect(), cs = getComputedStyle(card);
      return {artwork:card.dataset.artworkVisible, panes:[...card.querySelectorAll(':scope > [data-layout-pane]')].map(x=>x.dataset.layoutPane), card:{w:cr.width,h:cr.height}, lyrics:{w:lr.width,h:lr.height}, content:{w:cr.width-parseFloat(cs.paddingLeft)-parseFloat(cs.paddingRight),h:cr.height-parseFloat(cs.paddingTop)-parseFloat(cs.paddingBottom)}, justify:getComputedStyle(lyrics).justifyContent};
    });
    if(state.artwork!=='false'||JSON.stringify(state.panes)!=='["lyrics"]'||Math.abs(state.lyrics.w-state.content.w)>1||state.lyrics.h<=state.content.h*.8||state.justify!=='center') throw new Error(JSON.stringify({viewport,layout,state}));
    result.push({viewport:`${viewport.width}x${viewport.height}`,layout,lyrics:state.lyrics});
  }
  await page.evaluate(() => window.nowPlayingLayoutHarness.setArtworkVisible(true));
}
console.log(JSON.stringify(result));

}
