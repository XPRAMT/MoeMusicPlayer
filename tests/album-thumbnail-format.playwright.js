async page => {
  await page.goto('http://127.0.0.1:1450/tests/album-thumbnail-format-harness.html', {
    waitUntil: 'domcontentloaded',
    timeout: 30000,
  });
  await page.waitForFunction(() => window.__albumThumbnailFormatResult !== undefined, null, { timeout: 30000 });
  const result = await page.evaluate(() => window.__albumThumbnailFormatResult);
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  assert(result.size === 512, `thumbnail default must be 512px: ${JSON.stringify(result)}`);
  assert(result.mainDimensions[0] === 512 && result.mainDimensions[1] === 512, `non-worker thumbnail must decode as 512x512: ${JSON.stringify(result)}`);
  assert(result.workerDimensions[0] === 512 && result.workerDimensions[1] === 512, `worker thumbnail must decode as 512x512: ${JSON.stringify(result)}`);
  assert(result.mainMatchesQuality090, `non-worker output must match browser JPEG quality 0.90 encoding: ${JSON.stringify(result)}`);
  assert(result.workerMatchesQuality090, `worker output must match browser JPEG quality 0.90 encoding: ${JSON.stringify(result)}`);
  assert(result.mainDiffersFromQuality100 && result.workerDiffersFromQuality100, `both outputs must differ from quality 1.0 encoding: ${JSON.stringify(result)}`);
  assert(result.mainDiffersFromQuality088 && result.workerDiffersFromQuality088, `both outputs must differ from quality 0.88 encoding: ${JSON.stringify(result)}`);
  assert(result.workerBase64MatchesBytes, `worker Base64 payload must represent the exact generated thumbnail bytes: ${JSON.stringify(result)}`);
  return result;
}
