import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import {
  APP_FONT_STACK_CSS,
  DEFAULT_INTERFACE_FONT_STACK,
  FONT_FAMILY_MAX_CHARS,
  androidFontAlias,
  androidFontFaceSources,
  applyFontListUpdate,
  filterFonts,
  fontDisplayName,
  fontFamilyCss,
  fontPickerRows,
  normalizeFontFamily,
  quoteCssFamily,
  WINDOWS_NATIVE_LYRICS_CJK_FAMILY,
  cssStackFamilies,
  defaultFontFamily,
  fontPickerGroups,
} from '../src/lib/font-picker.js';
import { DEFAULT_LYRICS_PREFERENCES, normalizeLyricsPreferences } from '../src/lib/lyrics-preferences.js';
import { DEFAULT_TASKBAR_LYRICS_SETTINGS, normalizeTaskbarLyricsSettings } from '../src/lib/taskbar-lyrics-settings.js';
import { DEFAULT_FLOATING_LYRICS_SETTINGS, normalizeFloatingLyricsSettings } from '../src/lib/floating-lyrics-settings.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const read = (file) => readFileSync(path.join(root, file), 'utf8');

const face = (file, weight = 400, italic = false) => ({ path: `C:/Windows/Fonts/${file}`, faceIndex: 0, weight, italic });
const fonts = [
  { family: 'Microsoft JhengHei', displayName: '微軟正黑體', group: 'traditional', faces: [face('msjh.ttc'), face('msjhbd.ttc', 700)] },
  { family: 'Yu Gothic', displayName: '游ゴシック', group: 'japanese', faces: [face('yugothic.ttf')] },
  { family: 'Segoe UI', displayName: 'Segoe UI', group: 'latin', faces: [face('segoeui.ttf'), face('segoeuii.ttf', 400, true)] },
  { family: 'ＭＳ ゴシック', displayName: 'MS Gothic', group: 'japanese', faces: [face('msgothic.ttc')] },
];

test('every font setting defaults to the default font (null) and keeps only a usable name', () => {
  assert.equal(DEFAULT_LYRICS_PREFERENCES.fontFamily, null);
  assert.equal(DEFAULT_TASKBAR_LYRICS_SETTINGS.fontFamily, null);
  assert.equal(DEFAULT_FLOATING_LYRICS_SETTINGS.fontFamily, null);
  for (const normalize of [normalizeLyricsPreferences, normalizeTaskbarLyricsSettings, normalizeFloatingLyricsSettings]) {
    assert.equal(normalize({}).fontFamily, null, 'an old file without the field');
    assert.equal(normalize({ fontFamily: '  Yu Gothic ' }).fontFamily, 'Yu Gothic');
    assert.equal(normalize({ fontFamily: '' }).fontFamily, null);
    assert.equal(normalize({ fontFamily: 3 }).fontFamily, null);
    assert.equal(normalize({ fontFamily: 'Not Installed' }).fontFamily, 'Not Installed', 'a missing font is kept');
  }
  assert.equal(normalizeFontFamily('x'.repeat(FONT_FAMILY_MAX_CHARS)), 'x'.repeat(FONT_FAMILY_MAX_CHARS));
  assert.equal(normalizeFontFamily('x'.repeat(FONT_FAMILY_MAX_CHARS + 1)), null);
  assert.equal(normalizeFontFamily(null), null);
});

test('a family becomes a quoted CSS list that falls back to the default stack', () => {
  assert.equal(quoteCssFamily('Yu Gothic'), '"Yu Gothic"');
  assert.equal(quoteCssFamily('a"b\\c'), '"a\\"b\\\\c"');
  assert.equal(fontFamilyCss(null, DEFAULT_INTERFACE_FONT_STACK), DEFAULT_INTERFACE_FONT_STACK);
  assert.equal(fontFamilyCss('  ', APP_FONT_STACK_CSS), APP_FONT_STACK_CSS);
  assert.equal(fontFamilyCss('微軟正黑體', DEFAULT_INTERFACE_FONT_STACK), `"微軟正黑體", ${DEFAULT_INTERFACE_FONT_STACK}`);
  // Android first tries the @font-face alias registered from the font's files.
  assert.equal(
    fontFamilyCss('Noto Sans TC', APP_FONT_STACK_CSS, { android: true }),
    `"${androidFontAlias('Noto Sans TC')}", "Noto Sans TC", var(--app-font-stack)`,
  );
});

test('search matches the shown and the stored name, ignoring case, width and spaces', () => {
  const names = (query) => filterFonts(fonts, query).map((font) => font.family);
  assert.deepEqual(names(''), fonts.map((font) => font.family));
  assert.deepEqual(names('正黑'), ['Microsoft JhengHei']);
  assert.deepEqual(names('jhenghei'), ['Microsoft JhengHei']);
  assert.deepEqual(names('micro soft'), ['Microsoft JhengHei']);
  assert.deepEqual(names('ms ゴシック'), ['ＭＳ ゴシック'], 'full-width letters match half-width ones');
  assert.deepEqual(names('segoeui'), ['Segoe UI']);
  assert.deepEqual(names('nothing'), []);
  // The backend's order is kept.
  assert.deepEqual(names('o'), ['Microsoft JhengHei', 'Yu Gothic', 'Segoe UI', 'ＭＳ ゴシック']);
});

