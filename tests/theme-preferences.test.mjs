import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_THEME_PREFERENCES,
  compositeBackgroundHex,
  contrastRatio,
  contrastingTextColor,
  createThemeCssVariables,
  normalizeThemePreferences,
  applyTheme,
} from '../src/lib/theme.js';

test('default appearance uses pure black and a water-blue accent', () => {
  assert.deepEqual(DEFAULT_THEME_PREFERENCES, {
    backgroundHex: '#000000',
    accentHex: '#55D9FF',
    quickSettingsOpacityPercent: 70,
    quickSettingsBlurPx: 12,
    mainBackgroundBlurPx: 20,
    mainBackgroundOpacityPercent: 40,
  });
});

test('text color automatically reaches WCAG AA contrast on light and dark backgrounds', () => {
  for (const background of ['#000000', '#FFFFFF', '#55D9FF', '#7A3CE7', '#A8B0B8']) {
    const text = contrastingTextColor(background);
    assert.ok(contrastRatio(background, text) >= 4.5, `${text} on ${background} must reach 4.5:1`);
  }
});

test('theme variables follow both selected colors and derive coordinated surfaces', () => {
  const variables = createThemeCssVariables({ backgroundHex: '#18283A', accentHex: '#D84A9A' });
  assert.equal(variables['--page'], '#18283A');
  assert.equal(variables['--accent'], '#D84A9A');
  assert.equal(variables['--text'], '#FFFFFF');
  assert.notEqual(variables['--panel'], variables['--page']);
  assert.ok(contrastRatio('#D84A9A', variables['--accent-foreground']) >= 4.5);
});

test('composite background mixes page color with image average by opacity', () => {
  assert.equal(compositeBackgroundHex('#000000', '#FFFFFF', 0), '#000000');
  assert.equal(compositeBackgroundHex('#000000', '#FFFFFF', 100), '#FFFFFF');
  assert.equal(compositeBackgroundHex('#000000', '#FFFFFF', 40), '#666666');
  assert.equal(compositeBackgroundHex('#102030', null, 80), '#102030');
});

test('contrast background can flip text while keeping the solid page color', () => {
  const variables = createThemeCssVariables(
    { backgroundHex: '#000000', accentHex: '#55D9FF' },
    { contrastBackgroundHex: '#E8E8E8' },
  );
  assert.equal(variables['--page'], '#000000');
  assert.equal(variables['--text'], '#000000');
  assert.equal(variables['--text-inverse'], '#FFFFFF');
  assert.equal(variables['--text-inverse-rgb'], '255, 255, 255');
  assert.equal(variables['--color-scheme'], 'light');
  assert.ok(contrastRatio(variables['--panel'], variables['--text']) >= 4.5);
});

test('text inverse is always the opposite of the chosen text color', () => {
  const dark = createThemeCssVariables({ backgroundHex: '#000000', accentHex: '#55D9FF' });
  assert.equal(dark['--text'], '#FFFFFF');
  assert.equal(dark['--text-inverse'], '#000000');
  const light = createThemeCssVariables({ backgroundHex: '#FFFFFF', accentHex: '#55D9FF' });
  assert.equal(light['--text'], '#000000');
  assert.equal(light['--text-inverse'], '#FFFFFF');
});

test('accent and status text colors retain readable contrast on either background', () => {
  const backgrounds = ['#FFFFFF', '#000000', '#55D9FF', '#4A226E'];
  for (let channel = 0; channel <= 255; channel += 1) {
    const value = channel.toString(16).padStart(2, '0');
    backgrounds.push(`#${value}${value}${value}`);
  }
  for (const backgroundHex of backgrounds) {
    const variables = createThemeCssVariables({ backgroundHex, accentHex: '#55D9FF' });
    const surfaces = ['--page', '--panel', '--panel-soft', '--sidebar-bg', '--dock-bg']
      .map((name) => variables[name]);
    for (const name of ['--text', '--text-soft', '--muted', '--quiet', '--accent-text', '--status-danger', '--status-success', '--status-warning']) {
      for (const surface of surfaces) {
        assert.ok(
          contrastRatio(surface, variables[name]) >= 4.5,
          `${name} must remain readable against ${surface} derived from ${backgroundHex}`,
        );
      }
    }
    const expectedColorScheme = contrastRatio(backgroundHex, '#000000') > contrastRatio(backgroundHex, '#FFFFFF')
      ? 'light'
      : 'dark';
    assert.equal(variables['--color-scheme'], expectedColorScheme);
  }
});

test('low-luminance accents keep labels and filled-control text readable', () => {
  for (const backgroundHex of ['#000000', '#FFFFFF', '#636363']) {
    for (const accentHex of ['#000000', '#03060B', '#171717', '#FFFFFF', '#55D9FF']) {
      const variables = createThemeCssVariables({ backgroundHex, accentHex });
      const surfaces = ['--page', '--panel', '--panel-soft', '--sidebar-bg', '--dock-bg']
        .map((name) => variables[name]);
      for (const surface of surfaces) {
        assert.ok(contrastRatio(surface, variables['--accent-text']) >= 4.5);
      }
      assert.ok(contrastRatio(accentHex, variables['--accent-foreground']) >= 4.5);
    }
  }
});

test('applyTheme writes every computed preference to the document root', () => {
  const written = new Map();
  const root = { style: { setProperty: (name, value) => written.set(name, value) } };
  const variables = applyTheme(root, { backgroundHex: '#FFFFFF', accentHex: '#03060B' });
  assert.equal(written.get('--page'), '#FFFFFF');
  assert.equal(written.get('--color-scheme'), 'light');
  assert.equal(written.get('--accent'), '#03060B');
  assert.deepEqual(Object.fromEntries(written), variables);
});

test('invalid persisted values fall back to safe defaults', () => {
  assert.deepEqual(normalizeThemePreferences({ backgroundHex: 'red', accentHex: '#fff' }), {
    backgroundHex: '#000000',
    accentHex: '#55D9FF',
    quickSettingsOpacityPercent: 70,
    quickSettingsBlurPx: 12,
    mainBackgroundBlurPx: 20,
    mainBackgroundOpacityPercent: 40,
  });
  assert.equal(normalizeThemePreferences({ quickSettingsOpacityPercent: 140 }).quickSettingsOpacityPercent, 100);
  assert.equal(normalizeThemePreferences({ quickSettingsOpacityPercent: -4 }).quickSettingsOpacityPercent, 0);
  assert.equal(normalizeThemePreferences({ quickSettingsBlurPx: 80 }).quickSettingsBlurPx, 40);
  assert.equal(normalizeThemePreferences({ mainBackgroundBlurPx: 80 }).mainBackgroundBlurPx, 40);
  assert.equal(normalizeThemePreferences({ mainBackgroundOpacityPercent: -2 }).mainBackgroundOpacityPercent, 0);
  assert.equal(normalizeThemePreferences({ mainBackgroundBrightnessPercent: 55 }).mainBackgroundOpacityPercent, 55);
});
