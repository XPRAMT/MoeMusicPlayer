// The continuous strip of the lyric windows is drawn natively (ADR 0034): its rules live in crates/player-lyrics-rules
// (the reference, which generates the shared vectors) and Kotlin FloatingLyricsRules / WordLyricsRules. These checks
// pin how the renderers use them and the animation setting the main window normalises.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { normalizeLyricsAnimation } from '../src/lib/taskbar-lyrics-settings.js';

test('the animation setting falls back to none', () => {
  assert.equal(normalizeLyricsAnimation('continuous'), 'continuous');
  assert.equal(normalizeLyricsAnimation('none'), 'none');
  assert.equal(normalizeLyricsAnimation('bounce'), 'none');
  assert.equal(normalizeLyricsAnimation(undefined), 'none');
});

test('no JS copy of the strip rules is left', () => {
  for (const file of ['lyrics-strip.js', 'lyrics-word-strip.js', 'east-asian-width.js']) {
    assert.throws(() => readFileSync(new URL(`../src/lib/${file}`, import.meta.url)), file);
  }
  const glide = readFileSync(new URL('../src/lib/lyrics-glide.js', import.meta.url), 'utf8');
  assert.doesNotMatch(glide, /wordStripKnots|knotCubic/);
});

test('the renderers anchor the continuous strip on the lyric area centre whatever the alignment setting', async () => {
  const { readFileSync } = await import('node:fs');
  // Windows draws natively (ADR 0034): the plan anchors on STRIP_ALIGN / STRIP_TARGET, never the alignment setting.
  const plan = readFileSync(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  assert.match(plan, /rules::STRIP_ALIGN,[\s\S]*rules::STRIP_TARGET,/);
  assert.doesNotMatch(plan, /align: Align|look\.align/);
  const kotlin = readFileSync(new URL('../plugins/tauri-plugin-media-index/android/src/main/java/com/moemusicplayer/mediaindex/LyricsFramePlanner.kt', import.meta.url), 'utf8');
  assert.match(kotlin, /FloatingLyricsRules\.STRIP_ALIGN, arrival,\s*strip\.mainWidths, strip\.lastGlyphCenters/);
  assert.match(kotlin, /stripTarget\(areaW, FloatingLyricsRules\.STRIP_TARGET\)/);
});

test('the native renderer draws only the strip window', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  assert.match(source, /rules::strip_window\(/);
});

test('continuous mode dims by the static mask only, fed by each window own main and secondary opacity', () => {
  // ADR 0041 D10: no per-line fades; the mask takes main (plateau) and secondary (where the edge ramp starts).
  const plan = readFileSync(new URL('../crates/player-lyrics-native/src/plan.rs', import.meta.url), 'utf8');
  const bridge = readFileSync(new URL('../src-tauri/src/native_lyrics.rs', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../src/App.svelte', import.meta.url), 'utf8');
  const win = readFileSync(new URL('../crates/player-lyrics-native/src/win.rs', import.meta.url), 'utf8');
  assert.doesNotMatch(plan, /rules::line_opacity_factor\(/);
  assert.match(win, /rules::centre_mask_stops\(\s*area\.width,\s*em,\s*f64::from\(main\) \/ 100\.0,\s*f64::from\(secondary\) \/ 100\.0,?\s*\)/);
  // No separate shared channel any more: each window's own settings carry both values.
  assert.doesNotMatch(bridge, /lyrics-inactive-opacity/);
  assert.doesNotMatch(app, /lyrics-inactive-opacity/);
  assert.doesNotMatch(app, /inactiveOpacityPercent/);
  assert.match(bridge, /main_opacity_percent: f64::from\(settings\.main_opacity_percent\)/);
  assert.match(bridge, /secondary_opacity_percent: f64::from\(settings\.secondary_opacity_percent\)/);
});
