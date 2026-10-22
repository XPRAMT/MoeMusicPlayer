import assert from 'node:assert/strict';
import test from 'node:test';
import { originalForPastedCover, pixelFingerprint, rememberCopiedCover } from '../src/lib/cover-clipboard.js';

test('paste uses the copied original bytes only while the clipboard holds the same pixels', async () => {
  const pixels = new Uint8Array([1, 2, 3, 255, 4, 5, 6, 255]);
  const fingerprint = await pixelFingerprint(2, 1, pixels);
  const original = new Blob([new Uint8Array([0xff, 0xd8, 0xff])], { type: 'image/jpeg' });
  rememberCopiedCover(fingerprint, original);

  // The clipboard round trip re-encodes to PNG but the pixels match: the original JPG comes back.
  assert.equal(originalForPastedCover(await pixelFingerprint(2, 1, pixels.slice())), original);
  // Another image (or the same pixels at another size) is pasted as read.
  assert.equal(originalForPastedCover(await pixelFingerprint(2, 1, new Uint8Array([9, 2, 3, 255, 4, 5, 6, 255]))), null);
  assert.equal(originalForPastedCover(await pixelFingerprint(1, 2, pixels)), null);
  assert.equal(originalForPastedCover(null), null);
});

test('the dialog wires copy and paste through the fingerprint', async () => {
  const { readFile } = await import('node:fs/promises');
  const source = await readFile(new URL('../src/lib/TrackDetailsDialog.svelte', import.meta.url), 'utf8');
  assert.match(source, /rememberCopiedCover\(/);
  assert.match(source, /originalForPastedCover\(/);
});
