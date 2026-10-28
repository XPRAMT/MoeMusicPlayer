import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createAlbumDetailCoverScan } from '../src/lib/album-detail-cover-scan.js';

function backend() {
  const log = { ensured: [], cancelled: [] };
  return {
    log,
    ensure: async (args) => { log.ensured.push(args); return { trackIds: [] }; },
    cancel: async (keys) => { log.cancelled.push(keys); },
  };
}

test('leaving the page cancels under every key the scan used, the quick key and the full key', async () => {
  const { log, ensure, cancel } = backend();
  const scan = createAlbumDetailCoverScan({ ensure, cancel });
  await scan.ensure({ key: 'quick', albumNames: ['a'], representative: false, forceRebuild: false });
  await scan.ensure({ key: 'full', albumNames: ['a'], representative: true, forceRebuild: false });
  await scan.ensure({ key: 'full', albumNames: ['a'], representative: true, forceRebuild: true });
  scan.cancel();
  assert.deepEqual(log.cancelled, [['quick', 'full']]);
  assert.equal(scan.cancelled, true);
});

test('the same call is not fired twice for one album entry', async () => {
  const { log, ensure, cancel } = backend();
  const scan = createAlbumDetailCoverScan({ ensure, cancel });
  const args = { key: 'full', albumNames: ['a'], representative: true, forceRebuild: false };
  const first = scan.ensure(args);
  const second = scan.ensure({ ...args });
  assert.equal(first, second, 'the second ask gets the first answer');
  await first;
  await scan.ensure(args);
  assert.equal(log.ensured.length, 1);
  // The rebuild is a different call and is allowed once.
  await scan.ensure({ ...args, forceRebuild: true });
  await scan.ensure({ ...args, forceRebuild: true });
  assert.equal(log.ensured.length, 2);
});

test('after the page was left nothing more is asked of the backend, and cancelling twice cancels once', async () => {
  const { log, ensure, cancel } = backend();
  const scan = createAlbumDetailCoverScan({ ensure, cancel });
  await scan.ensure({ key: 'quick', representative: false, forceRebuild: false });
  scan.cancel();
  scan.cancel();
  assert.equal(await scan.ensure({ key: 'full', representative: true, forceRebuild: false }), null);
  assert.equal(log.ensured.length, 1);
  assert.equal(log.cancelled.length, 1);
});

test('leaving before anything was asked has nothing to cancel, and a failing cancel is harmless', async () => {
  const { log, ensure, cancel } = backend();
  const early = createAlbumDetailCoverScan({ ensure, cancel });
  early.cancel();
  assert.deepEqual(log.cancelled, []);
  const failing = createAlbumDetailCoverScan({
    ensure,
    cancel: async () => { throw new Error('gone'); },
  });
  await failing.ensure({ key: 'k', representative: true, forceRebuild: false });
  assert.doesNotThrow(() => failing.cancel());
});

test('the detail page wires the scan: one scan per opened album, started once, cancelled when left, through the detail commands', () => {
  const page = readFileSync(new URL('../src/lib/AlbumDetail.svelte', import.meta.url), 'utf8');
  assert.match(page, /createAlbumDetailCoverScan\(/);
  assert.match(page, /'album_detail_cover_ensure'/);
  assert.match(page, /'album_detail_cover_cancel'/);
  // The card command is the album index cards' and is not used by the detail page.
  assert.doesNotMatch(page, /'album_artwork_selection_ensure'/);
  // Started when the album is opened (mounted), never from an effect that follows reactive state.
  assert.match(page, /onMount\([\s\S]*?startCoverScan\(\)/);
  assert.doesNotMatch(page, /\$effect\(\(\) => \{[^}]*scanCovers/);
  // Left = destroyed: the scan is cancelled there.
  assert.match(page, /onDestroy\([\s\S]*?coverScan\.cancel\(\)/);
});
