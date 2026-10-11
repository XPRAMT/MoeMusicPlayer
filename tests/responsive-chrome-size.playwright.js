async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    ok(value, message) {
      if (!value) throw new Error(message);
    },
  };

  const readSizes = () => page.evaluate(() => {
    const box = (selector) => {
      const element = document.querySelector(selector);
      if (!element) return null;
      const rect = element.getBoundingClientRect();
      return { top: Math.round(rect.top), bottom: Math.round(rect.bottom), height: Math.round(rect.height) };
    };
    return {
      mainTopbar: box('.workspace .topbar'),
      playbackTopbar: box('.now-playing-overlay-topbar'),
      playbackInfo: box('.now-playing-header-track'),
      playbackTools: box('.now-playing-topbar-tools'),
      dock: box('.player-dock'),
      controls: box('.player-dock .dock-controls'),
      controlButton: box('.player-dock .control-button'),
      playButton: box('.player-dock .play-button'),
    };
  });

  await page.setViewportSize({ width: 1280, height: 800 });
  if (await page.locator('.now-playing-quick-settings-scrim').count()) {
    await page.locator('.now-playing-quick-settings-scrim').evaluate((element) => element.click());
  }
  if (await page.locator('.now-playing-overlay.is-open').count()) {
    await page.locator('.dock-now-playing-dismiss').evaluate((element) => element.click());
  }
  await page.locator('.dock-art').click();
  await page.waitForFunction(() => document.querySelector('.now-playing-overlay')?.classList.contains('is-open'));
  const wide = await readSizes();

  await page.setViewportSize({ width: 600, height: 800 });
  await page.waitForTimeout(35);
  const beforeStack = await readSizes();
  assert.equal(beforeStack.mainTopbar.height, wide.mainTopbar.height, 'main topbar height stays unchanged before Now Playing content stacks');
  assert.equal(beforeStack.playbackTopbar.height, wide.playbackTopbar.height, 'Now Playing topbar height stays unchanged before its content stacks');
  assert.equal(beforeStack.dock.height, wide.dock.height, 'playback dock height stays unchanged before Now Playing content stacks');
  assert.equal(beforeStack.controls.height, wide.controls.height, 'playback control row height stays unchanged before Now Playing content stacks');

  await page.setViewportSize({ width: 520, height: 800 });
  await page.waitForTimeout(35);
  const stacked = await readSizes();
  assert.equal(stacked.mainTopbar.height, 74, 'main topbar remains 74px after Now Playing content stacks');
  assert.ok(stacked.playbackTopbar.height >= 88, `stacked Now Playing topbar grows to contain its rows (${stacked.playbackTopbar.height}px)`);
  assert.ok(stacked.playbackTools.bottom <= stacked.playbackInfo.top, 'stacked buttons and song information occupy separate rows');
  assert.ok(stacked.playbackInfo.bottom <= stacked.playbackTopbar.bottom, 'stacked song information fits inside the expanded topbar');
  assert.equal(stacked.dock.height, 74, 'playback dock remains 74px after Now Playing content stacks');
  assert.equal(stacked.controls.height, 44, 'playback control row remains 44px after Now Playing content stacks');

  await page.setViewportSize({ width: 360, height: 800 });
  await page.waitForTimeout(35);
  const narrow = await readSizes();
  assert.equal(narrow.mainTopbar.height, 74, 'main topbar stays fixed at 360px width');
  assert.ok(narrow.playbackTopbar.height >= 82, `stacked Now Playing topbar keeps its narrow-width content height (${narrow.playbackTopbar.height}px)`);
  assert.ok(narrow.playbackTools.bottom <= narrow.playbackInfo.top, 'narrow buttons and song information occupy separate rows');
  assert.equal(narrow.dock.height, 74, 'playback dock stays fixed at 360px width');
  assert.equal(narrow.controls.height, 44, 'playback control row stays fixed at 360px width');

  return { result: 'PASS', wide, beforeStack, stacked, narrow };
}
