import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('the progress bar and the marquees run linear (sub-pixel)', () => {
  const progress = read('src/lib/PlaybackProgress.svelte');
  const run = read('src/lib/progress-run.js');
  // One linear compositor animation per run, no stepping and no per-frame JS.
  assert.match(run, /easing: 'linear'/);
  assert.match(run, /fill: .*'forwards'/);
  assert.doesNotMatch(progress + run, /steps\(/);
  assert.doesNotMatch(run, /requestAnimationFrame|setInterval|setTimeout/);
  // Only transform is animated: no custom property, width, left or gradient stops.
  assert.match(run, /transform: from\.thumb/);
  assert.doesNotMatch(run, /\bleft\b\s*:|\bwidth\b\s*:|--progress/);
  const marquee = read('src/lib/dock-marquee.ts');
  assert.doesNotMatch(marquee, /animationTimingFunction|steps\(/);
  const css = read('src/app.css');
  assert.match(css, /animation: dock-marquee-scroll 9s linear infinite/);
  // Both pauses hold a single transform, so they present nothing.
  assert.match(css, /0%,\s*12% \{\s*transform: translateX\(0\);\s*\}\s*88%,\s*100% \{/);
});

test('the progress bar moves by compositor transforms only', async () => {
  const { progressTransforms, progressRunKeyframes, PROGRESS_BAND_SPAN } = await import('../src/lib/progress-run.js');
  assert.equal(PROGRESS_BAND_SPAN, 3);
  assert.deepEqual(progressTransforms(0), { thumb: 'translateX(0%)', band: 'translateX(0%)' });
  assert.deepEqual(progressTransforms(0.25), { thumb: 'translateX(25%)', band: 'translateX(8.33333%)' });
  assert.deepEqual(progressTransforms(1), { thumb: 'translateX(100%)', band: 'translateX(33.33333%)' });
  // Out-of-range and malformed ratios are clamped.
  assert.equal(progressTransforms(-1).thumb, 'translateX(0%)');
  assert.equal(progressTransforms(7).thumb, 'translateX(100%)');
  assert.equal(progressTransforms(Number.NaN).thumb, 'translateX(0%)');
  const keyframes = progressRunKeyframes(0.5);
  assert.deepEqual(keyframes.thumb, [{ transform: 'translateX(50%)' }, { transform: 'translateX(100%)' }]);
  assert.deepEqual(keyframes.band, [{ transform: 'translateX(16.66667%)' }, { transform: 'translateX(33.33333%)' }]);
  // The band is 3 x the track and centred on its left edge: ratio x track = ratio / 3 x band width.
  const css = read('src/app.css');
  assert.match(css, /\.progress-band \{[^}]*left: -150%;[^}]*width: 300%;/);
});

test('the progress run starts at the current ratio, restarts only on a changed run, and parks when paused', async () => {
  const { progressRun } = await import('../src/lib/progress-run.js');
  const made = [];
  const element = () => {
    const el = { style: {}, animate(keyframes, timing) { const a = { keyframes, timing, cancelled: false, cancel() { this.cancelled = true; } }; made.push(a); return a; } };
    return el;
  };
  const thumb = element();
  const band = element();
  const node = { animate() {}, querySelector: (selector) => (selector === '.progress-thumb-mover' ? thumb : band) };
  const action = progressRun(node, { ratio: 0.25, remainingMs: 7500, key: 'a' });
  assert.equal(made.length, 2);
  assert.equal(thumb.style.transform, 'translateX(25%)');
  assert.equal(made[0].timing.duration, 7500);
  assert.equal(made[0].timing.easing, 'linear');
  assert.equal(made[0].keyframes[0].transform, 'translateX(25%)');
  assert.equal(made[0].keyframes[1].transform, 'translateX(100%)');
  // The same run again (a re-render) leaves the animations alone.
  action.update({ ratio: 0.25, remainingMs: 7500, key: 'a' });
  assert.equal(made.length, 2);
  assert.equal(made[0].cancelled, false);
  // A restart (new key) cancels the old animations and starts new ones.
  action.update({ ratio: 0.5, remainingMs: 5000, key: 'b' });
  assert.equal(made.length, 4);
  assert.equal(made[0].cancelled && made[1].cancelled, true);
  // Paused or dragging: parked at the ratio, no animation.
  action.update({ ratio: 0.75, remainingMs: null, key: 'parked' });
  assert.equal(made.length, 4);
  assert.equal(made[2].cancelled && made[3].cancelled, true);
  assert.equal(thumb.style.transform, 'translateX(75%)');
  assert.equal(band.style.transform, 'translateX(25%)');
  action.destroy();
});
