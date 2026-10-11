async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
  };

  const readSizes = () => page.evaluate(() => {
    const height = (selector) => {
      const element = document.querySelector(selector);
      return element ? Math.round(element.getBoundingClientRect().height) : null;
    };
    return {
      mainTopbar: height('.workspace .topbar'),
      playbackTopbar: height('.now-playing-overlay-topbar'),
      dock: height('.player-dock'),
      controls: height('.player-dock .dock-controls'),
      controlButton: height('.player-dock .control-button'),
      playButton: height('.player-dock .play-button'),
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

  await page.setViewportSize({ width: 360, height: 800 });
  await page.waitForTimeout(35);
  const narrow = await readSizes();

  for (const key of ['mainTopbar', 'playbackTopbar', 'dock', 'controls', 'controlButton', 'playButton']) {
    assert.equal(narrow[key], wide[key], `${key} height must remain stable when the viewport narrows`);
  }
  assert.equal(narrow.mainTopbar, 74, 'main topbar remains 74px');
  assert.equal(narrow.playbackTopbar, 74, 'Now Playing topbar remains 74px');
  assert.equal(narrow.dock, 74, 'playback dock remains 74px');
  assert.equal(narrow.controls, 44, 'playback control row remains 44px');

  return { result: 'PASS', wide, narrow };
}
