async page => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  await page.setViewportSize({ width: 900, height: 720 });
  await page.addInitScript(() => {
    const state = { mainThreadDraws: 0, maximumHeartbeatGapMs: 0, lastHeartbeat: performance.now(), longTasks: [] };
    Object.assign(window, { __thumbnailPerf: state });
    const originalDrawImage = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      state.mainThreadDraws += 1;
      const until = performance.now() + 140;
      while (performance.now() < until) { /* inject a deterministic cold thumbnail resize cost */ }
      return originalDrawImage.apply(this, args);
    };
    if ('PerformanceObserver' in window) {
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) state.longTasks.push(entry.duration);
      }).observe({ type: 'longtask', buffered: true });
    }
    window.setInterval(() => {
      const now = performance.now();
      state.maximumHeartbeatGapMs = Math.max(state.maximumHeartbeatGapMs, now - state.lastHeartbeat);
      state.lastHeartbeat = now;
    }, 16);
  });
  await page.goto('http://127.0.0.1:1450/tests/album-index-harness.html?cold', { waitUntil: 'domcontentloaded', timeout: 30000 });
  await page.waitForFunction(() => window.__albumIndexStats.thumbnailWrites > 0, null, { timeout: 30000 });
  await page.waitForTimeout(120);
  const result = await page.evaluate(() => ({
    stats: { ...window.__albumIndexStats },
    perf: { ...window.__thumbnailPerf, longTasks: [...window.__thumbnailPerf.longTasks] },
  }));
  assert(result.stats.rawArtworkReads > 0, `cold-cache path must read original artwork: ${JSON.stringify(result)}`);
  assert(result.stats.thumbnailWrites > 0, `cold-cache path must produce and save a thumbnail: ${JSON.stringify(result)}`);
  assert(result.perf.mainThreadDraws === 0, `thumbnail resize must not draw on the UI thread: ${JSON.stringify(result)}`);
  assert(result.perf.longTasks.every((duration) => duration < 100), `cold thumbnail work caused a long UI task: ${JSON.stringify(result)}`);
  assert(result.perf.maximumHeartbeatGapMs < 100, `UI heartbeat stalled during cold thumbnail work: ${JSON.stringify(result)}`);
  return result;
}
