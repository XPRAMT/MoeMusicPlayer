import test from 'node:test';
import assert from 'node:assert/strict';
import { createDecodedImageObjectUrl } from '../src/lib/decoded-image-object-url.js';

function makeDeferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test('keeps the displayed image until the replacement has decoded', async () => {
  const decode = makeDeferred();
  const revoked = [];
  const displayed = { url: 'blob:previous' };
  const nextPromise = createDecodedImageObjectUrl(new ArrayBuffer(1), 'image/png', {
    createObjectURL: () => 'blob:next',
    revokeObjectURL: (url) => revoked.push(url),
    createImage: () => ({
      naturalWidth: 512,
      naturalHeight: 512,
      set src(value) { this.source = value; },
      decode: () => decode.promise,
    }),
  }).then((url) => {
    displayed.url = url;
    revoked.push('blob:previous');
  });

  await Promise.resolve();
  assert.equal(displayed.url, 'blob:previous');
  assert.deepEqual(revoked, []);
  decode.resolve();
  await nextPromise;
  assert.equal(displayed.url, 'blob:next');
  assert.deepEqual(revoked, ['blob:previous']);
});

test('revokes an undecodable replacement and leaves the displayed image intact', async () => {
  const revoked = [];
  const displayed = { url: 'blob:previous' };
  await assert.rejects(createDecodedImageObjectUrl(new ArrayBuffer(1), 'image/png', {
    createObjectURL: () => 'blob:broken',
    revokeObjectURL: (url) => revoked.push(url),
    createImage: () => ({
      naturalWidth: 0,
      naturalHeight: 0,
      set src(value) { this.source = value; },
      decode: async () => { throw new Error('decode failed'); },
    }),
  }));
  assert.equal(displayed.url, 'blob:previous');
  assert.deepEqual(revoked, ['blob:broken']);
});