test('「預設字型」 is always the first row, also while searching', () => {
  const rows = fontPickerRows(fonts, 'yu');
  assert.equal(rows[0].family, null);
  assert.equal(rows[0].font, null);
  assert.deepEqual(rows.slice(1).map((row) => row.family), ['Yu Gothic']);
  assert.equal(new Set(fontPickerRows(fonts, '').map((row) => row.key)).size, fonts.length + 1, 'unique keys');
});

test('group headings follow the backend order and hide groups without a match while searching', () => {
  const sorted = [fonts[0], fonts[2], fonts[1], fonts[3]]; // traditional, latin, japanese, japanese
  assert.deepEqual(fontPickerGroups(fontPickerRows(sorted, '')), [
    { group: 'traditional', beforeIndex: 1 },
    { group: 'latin', beforeIndex: 2 },
    { group: 'japanese', beforeIndex: 3 },
  ], '「預設字型」 (row 0) stays above the first heading');
  assert.deepEqual(fontPickerGroups(fontPickerRows(sorted, 'gothic')), [{ group: 'japanese', beforeIndex: 1 }]);
  assert.deepEqual(fontPickerGroups(fontPickerRows(sorted, 'nothing')), []);
});

test('「預設（…）」 names the font the default actually draws (CJK) text in, per target', () => {
  const catalog = [
    { family: 'Segoe UI', displayName: 'Segoe UI', group: 'latin', faces: [] },
    { family: 'Microsoft JhengHei', displayName: '微軟正黑體', group: 'traditional', faces: [] },
    { family: 'Yu Gothic', displayName: '游ゴシック', group: 'japanese', faces: [] },
    { family: 'Noto Sans CJK TC', displayName: 'Noto Sans CJK TC', group: 'traditional', faces: [] },
  ];
  const name = (input) => defaultFontFamily({ fonts: catalog, ...input })?.displayName ?? null;
  assert.deepEqual(cssStackFamilies(DEFAULT_INTERFACE_FONT_STACK), ['Segoe UI', 'Microsoft JhengHei', 'sans-serif']);
  // Windows: Segoe UI has no CJK glyphs, so Chinese text is drawn by Microsoft JhengHei.
  assert.equal(name({ target: 'interface', platform: 'windows' }), '微軟正黑體');
  assert.equal(name({ target: 'lyrics', platform: 'windows' }), '微軟正黑體');
  assert.equal(name({ target: 'lyrics', platform: 'windows', interfaceFamily: 'yu gothic' }), '游ゴシック', 'the playing page follows the interface font');
  assert.equal(name({ target: 'lyrics', platform: 'windows', interfaceFamily: 'Not Installed' }), '微軟正黑體');
  assert.equal(name({ target: 'taskbar', platform: 'windows' }), '微軟正黑體');
  assert.equal(name({ target: 'floating', platform: 'windows' }), '微軟正黑體');
  // Only Latin fonts of the stack installed: the first of them.
  assert.equal(defaultFontFamily({ target: 'interface', platform: 'windows', fonts: [catalog[0]] })?.family, 'Segoe UI');
  // Android: nothing of the stack is installed, so sans-serif: the system default for the region.
  const androidFonts = [catalog[3]];
  assert.equal(defaultFontFamily({ target: 'interface', platform: 'android', fonts: androidFonts, systemDefault: 'Noto Sans CJK TC' })?.family, 'Noto Sans CJK TC');
  assert.equal(defaultFontFamily({ target: 'floating', platform: 'android', fonts: androidFonts, systemDefault: 'Noto Sans CJK TC' })?.family, 'Noto Sans CJK TC');
  // Unknown: the label stays 「預設字型」.
  assert.equal(defaultFontFamily({ target: 'floating', platform: 'android', fonts: androidFonts, systemDefault: null }), null);
  assert.equal(defaultFontFamily({ target: 'interface', platform: 'windows', fonts: [] }), null);
});

test('the native lyrics default named by the picker is the family win.rs maps CJK text to', () => {
  const win = read('crates/player-lyrics-native/src/win.rs');
  assert.ok(win.includes(`let family = HSTRING::from("${WINDOWS_NATIVE_LYRICS_CJK_FAMILY}");`), 'win.rs CJK fallback mapping');
  const picker = read('src/lib/FontPicker.svelte');
  assert.match(picker, /tr\('預設（\{0\}）', \[defaultFont\.displayName\]\)/);
  const look = read('src/lib/LyricsLookSettings.svelte');
  assert.match(look, /<FontPicker[^>]*target=\{fontTarget\}/);
  const app = read('src/App.svelte');
  assert.match(app, /fontTarget="taskbar"/);
  assert.match(app, /fontTarget="floating"/);
  assert.match(app, /setInterfaceFontForPickers\(interfaceFontFamily\)/);
});

