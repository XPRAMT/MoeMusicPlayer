async page => {
  page.setDefaultTimeout(6000);
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
    ok(value, message) {
      if (!value) throw new Error(message);
    },
    deepEqual(actual, expected, message) {
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw new Error(`${message}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
      }
    },
  };
  const wait = milliseconds => page.waitForTimeout(milliseconds);
  const setRange = async (locator, value, eventName = 'input') => locator.evaluate((element, next) => {
    element.value = String(next.value);
    element.dispatchEvent(new Event(next.eventName, { bubbles: true }));
  }, { value, eventName });

  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto('http://127.0.0.1:1452/');
  await page.evaluate(() => localStorage.removeItem('__appearancePreferences'));
  await page.goto('http://127.0.0.1:1452/tests/volume-slider-harness.html');
  await page.waitForFunction(() => document.querySelector('.dock-art-image')?.naturalWidth === 800);
  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '正在播放' }).click();
  const blur = page.getByRole('slider', { name: '封面背景模糊程度' });
  const transparency = page.getByRole('slider', { name: '元件底色透明度' });
  await page.waitForFunction(() => document.querySelector('[aria-label="封面背景模糊程度"]')?.value === '20');

  await page.evaluate(() => { window.__appearanceHarness.holdAcks = true; });
  await setRange(blur, 12);
  await page.waitForFunction(() => window.__appearanceHarness.requests.length === 1);
  for (const value of [14, 16, 17, 19, 21, 23, 25]) {
    await setRange(blur, value);
    await wait(9);
  }
  await setRange(transparency, 48);
  const preview = await page.evaluate(() => ({
    blur: document.querySelector('.now-playing-appearance-preview').style.getPropertyValue('--preview-blur'),
    alpha: document.querySelector('.now-playing-appearance-preview').style.getPropertyValue('--preview-surface-alpha'),
    blurValue: document.querySelector('[aria-label="封面背景模糊程度"]').value,
    transparencyValue: document.querySelector('[aria-label="元件底色透明度"]').value,
  }));
  assert.equal(preview.blur, '25px', 'blur preview should update immediately');
  assert.equal(preview.alpha, '0.52', 'surface preview should update immediately');
  assert.equal(preview.blurValue, '25', 'blur slider should keep the latest pointer value');
  assert.equal(preview.transparencyValue, '48', 'transparency slider should keep the latest pointer value');
  await wait(220);
  assert.equal(await page.evaluate(() => window.__appearanceHarness.requests.length), 1, 'rapid input should not enqueue writes behind a slow ACK');
  await page.evaluate(() => window.__appearanceHarness.releaseAck());
  await page.waitForFunction(() => window.__appearanceHarness.requests.length === 2);
  const latest = await page.evaluate(() => window.__appearanceHarness.requests[1]);
  assert.deepEqual(latest, { backgroundBlurPx: 25, surfaceTransparencyPercent: 48 }, 'the active request should flush only the latest preferences');
  await page.evaluate(() => window.__appearanceHarness.releaseAck());
  await page.waitForFunction(() => document.querySelector('.now-playing-appearance-settings [role="status"]')?.textContent.includes('已保存'));

  await page.evaluate(() => { window.__appearanceHarness.failNext = true; });
  await setRange(transparency, 68, 'change');
  await page.waitForFunction(() => window.__appearanceHarness.requests.length === 3);
  await page.evaluate(() => window.__appearanceHarness.releaseAck());
  await page.waitForFunction(() => document.querySelector('.now-playing-appearance-settings [role="status"]')?.textContent.includes('無法保存正在播放外觀'));
  assert.equal(await page.evaluate(() => window.__appearanceHarness.requests.length), 3, 'a failed save must stop cleanly and show an error');

  await page.reload();
  await page.waitForFunction(() => document.querySelector('.dock-art')?.disabled === false);
  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '正在播放' }).click();
  await page.waitForFunction(() => document.querySelector('[aria-label="封面背景模糊程度"]')?.value === '25');
  await page.waitForFunction(() => document.querySelector('[aria-label="元件底色透明度"]')?.value === '48');
  await page.evaluate(() => { window.__appearanceStep = 'preferences restored'; });
  await page.locator('.dock-art').click();
  await page.evaluate(() => { window.__appearanceStep = 'dock clicked'; });
  await page.waitForFunction(() => document.querySelector('.now-playing-overlay')?.classList.contains('is-open'));
  await page.evaluate(() => { window.__appearanceStep = 'overlay open'; });
  await page.waitForFunction(() => document.querySelector('.now-playing-backdrop img')?.complete && document.querySelector('.now-playing-backdrop img')?.naturalWidth === 800);

  const sharedArtwork = await page.evaluate(() => ({
    backgroundUrl: document.querySelector('.now-playing-backdrop img')?.src,
    coverUrl: document.querySelector('.cover-stage-image')?.src,
    dockUrl: document.querySelector('.dock-art-image')?.src,
    layerCount: document.querySelectorAll('.now-playing-backdrop').length,
    filter: getComputedStyle(document.querySelector('.now-playing-backdrop img')).filter,
    z: {
      backdrop: Number.parseInt(getComputedStyle(document.querySelector('.now-playing-backdrop')).zIndex, 10) || 0,
      sidebar: Number.parseInt(getComputedStyle(document.querySelector('.sidebar')).zIndex, 10) || 0,
      dock: Number.parseInt(getComputedStyle(document.querySelector('.player-dock')).zIndex, 10) || 0,
      overlay: Number.parseInt(getComputedStyle(document.querySelector('.now-playing-overlay')).zIndex, 10) || 0,
    },
  }));
  assert.equal(sharedArtwork.backgroundUrl, sharedArtwork.coverUrl, 'full-screen background should reuse the Now Playing artwork URL');
  assert.equal(sharedArtwork.backgroundUrl, sharedArtwork.dockUrl, 'dock should reuse the same active artwork URL');
  assert.equal(sharedArtwork.layerCount, 1, 'render exactly one full-screen background layer');
  assert.equal(sharedArtwork.filter, 'blur(25px)', 'loaded preferences should set the backdrop blur');
  assert.ok(sharedArtwork.z.backdrop > sharedArtwork.z.sidebar, 'backdrop must cover the mobile sidebar');
  assert.ok(sharedArtwork.z.overlay > sharedArtwork.z.backdrop, 'Now Playing overlay must remain above the backdrop');
  assert.ok(sharedArtwork.z.dock > sharedArtwork.z.backdrop, 'dock must remain above the backdrop');

  const viewports = [
    [3840, 2160], [2560, 1440], [1920, 1080], [1600, 900],
    [1366, 768], [1280, 1024], [1024, 768], [900, 900],
    [800, 1200], [720, 1280], [412, 915], [360, 800],
  ];
  const geometries = [];
  for (const [width, height] of viewports) {
    await page.setViewportSize({ width, height });
    await wait(25);
    const geometry = await page.evaluate(() => {
      const backdrop = document.querySelector('.now-playing-backdrop');
      const dock = document.querySelector('.player-dock');
      const button = document.querySelector('.player-dock .play-button');
      const slider = document.querySelector('.player-dock .volume-slider');
      const bounds = element => {
        const rect = element.getBoundingClientRect();
        return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, width: rect.width, height: rect.height };
      };
      const centerHit = element => {
        const rect = element.getBoundingClientRect();
        const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
        return Boolean(hit && (hit === element || element.contains(hit)));
      };
      return {
        viewport: [innerWidth, innerHeight],
        document: [document.documentElement.scrollWidth, document.documentElement.scrollHeight],
        backdrop: bounds(backdrop),
        pointerEvents: getComputedStyle(backdrop).pointerEvents,
        dock: bounds(dock),
        buttonHit: centerHit(button),
        sliderHit: centerHit(slider),
      };
    });
    assert.ok(geometry.backdrop.left <= 0 && geometry.backdrop.top <= 0, `${width}x${height}: backdrop leaves an upper edge uncovered`);
    assert.ok(geometry.backdrop.right >= width && geometry.backdrop.bottom >= height, `${width}x${height}: backdrop does not reach every viewport edge`);
    assert.deepEqual(geometry.document, [width, height], `${width}x${height}: full-screen background caused page overflow`);
    assert.equal(geometry.pointerEvents, 'none', `${width}x${height}: backdrop must let pointer input pass through`);
    assert.ok(geometry.buttonHit && geometry.sliderHit, `${width}x${height}: dock playback controls are blocked`);
    assert.ok(geometry.dock.bottom <= height + 1, `${width}x${height}: dock is clipped`);
    geometries.push(`${width}x${height}`);
  }

  await page.setViewportSize({ width: 1600, height: 900 });
  await wait(100);
  await page.screenshot({ path: 'C:/APP/@Audio/MoeMusicPlayer/target/now-playing-appearance-light-cover.png' });

  await page.locator('.now-playing-backdrop img').dispatchEvent('error');
  await page.waitForFunction(() => document.querySelector('.now-playing-backdrop') && !document.querySelector('.now-playing-backdrop img'));
  const fallback = await page.evaluate(() => ({
    background: getComputedStyle(document.querySelector('.now-playing-backdrop')).backgroundColor,
    buttonHit: (() => {
      const button = document.querySelector('.player-dock .play-button');
      const rect = button.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2);
      return Boolean(hit && (hit === button || button.contains(hit)));
    })(),
  }));
  assert.ok(fallback.background !== 'rgba(0, 0, 0, 0)', 'cover failure should keep a theme-colored fallback layer');
  assert.ok(fallback.buttonHit, 'fallback layer must not block playback controls');
  await page.locator('.dock-art').click();
  await page.waitForFunction(() => !document.querySelector('.now-playing-backdrop'));
  const closed = await page.evaluate(() => ({
    activeClass: document.querySelector('.app-shell').classList.contains('has-now-playing-backdrop'),
    dockBackground: getComputedStyle(document.querySelector('.player-dock')).backgroundImage,
  }));
  assert.equal(closed.activeClass, false, 'closing the page should remove the background layer');
  assert.ok(closed.dockBackground.startsWith('linear-gradient'), 'closing the page should restore the ordinary dock background');

  return { result: 'PASS', savedAppearance: latest, checkedViewports: geometries, screenshot: 'target/now-playing-appearance-light-cover.png', fallback: fallback.background };
}
