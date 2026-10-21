let requestSequence = 0;
/** @type {Promise<boolean> | null} */
let pendingRequest = null;

/** @typedef {{ hasLocalNetworkAccess: () => boolean, requestLocalNetworkPermission: (requestId: string) => void }} AndroidInsetsBridge */

/** Ask Android 17 for local-network access only when a remote source is about to connect. */
export function ensureAndroidLocalNetworkAccess() {
  /** @type {Window & typeof globalThis & { MoeAndroidInsets?: AndroidInsetsBridge }} */
  const appWindow = globalThis.window;
  const bridge = appWindow?.MoeAndroidInsets;
  if (!bridge || typeof bridge.hasLocalNetworkAccess !== 'function' || typeof bridge.requestLocalNetworkPermission !== 'function') {
    return Promise.resolve(true);
  }

  try {
    if (bridge.hasLocalNetworkAccess()) return Promise.resolve(true);
  } catch {
    return Promise.resolve(false);
  }
  if (pendingRequest) return pendingRequest;

  const requestId = `local-network-${Date.now()}-${++requestSequence}`;
  pendingRequest = new Promise((resolve) => {
    /** @param {boolean} granted */
    const finish = (granted) => {
      globalThis.window.removeEventListener('moe:android-local-network-permission-result', onResult);
      pendingRequest = null;
      resolve(Boolean(granted));
    };
    /** @param {Event} event */
    const onResult = (event) => {
      const detail = /** @type {CustomEvent<{ requestId: string, granted: boolean }>} */ (event).detail;
      if (detail?.requestId !== requestId) return;
      finish(detail.granted);
    };

    globalThis.window.addEventListener('moe:android-local-network-permission-result', onResult);
    try {
      bridge.requestLocalNetworkPermission(requestId);
    } catch {
      finish(false);
    }
  });

  return pendingRequest;
}