test('the picker reuses the library virtual list (group rows, quick index) and never draws a row in the real family', () => {
  const picker = read('src/lib/FontPicker.svelte');
  assert.match(picker, /import PagedVirtualList from '\.\/PagedVirtualList\.svelte'/);
  assert.match(picker, /<PagedVirtualList[\s\S]*\{groupRows\}[\s\S]*\{quickIndexLabels\}/);
  assert.match(picker, /use:rowFace=\{item\.font\}/);
  assert.doesNotMatch(picker, /ensureAndroidFontFaces\(row/, 'rows no longer register the real font files');
  for (const label of ['繁體中文', '簡體中文', '日文', '英文']) assert.ok(picker.includes(`tr('${label}')`), label);
});

test('the current setting shows its localized name, or the stored one when it is not listed', () => {
  assert.equal(fontDisplayName(fonts, 'microsoft jhenghei'), '微軟正黑體');
  assert.equal(fontDisplayName(fonts, 'Gone Font'), 'Gone Font');
  assert.equal(fontDisplayName([], 'Yu Gothic'), 'Yu Gothic');
  assert.equal(fontDisplayName(fonts, null), null);
});

test('a background refresh replaces the list only when it changed', () => {
  const next = [fonts[1]];
  assert.equal(applyFontListUpdate(fonts, { fonts: next, changed: true }), next);
  assert.equal(applyFontListUpdate(fonts, { fonts: [], changed: false }), fonts);
  assert.equal(applyFontListUpdate(fonts, null), fonts);
});

test('Android loads a family through one @font-face per face file', () => {
  const sources = androidFontFaceSources(fonts[2], (file) => `http://asset.localhost/${encodeURIComponent(file)}`);
  assert.deepEqual(sources, [
    { family: 'MoeFont Segoe UI', source: 'url("http://asset.localhost/C%3A%2FWindows%2FFonts%2Fsegoeui.ttf")', descriptors: { weight: '400', style: 'normal' } },
    { family: 'MoeFont Segoe UI', source: 'url("http://asset.localhost/C%3A%2FWindows%2FFonts%2Fsegoeuii.ttf")', descriptors: { weight: '400', style: 'italic' } },
  ]);
});

test('the pickers are wired into the four places', () => {
  const look = read('src/lib/LyricsLookSettings.svelte');
  const at = (needle) => look.indexOf(needle);
  assert.ok(at('size-heading') < at('<FontPicker') && at('<FontPicker') < at('tr("字體大小")'), '字型 is the first item of 字體與大小');
  assert.match(look, /<FontPicker[^>]*value=\{settings\.fontFamily\}[^>]*onChange=\{\(fontFamily\) => onChange\(\{ fontFamily \}\)\}/);
  const quick = read('src/lib/NowPlayingQuickSettingsControls.svelte');
  assert.ok(quick.includes('fontFamily: lyrics.fontFamily'));
  assert.ok(quick.includes("'fontFamily' in patch"), 'choosing 預設字型 (null) is passed on');
  const app = read('src/App.svelte');
  assert.match(app, /<FontPicker idPrefix="\{scope\}interface-font"[^>]*value=\{interfaceFontFamily\}[^>]*onChange=\{setInterfaceFont\}/);
  assert.match(app, /setProperty\('--app-font-family', fontFamilyCss\(interfaceFontFamily, DEFAULT_INTERFACE_FONT_STACK/);
  assert.match(app, /removeProperty\('--app-font-family'\)/);
  const css = read('src/app.css');
  assert.match(css, /--app-default-font: 'Segoe UI', 'Microsoft JhengHei', sans-serif;/);
  assert.match(css, /--app-font-stack: var\(--app-font-family, var\(--app-default-font\)\);/);
  assert.match(css, /:root \{[^}]*font-family: var\(--app-font-stack\);/s);
  const view = read('src/lib/LyricsView.svelte');
  assert.match(view, /--lyric-font-family:\$\{lyricFontFamily\}/);
  assert.match(view, /\.lyrics-view \{[^}]*font-family: var\(--lyric-font-family, inherit\);/s);
  assert.match(view, /void lyricFontFamily/, 'the ink-height measure follows the font');
});

test('the picker reads the list only when it opens and keeps one shared list', () => {
  const picker = read('src/lib/FontPicker.svelte');
  assert.match(picker, /<script module lang="ts">[\s\S]*const catalog = \$state\(/, 'the list is shared by every mounted picker');
  assert.match(picker, /function toggle\(\) \{\s*open = !open;\s*if \(open\) ensureLoaded\(\);/);
  assert.match(picker, /'fonts-list-updated'/);
  for (const file of ['src/App.svelte', 'src/main.ts']) {
    assert.doesNotMatch(read(file), /'fonts_list'/, `${file} never scans at startup`);
  }
});

test('the new labels are translated', () => {
  for (const language of ['ja', 'en', 'zh-CN']) {
    const catalog = JSON.parse(read(`src/lib/i18n/${language}.json`));
    for (const key of ['字型', '預設字型', '預設（{0}）', '搜尋字型', '讀取字型中…', '主介面字型', '繁體中文', '簡體中文', '日文', '英文']) assert.ok(catalog[key], `${language}: ${key}`);
  }
});
