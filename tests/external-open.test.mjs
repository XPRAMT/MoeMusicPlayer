import assert from 'node:assert/strict';
import test from 'node:test';

import { createExternalOpenTaker } from '../src/lib/external-open.js';

test('a request is taken once per call', async () => {
  let calls = 0;
  const take = createExternalOpenTaker(async () => {
    calls += 1;
    return false;
  }, () => assert.fail('no error expected'));
  await take();
  await take();
  assert.equal(calls, 2);
});

test('a call during a take runs again afterwards, however many arrive', async () => {
  let calls = 0;
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const take = createExternalOpenTaker(async () => {
    calls += 1;
    if (calls === 1) await gate;
    return true;
  }, () => assert.fail('no error expected'));
  const first = take();
  await take();
  await take();
  assert.equal(calls, 1);
  release();
  await first;
  assert.equal(calls, 2);
});

test('an error is reported and the next call still works', async () => {
  const errors = [];
  let fail = true;
  const take = createExternalOpenTaker(async () => {
    if (fail) throw new Error('no file');
    return true;
  }, (error) => errors.push(error.message));
  await take();
  fail = false;
  await take();
  assert.deepEqual(errors, ['no file']);
});
