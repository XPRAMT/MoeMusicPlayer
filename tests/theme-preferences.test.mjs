import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEFAULT_THEME_PREFERENCES,
  compositeBackgroundHex,
  contrastRatio,
  contrastingTextColor,
  createThemeCssVariables,
  normalizeThemePreferences,
  relativeLuminance,
  applyTheme,
} from '../src/lib/theme.js';

test('default appearance uses pure black and a water-blue accent', () => {
  assert.deepEqual(DEFAULT_THEME_PREFERENCES, {
    backgroundHex: '#000000',
    accentHex: '#55D9FF',
    colorMode: 'autoTextColor',
    backgroundOpacityTextColor: 'white',
    quickSettingsOpacityPercent: 70,
    quickSettingsBlurPx: 12,
    mainBackgroundBlurPx: 20,
    mainBackgroundOpacityPercent: 40,
    mainBackgroundMode: 'solid',
    slideshowFolder: null,
    slideshowIntervalMinutes: 10,
    slideshowOrder: 'sequential',
    autoTextColorThresholdPercent: 50,
  });
});

test('text color follows relative-luminance Y against the threshold', () => {
  assert.equal(contrastingTextColor('#000000', 50), '#FFFFFF');
  assert.equal(contrastingTextColor('#FFFFFF', 50), '#000000');
  assert.ok(relativeLuminance('#808080') < 0.5);
  assert.equal(contrastingTextColor('#808080', 50), '#FFFFFF');
  assert.equal(contrastingTextColor('#808080', 10), '#000000');
  // Threshold 100%: only Y >= 1 (pure white) uses black text.
  assert.equal(contrastingTextColor('#FFFFFF', 100), '#000000');
  assert.equal(contrastingTextColor('#FEFEFE', 100), '#FFFFFF');
  // Threshold 0%: every color uses black text.
  assert.equal(contrastingTextColor('#000000', 0), '#000000');
  assert.equal(contrastingTextColor('#FFFFFF', 0), '#000000');
});

test('theme variables follow both selected colors and derive coordinated surfaces', () => {
  const variables = createThemeCssVariables({ backgroundHex: '#18283A', accentHex: '#D84A9A' });
  assert.equal(variables['--page'], '#18283A');
  assert.equal(variables['--accent'], '#D84A9A');
  assert.equal(variables['--text'], '#FFFFFF');
  assert.notEqual(variables['--panel'], variables['--page']);
  assert.equal(
    variables['--accent-foreground'],
    contrastingTextColor('#D84A9A', DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent),
  );
});

