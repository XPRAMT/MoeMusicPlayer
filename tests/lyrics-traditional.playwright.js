async page => {
  const assert = {
    equal(actual, expected, message) {
      if (actual !== expected) throw new Error(`${message}: expected ${expected}, got ${actual}`);
    },
  };
  await page.setViewportSize({ width: 1400, height: 900 });
  await page.goto('http://127.0.0.1:1450/tests/volume-slider-harness.html');
  await page.getByRole('button', { name: '設定', exact: true }).click();
  await page.getByRole('tab', { name: '歌詞' }).click();
  const toggle = page.getByRole('checkbox', { name: '簡體轉繁體' });
  await toggle.waitFor();
  assert.equal(await toggle.isChecked(), false, 'conversion starts off');
  await toggle.check();
  await page.waitForFunction(() => {
    const saved = window.__volumeHarness && document.body;
    return document.querySelector('#lyrics-panel [role="status"]')?.textContent?.includes('瀏覽器預覽')
      || document.querySelector('#lyrics-panel [role="status"]')?.textContent?.includes('歌詞設定已保存');
  });
  assert.equal(await toggle.isChecked(), true, 'conversion toggle stays on');
  await page.locator('.dock-art').click();
  await page.waitForFunction(() => document.querySelector('.now-playing-overlay')?.classList.contains('is-open'));
  await page.waitForFunction(() => (document.querySelector('.lyric-primary')?.textContent || '').includes('晨光'));
  const primary = await page.locator('.lyric-primary').first().innerText();
  assert.equal(primary.includes('晨光'), true, 'lyrics still render after conversion is enabled');
  return { primary };
}
