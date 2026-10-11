async page => {
  await page.setContent(`<!doctype html><html><head><link rel="stylesheet" href="http://127.0.0.1:1452/src/app.css"></head><body>
    <div class="app-shell" data-active-view="library">
      <main class="workspace"><header class="topbar"></header></main>
      <footer class="player-dock" data-timeline-style="bar">
        <div class="dock-track"><button class="dock-art">cover</button><div class="dock-track-copy"><strong>Track</strong><span>Artist</span></div></div>
        <div class="dock-center"><div class="dock-controls">
          <button class="control-button dock-favorite">favorite</button>
          <button class="control-button secondary-control" aria-label="shuffle">shuffle</button>
          <button class="control-button">previous</button><button class="play-button">play</button><button class="control-button">next</button>
          <button class="control-button secondary-control" aria-label="repeat">repeat</button>
          <div class="dock-volume">volume</div>
        </div></div>
      </footer>
    </div></body></html>`);
  await page.waitForFunction(() => getComputedStyle(document.querySelector('.player-dock')).display === 'grid');

  const samples = [];
  for (const width of [1000, 820, 800, 721, 720, 621, 620, 521, 520, 390, 320, 200]) {
    await page.setViewportSize({ width, height: 800 });
    samples.push(await page.evaluate(() => {
      const visible = (selector) => getComputedStyle(document.querySelector(selector)).display !== 'none';
      const rect = (selector) => {
        const { x, right, width, height } = document.querySelector(selector).getBoundingClientRect();
        return { x, right, width, height };
      };
      return {
        viewport: innerWidth,
        art: rect('.dock-art'),
        dock: rect('.player-dock'),
        controls: rect('.dock-controls'),
        favorite: visible('.dock-favorite'),
        volume: visible('.dock-volume'),
        shuffle: visible('.secondary-control[aria-label="shuffle"]'),
        repeat: visible('.secondary-control[aria-label="repeat"]'),
      };
    }));
  }

  for (const sample of samples) {
    if (sample.art.width < 64) throw new Error(`Cover shrank to ${sample.art.width}px at viewport ${sample.viewport}px`);
    const stageOne = sample.viewport <= 720;
    if (sample.favorite === stageOne || sample.volume === stageOne) throw new Error(`Favorite/volume stage mismatch at ${sample.viewport}px`);
    const stageTwo = sample.viewport <= 520;
    if (sample.shuffle === stageTwo || sample.repeat === stageTwo) throw new Error(`Shuffle/repeat stage mismatch at ${sample.viewport}px`);
    if (stageTwo && Math.abs(sample.controls.right - (sample.dock.right - 8)) > 1) throw new Error(`Controls are not right aligned at ${sample.viewport}px: ${JSON.stringify(sample.controls)}`);
  }

  const wideSize = samples.find((sample) => sample.viewport === 1000).art.width;
  if (samples.some((sample) => sample.art.width < wideSize)) throw new Error('Cover is smaller at a narrow width than on a wide viewport.');
  return { result: 'PASS', samples };
}
