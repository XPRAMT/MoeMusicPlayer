import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createActiveTrackArtworkController,
  detectArtworkMimeType,
} from '../src/lib/active-track-artwork.ts';

function bytes(...values) {
  return Uint8Array.from(values).buffer;
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function nextTurn() {
  await new Promise((resolve) => setImmediate(resolve));
}

function controllerWith(overrides = {}) {
  const created = [];
  const revoked = [];
  const states = [];
  let sequence = 0;
  const controller = createActiveTrackArtworkController({
    fetchBytes: async () => bytes(0xff, 0xd8, 0xff, 0x00),
    createObjectUrl: (imageBytes, mimeType) => {
      const objectUrl = `blob:artwork-${++sequence}`;
      created.push({ objectUrl, imageBytes, mimeType });
      return objectUrl;
    },
    revokeObjectUrl: (objectUrl) => revoked.push(objectUrl),
    onChange: (state) => states.push(state),
    ...overrides,
  });
  return { controller, created, revoked, states };
}

test('one active track requests once and exposes the original JPEG bytes as a Blob URL', async () => {
  const imageBytes = bytes(0xff, 0xd8, 0xff, 0x01, 0x02);
  const requests = [];
  const fixture = controllerWith({
    fetchBytes: async (trackId) => {
      requests.push(trackId);
      return imageBytes;
    },
  });

  fixture.controller.setTrack('track-1');
  fixture.controller.setTrack('track-1');
  await nextTurn();

  assert.deepEqual(requests, ['track-1'], 'playback polling must not fetch the same track repeatedly');
  assert.equal(fixture.controller.snapshot().status, 'ready');
  assert.equal(fixture.created[0].imageBytes, imageBytes, 'the original binary buffer is passed through unchanged');
  assert.equal(fixture.created[0].mimeType, 'image/jpeg');
  assert.deepEqual(fixture.revoked, []);
});

test('track changes keep the current image until the latest replacement is decoded and applied', async () => {
  const first = deferred();
  const second = deferred();
  const third = deferred();
  const requests = [];
  const fixture = controllerWith({
    fetchBytes: (trackId) => {
      requests.push(trackId);
      return trackId === 'track-a' ? first.promise : trackId === 'track-b' ? second.promise : third.promise;
    },
  });

  fixture.controller.setTrack('track-a');
  fixture.controller.setTrack('track-b');
  assert.deepEqual(requests, ['track-a'], 'there is at most one native artwork request at a time');
  first.resolve(bytes(0xff, 0xd8, 0xff, 0xa));
  await nextTurn();
  assert.deepEqual(requests, ['track-a', 'track-b']);
  assert.equal(fixture.created.length, 0, 'the stale track response is discarded');

  second.resolve(bytes(0xff, 0xd8, 0xff, 0xb));
  await nextTurn();
  assert.equal(fixture.controller.snapshot().trackId, 'track-b');
  assert.equal(fixture.controller.snapshot().status, 'ready');
  assert.equal(fixture.created.length, 1);

  fixture.controller.setTrack('track-c');
  assert.equal(fixture.controller.snapshot().status, 'loading');
  assert.equal(fixture.controller.snapshot().objectUrl, 'blob:artwork-1');
  assert.deepEqual(fixture.revoked, [], 'the displayed image remains alive while its replacement is loading');
  fixture.controller.imageFailed('blob:artwork-1');
  assert.equal(fixture.controller.snapshot().trackId, 'track-c', 'an old image error does not cancel the pending track request');

  third.resolve(bytes(0xff, 0xd8, 0xff, 0xc));
  await nextTurn();
  await nextTurn();
  assert.equal(fixture.controller.snapshot().trackId, 'track-c');
  assert.equal(fixture.controller.snapshot().status, 'ready');
  assert.equal(fixture.controller.snapshot().objectUrl, 'blob:artwork-2');
  assert.deepEqual(fixture.revoked, ['blob:artwork-1'], 'the old image is released after the decoded replacement is published');
});

test('artwork replacement waits for image decoding and DOM application before releasing the old URL', async () => {
  const decode = deferred();
  const applied = deferred();
  let sequence = 0;
  const revoked = [];
  const controller = createActiveTrackArtworkController({
    fetchBytes: async () => bytes(0xff, 0xd8, 0xff, 0x01),
    createObjectUrl: async () => {
      const url = `blob:decoded-${++sequence}`;
      if (sequence === 2) await decode.promise;
      return url;
    },
    revokeObjectUrl: (url) => revoked.push(url),
    onChange: () => {},
    afterChange: () => applied.promise,
  });

  controller.setTrack('track-a');
  await nextTurn();
  assert.equal(controller.snapshot().objectUrl, 'blob:decoded-1');

  controller.setTrack('track-b');
  assert.equal(controller.snapshot().status, 'loading');
  assert.equal(controller.snapshot().objectUrl, 'blob:decoded-1');
  await nextTurn();
  assert.deepEqual(revoked, [], 'the old URL remains valid during the decode');

  decode.resolve();
  await nextTurn();
  await nextTurn();
  assert.equal(controller.snapshot().objectUrl, 'blob:decoded-2');
  assert.deepEqual(revoked, [], 'the old URL remains valid until the subscriber applies the new state');

  applied.resolve();
  await nextTurn();
  assert.deepEqual(revoked, ['blob:decoded-1']);
  controller.dispose();
});

test('stopping or disposing invalidates in-flight results and releases the active URL', async () => {
  const pending = deferred();
  const fixture = controllerWith({ fetchBytes: () => pending.promise });
  fixture.controller.setTrack('track-a');
  fixture.controller.setTrack(null);
  pending.resolve(bytes(0xff, 0xd8, 0xff));
  await nextTurn();
  assert.equal(fixture.controller.snapshot().status, 'empty');
  assert.equal(fixture.created.length, 0);

  const loaded = controllerWith();
  loaded.controller.setTrack('track-b');
  await nextTurn();
  loaded.controller.dispose();
  assert.deepEqual(loaded.revoked, ['blob:artwork-1']);
  assert.equal(loaded.controller.snapshot().objectUrl, null);
  loaded.controller.setTrack('track-c');
  assert.equal(loaded.created.length, 1, 'a disposed controller never starts more reads');
});

test('missing, unsupported, and oversized artwork leave the placeholder available', async () => {
  const missing = controllerWith({ fetchBytes: async () => new ArrayBuffer(0) });
  missing.controller.setTrack('missing');
  await nextTurn();
  assert.equal(missing.controller.snapshot().status, 'missing');

  const unsupported = controllerWith({ fetchBytes: async () => bytes(0x00, 0x01, 0x02) });
  unsupported.controller.setTrack('unsupported');
  await nextTurn();
  assert.equal(unsupported.controller.snapshot().status, 'error');

  const oversized = controllerWith({
    maxBytes: 3,
    fetchBytes: async () => bytes(0xff, 0xd8, 0xff, 0x00),
  });
  oversized.controller.setTrack('large');
  await nextTurn();
  assert.equal(oversized.controller.snapshot().status, 'too-large');
  assert.equal(oversized.created.length, 0);
});

test('the native oversized-result code is shown as too-large instead of missing or generic error', async () => {
  for (const error of ['ARTWORK_TOO_LARGE', new Error('ARTWORK_TOO_LARGE')]) {
    const fixture = controllerWith({ fetchBytes: async () => { throw error; } });
    fixture.controller.setTrack('oversized-at-source');
    await nextTurn();
    assert.equal(fixture.controller.snapshot().status, 'too-large');
    assert.equal(fixture.created.length, 0);
  }
});

test('a stale image error cannot revoke or clear the newer track', async () => {
  const fixture = controllerWith();
  fixture.controller.setTrack('track-a');
  await nextTurn();
  const oldUrl = fixture.controller.snapshot().objectUrl;
  fixture.controller.setTrack('track-b');
  await nextTurn();
  await nextTurn();
  const currentUrl = fixture.controller.snapshot().objectUrl;

  fixture.controller.imageFailed(oldUrl);
  assert.equal(fixture.controller.snapshot().objectUrl, currentUrl);
  assert.deepEqual(fixture.revoked, [oldUrl]);
});

test('binary signatures identify supported original raster formats without decoding them', () => {
  assert.equal(detectArtworkMimeType(bytes(0xff, 0xd8, 0xff)), 'image/jpeg');
  assert.equal(detectArtworkMimeType(bytes(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a)), 'image/png');
  assert.equal(detectArtworkMimeType(bytes(0x52, 0x49, 0x46, 0x46, 0, 0, 0, 0, 0x57, 0x45, 0x42, 0x50)), 'image/webp');
  assert.equal(detectArtworkMimeType(bytes(0x00, 0x01, 0x02)), null);
});

test('reload fetches the same track again and swaps in the new image', async () => {
  let version = 0;
  const { controller, states, revoked } = controllerWith({
    fetchBytes: async () => bytes(0xff, 0xd8, 0xff, ++version),
  });
  controller.setTrack('t1');
  await nextTurn();
  await nextTurn();
  const first = controller.snapshot().objectUrl;
  assert.equal(controller.snapshot().status, 'ready');

  controller.reload();
  await nextTurn();
  await nextTurn();
  const second = controller.snapshot().objectUrl;
  assert.equal(version, 2);
  assert.notEqual(second, first);
  assert.equal(controller.snapshot().status, 'ready');
  assert.ok(revoked.includes(first));
  assert.ok(states.length > 1);

  controller.setTrack(null);
  controller.reload();
  await nextTurn();
  assert.equal(version, 2, 'no track, nothing fetched');
});
