export interface VolumeCommandQueueOptions {
  delayMs?: number;
  send: (volume: number) => Promise<void>;
  onError?: (error: unknown) => void;
  onIdle?: () => void;
}

export interface VolumeCommandQueue {
  enqueue(volume: number): void;
  flush(): void;
  dispose(): void;
}

/**
 * Keep at most one volume command in flight and one latest pending value.
 * A caller may debounce live input with enqueue() and force the final value
 * through with flush() on pointer release or the range change event.
 */
export function createVolumeCommandQueue({
  delayMs = 64,
  send,
  onError = () => {},
  onIdle = () => {},
}: VolumeCommandQueueOptions): VolumeCommandQueue {
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pendingVolume: number | null = null;
  let inFlight = false;
  let flushAfterCurrent = false;
  let disposed = false;

  function clearTimer(): void {
    if (timer === null) return;
    clearTimeout(timer);
    timer = null;
  }

  function schedule(delay: number): void {
    clearTimer();
    timer = setTimeout(() => {
      timer = null;
      void dispatchLatest();
    }, delay);
  }

  async function dispatchLatest(): Promise<void> {
    if (disposed || inFlight || pendingVolume === null) return;
    clearTimer();
    const volume = pendingVolume;
    pendingVolume = null;
    inFlight = true;
    try {
      await send(volume);
    } catch (error) {
      if (!disposed) onError(error);
    } finally {
      inFlight = false;
      if (disposed) return;
      if (pendingVolume !== null) {
        if (flushAfterCurrent) {
          flushAfterCurrent = false;
          void dispatchLatest();
        } else {
          schedule(delayMs);
        }
      } else {
        flushAfterCurrent = false;
        onIdle();
      }
    }
  }

  return {
    enqueue(volume) {
      if (disposed || !Number.isFinite(volume)) return;
      pendingVolume = Math.max(0, Math.min(1, volume));
      if (!inFlight) schedule(delayMs);
    },
    flush() {
      if (disposed) return;
      clearTimer();
      if (pendingVolume === null) {
        if (!inFlight) onIdle();
        return;
      }
      if (inFlight) {
        flushAfterCurrent = true;
        return;
      }
      void dispatchLatest();
    },
    dispose() {
      disposed = true;
      clearTimer();
      pendingVolume = null;
    },
  };
}
