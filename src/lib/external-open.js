/**
 * Android 「開啟方式」 (ADR 0054): another app opened an audio file with this one. The plugin holds the request until
 * the page takes it; `open` is the command that takes it and starts the file (it resolves to `false` when none waits).
 * The page asks at start, when a running app is sent a new request, and when the app comes forward again. Calls that
 * overlap run one after the other, so a request that arrives while one is being started is not lost.
 * @param {() => Promise<boolean>} open
 * @param {(error: unknown) => void} onError
 * @returns {() => Promise<void>}
 */
export function createExternalOpenTaker(open, onError) {
  let running = false;
  let again = false;
  return async function take() {
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      do {
        again = false;
        try {
          await open();
        } catch (error) {
          onError(error);
        }
      } while (again);
    } finally {
      running = false;
    }
  };
}
