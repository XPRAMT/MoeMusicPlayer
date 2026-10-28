/**
 * The cover scan of the album detail page, as the page talks to the backend (ADR 0037). The page opens one scan per album
 * entry: it asks the backend for the covers (the quick one for the first cover, the full one to choose the representative
 * covers) and, when it is left, cancels whatever it asked for under every key it used, so the backend stops reading files
 * at once. Each call is made at most once; after `cancel()` nothing more is asked.
 * @param {{ ensure: (args: object) => Promise<any>, cancel: (keys: string[]) => Promise<any> | void }} backend
 */
export function createAlbumDetailCoverScan({ ensure, cancel }) {
  const keys = new Set();
  /** @type {Map<string, Promise<any>>} */
  const calls = new Map();
  let cancelled = false;
  return {
    /**
     * Ask the backend for the covers described by `args` (`key`, `representative`, `forceRebuild`, ...). The same call is
     * not made twice: asking again for it returns the first answer. `null` once the scan was cancelled.
     * @param {{ key: string, representative?: boolean, forceRebuild?: boolean, [name: string]: any }} args
     */
    ensure(args) {
      if (cancelled) return Promise.resolve(null);
      const id = `${args.key}|${args.representative ? 1 : 0}|${args.forceRebuild ? 1 : 0}`;
      const made = calls.get(id);
      if (made) return made;
      keys.add(args.key);
      const call = ensure(args);
      calls.set(id, call);
      return call;
    },
    /** The page was left or closed: the backend ends the scans asked for under the keys used so far. */
    cancel() {
      if (cancelled) return;
      cancelled = true;
      if (keys.size === 0) return;
      try {
        const result = cancel([...keys]);
        if (result && typeof result.catch === 'function') result.catch(() => undefined);
      } catch { /* the backend is gone: nothing is left to cancel */ }
    },
    get cancelled() { return cancelled; },
  };
}
