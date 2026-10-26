// The font picker (ADR 0042) outside Tauri: `fonts_list` and the event plugin are answered here. The list is
// `globalThis.__FONT_PICKER_FIXTURE__` when a test sets one (e.g. a real list with subset files, served by Vite's
// `/@fs/`), else a small synthetic list in the four groups.
import { mount } from 'svelte';
import FontPickerHarness from './FontPickerHarness.svelte';
import '../src/app.css';

type Fixture = Array<{ family: string; displayName: string; group: string; faces: unknown[]; subset?: string }>;
const globals = globalThis as unknown as Record<string, unknown>;
const synthetic: Fixture = [
  ...['微軟正黑體', '標楷體', '新細明體'].map((name) => ({ family: name, displayName: name, group: 'traditional', faces: [] })),
  ...['微软雅黑', '宋体'].map((name) => ({ family: name, displayName: name, group: 'simplified', faces: [] })),
  ...['游ゴシック', 'メイリオ'].map((name) => ({ family: name, displayName: name, group: 'japanese', faces: [] })),
  ...Array.from({ length: 60 }, (_, index) => ({ family: `Latin ${index + 1}`, displayName: `Latin ${index + 1}`, group: 'latin', faces: [] })),
];
const fonts = (globals.__FONT_PICKER_FIXTURE__ as Fixture | undefined) ?? synthetic;
globals.isTauri = true;
let nextCallback = 0;
globals.__TAURI_INTERNALS__ = {
  transformCallback: (callback: unknown) => {
    nextCallback += 1;
    globals[`_${nextCallback}`] = callback;
    return nextCallback;
  },
  convertFileSrc: (path: string) => `/@fs/${path.split(String.fromCharCode(92)).join('/')}`,
  invoke: async (command: string) => {
    if (command === 'fonts_list') return { fonts, refreshing: false, systemDefault: null };
    if (command === 'plugin:event|listen') return 1;
    return null;
  },
};

mount(FontPickerHarness, { target: document.querySelector('#app')! });
