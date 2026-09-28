import assert from 'node:assert/strict';
import test from 'node:test';
import { createVolumeCommandQueue } from '../src/lib/volume-command-queue.ts';

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

async function waitFor(predicate, timeoutMs = 1_000) {
  const startedAt = Date.now();
  while (!predicate()) {
    if (Date.now() - startedAt >= timeoutMs) throw new Error('condition did not become true');
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
}

test('volume drag coalesces updates, serializes commands, and flushes the latest endpoint', async () => {
  const calls = [];
  const completions = [];
  let active = 0;
  let maximumActive = 0;
  let idleCount = 0;
  const queue = createVolumeCommandQueue({
    delayMs: 15,
    send: (volume) => {
      calls.push(volume);
      active += 1;
      maximumActive = Math.max(maximumActive, active);
      const completion = deferred();
      completions.push(() => {
        active -= 1;
        completion.resolve();
      });
      return completion.promise;
    },
    onIdle: () => { idleCount += 1; },
  });

  queue.enqueue(0.1);
  queue.enqueue(0.25);
  queue.enqueue(0.4);
  await waitFor(() => calls.length === 1);
  assert.equal(calls[0], 0.4, 'the initial burst must send only its latest value');

  queue.enqueue(0.5);
  queue.enqueue(0.7);
  queue.flush();
  assert.equal(calls.length, 1, 'a second command must wait for the in-flight command');
  completions[0]();
  await waitFor(() => calls.length === 2);
  assert.equal(calls[1], 0.7, 'flush must send the latest pending value immediately after the ACK');

  queue.enqueue(0.8);
  queue.enqueue(1.4);
  queue.flush();
  completions[1]();
  await waitFor(() => calls.length === 3);
  assert.equal(calls[2], 1, 'volume values must clamp to the supported range');
  completions[2]();
  await waitFor(() => idleCount === 1);

  assert.equal(maximumActive, 1, 'volume commands must never overlap');
  assert.equal(calls.length, 3, 'drag input updates must collapse into a bounded command count');
  queue.dispose();
});

test('disposing a volume queue cancels a pending debounce', async () => {
  const calls = [];
  const queue = createVolumeCommandQueue({
    delayMs: 25,
    send: async (volume) => { calls.push(volume); },
  });

  queue.enqueue(0.7);
  queue.dispose();
  await new Promise((resolve) => setTimeout(resolve, 40));
  assert.deepEqual(calls, []);
});
