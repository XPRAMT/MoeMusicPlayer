import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const rustSource = readFileSync(path.join(root, 'src-tauri/src/lib.rs'), 'utf8');
const tauriConfig = JSON.parse(readFileSync(path.join(root, 'src-tauri/tauri.conf.json'), 'utf8'));

function commandBlock(name) {
  const start = rustSource.indexOf(`async fn ${name}(`);
  assert.notEqual(start, -1, `missing async command ${name}`);
  const nextCommand = rustSource.indexOf('\n#[tauri::command]', start + 1);
  return rustSource.slice(start, nextCommand === -1 ? undefined : nextCommand);
}

function assertOwnedDialog(name, expectedCount) {
  const block = commandBlock(name);
  assert.match(block, /window\s*:\s*WebviewWindow/, `${name} must receive the invoking webview window`);
  assert.equal(
    [...block.matchAll(/\.set_parent\(&window\)/g)].length,
    expectedCount,
    `${name} must make every native dialog owned by the invoking window`,
  );
}

assertOwnedDialog('library_pick_windows_folder', 1);
assertOwnedDialog('playlist_import_m3u', 1);
assertOwnedDialog('playlist_export_m3u', 2);

const mainWindow = tauriConfig.app.windows.find(window => window.label === 'main');
assert.ok(mainWindow, 'missing main Tauri window');
assert.equal(mainWindow.alwaysOnTop, false, 'the main window must never be configured always-on-top');

console.log('pass: native folder and playlist dialogs have a parent; main window is not topmost');
