import test from 'node:test';
import assert from 'node:assert/strict';
import {
  ensureAndroidLocalNetworkAccess,
  shouldShowAndroidLocalNetworkAuthorizationPanel,
} from '../src/lib/android-local-network-permission.js';

function withWindow(windowValue, callback) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: windowValue });
  return Promise.resolve().then(callback).finally(() => {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else delete globalThis.window;
  });
}

test('non-Android runtime does not request local-network permission', async () => {
  await withWindow(undefined, async () => {
    assert.equal(await ensureAndroidLocalNetworkAccess(), true);
    assert.equal(shouldShowAndroidLocalNetworkAuthorizationPanel(), false);
  });
});

test('authorization panel is shown only when Android reports local-network access missing', async () => {
  const windowValue = new EventTarget();
  let hasPermission = true;
  windowValue.MoeAndroidInsets = {
    hasLocalNetworkAccess: () => hasPermission,
    requestLocalNetworkPermission: () => assert.fail('the panel check must not request permission'),
  };
  await withWindow(windowValue, async () => {
    assert.equal(shouldShowAndroidLocalNetworkAuthorizationPanel(), false);
    hasPermission = false;
    assert.equal(shouldShowAndroidLocalNetworkAuthorizationPanel(), true);
  });
});

test('granted Android permission avoids a system request', async () => {
  const windowValue = new EventTarget();
  windowValue.MoeAndroidInsets = {
    hasLocalNetworkAccess: () => true,
    requestLocalNetworkPermission: () => assert.fail('permission is already granted'),
  };
  await withWindow(windowValue, async () => {
    assert.equal(await ensureAndroidLocalNetworkAccess(), true);
  });
});

test('permission result is returned and concurrent callers share one system request', async () => {
  const windowValue = new EventTarget();
  let hasPermission = false;
  let requestCount = 0;
  let requestId;
  windowValue.MoeAndroidInsets = {
    hasLocalNetworkAccess: () => hasPermission,
    requestLocalNetworkPermission: (id) => { requestCount += 1; requestId = id; },
  };

  await withWindow(windowValue, async () => {
    const first = ensureAndroidLocalNetworkAccess();
    const second = ensureAndroidLocalNetworkAccess();
    assert.equal(requestCount, 1);
    hasPermission = true;
    windowValue.dispatchEvent(new CustomEvent('moe:android-local-network-permission-result', {
      detail: { requestId, granted: true },
    }));
    assert.deepEqual(await Promise.all([first, second]), [true, true]);
  });
});

test('denial resolves false', async () => {
  const windowValue = new EventTarget();
  let requestId;
  windowValue.MoeAndroidInsets = {
    hasLocalNetworkAccess: () => false,
    requestLocalNetworkPermission: (id) => { requestId = id; },
  };

  await withWindow(windowValue, async () => {
    const result = ensureAndroidLocalNetworkAccess();
    windowValue.dispatchEvent(new CustomEvent('moe:android-local-network-permission-result', {
      detail: { requestId, granted: false },
    }));
    assert.equal(await result, false);
  });
});