test('automatic background opacity fixes page and text colors without overwriting the saved custom background', () => {
  const preferences = {
    ...DEFAULT_THEME_PREFERENCES,
    backgroundHex: '#203040',
    colorMode: 'autoBackgroundOpacity',
    backgroundOpacityTextColor: 'black',
  };
  const variables = createThemeCssVariables(preferences, { pageBackgroundHex: '#FFFFFF' });
  assert.equal(variables['--page'], '#FFFFFF');
  assert.equal(variables['--text'], '#000000');
  assert.equal(normalizeThemePreferences(preferences).backgroundHex, '#203040');

  const whiteTextVariables = createThemeCssVariables(
    { ...preferences, backgroundOpacityTextColor: 'white' },
    { pageBackgroundHex: '#000000' },
  );
  assert.equal(whiteTextVariables['--page'], '#000000');
  assert.equal(whiteTextVariables['--text'], '#FFFFFF');
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

test('threshold preference switches text without using contrast comparison', () => {
  const low = createThemeCssVariables({
    backgroundHex: '#808080',
    accentHex: '#55D9FF',
    autoTextColorThresholdPercent: 10,
  });
  assert.equal(low['--text'], '#000000');
  assert.equal(low['--color-scheme'], 'light');
  const high = createThemeCssVariables({
    backgroundHex: '#808080',
    accentHex: '#55D9FF',
    autoTextColorThresholdPercent: 50,
  });
  assert.equal(high['--text'], '#FFFFFF');
  assert.equal(high['--color-scheme'], 'dark');
});

test('color scheme follows the Y threshold for every gray background', () => {
  for (let channel = 0; channel <= 255; channel += 1) {
    const value = channel.toString(16).padStart(2, '0');
    const backgroundHex = `#${value}${value}${value}`;
    const variables = createThemeCssVariables({ backgroundHex, accentHex: '#55D9FF' });
    const expectedText = relativeLuminance(backgroundHex)
      >= DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent / 100
      ? '#000000'
      : '#FFFFFF';
    assert.equal(variables['--text'], expectedText, backgroundHex);
    assert.equal(variables['--color-scheme'], expectedText === '#000000' ? 'light' : 'dark');
    assert.equal(variables['--text-inverse'], expectedText === '#000000' ? '#FFFFFF' : '#000000');
  }
});

test('low-luminance accents keep filled-control text on the threshold rule', () => {
  for (const accentHex of ['#000000', '#03060B', '#171717', '#FFFFFF', '#55D9FF', '#D84A9A']) {
    const variables = createThemeCssVariables({ backgroundHex: '#000000', accentHex });
    assert.equal(
      variables['--accent-foreground'],
      contrastingTextColor(accentHex, DEFAULT_THEME_PREFERENCES.autoTextColorThresholdPercent),
    );
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
    colorMode: 'autoTextColor',
    backgroundOpacityTextColor: 'white',
    quickSettingsOpacityPercent: 70,
    quickSettingsBlurPx: 12,
    mainBackgroundBlurPx: 20,
    mainBackgroundOpacityPercent: 40,
    mainBackgroundMode: 'solid',
    slideshowFolder: null,
    slideshowIntervalMinutes: 10,
    slideshowOrder: 'sequential',
    autoTextColorThresholdPercent: 50,
  });
  assert.equal(normalizeThemePreferences({ quickSettingsOpacityPercent: 140 }).quickSettingsOpacityPercent, 100);
  assert.equal(normalizeThemePreferences({ quickSettingsOpacityPercent: -4 }).quickSettingsOpacityPercent, 0);
  assert.equal(normalizeThemePreferences({ quickSettingsBlurPx: 80 }).quickSettingsBlurPx, 50);
  assert.equal(normalizeThemePreferences({ mainBackgroundBlurPx: 80 }).mainBackgroundBlurPx, 50);
  assert.equal(normalizeThemePreferences({ mainBackgroundOpacityPercent: -2 }).mainBackgroundOpacityPercent, 0);
  assert.equal('mainBackgroundZoomPercent' in normalizeThemePreferences({ mainBackgroundZoomPercent: 7 }), false);
  assert.equal(normalizeThemePreferences({ mainBackgroundBrightnessPercent: 55 }).mainBackgroundOpacityPercent, 55);
  assert.equal(normalizeThemePreferences({ mainBackgroundMode: 'unknown' }).mainBackgroundMode, 'solid');
  assert.equal(normalizeThemePreferences({ mainBackgroundMode: 'albumCover' }).mainBackgroundMode, 'albumCover');
  assert.equal(normalizeThemePreferences({ slideshowIntervalMinutes: 0 }).slideshowIntervalMinutes, 1);
  assert.equal(normalizeThemePreferences({ slideshowIntervalMinutes: 180 }).slideshowIntervalMinutes, 120);
  assert.equal(normalizeThemePreferences({ slideshowOrder: 'random' }).slideshowOrder, 'random');
  assert.equal(normalizeThemePreferences({ autoTextColorThresholdPercent: 140 }).autoTextColorThresholdPercent, 100);
  assert.equal(normalizeThemePreferences({ autoTextColorThresholdPercent: -3 }).autoTextColorThresholdPercent, 0);
});
