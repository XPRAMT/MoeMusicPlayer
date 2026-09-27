export const MAX_ACTIVE_ARTWORK_BYTES = 32 * 1024 * 1024;

export type ActiveArtworkStatus = 'empty' | 'loading' | 'ready' | 'missing' | 'error';

export interface ActiveArtworkState {
  trackId: string | null;
  status: ActiveArtworkStatus;
  objectUrl: string | null;
}

export interface ActiveTrackArtworkController {
  setTrack(trackId: string | null): void;
  imageFailed(objectUrl: string): void;
  snapshot(): ActiveArtworkState;
  dispose(): void;
}

interface ActiveTrackArtworkOptions {
  fetchBytes(trackId: string): Promise<ArrayBuffer>;
  createObjectUrl(bytes: ArrayBuffer, mimeType: string): string;
  revokeObjectUrl(objectUrl: string): void;
  onChange(state: ActiveArtworkState): void;
  maxBytes?: number;
}

export function detectArtworkMimeType(bytes: ArrayBuffer): string | null {
  const data = new Uint8Array(bytes, 0, Math.min(bytes.byteLength, 16));
  if (data.length >= 3 && data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff) {
    return 'image/jpeg';
  }
  if (data.length >= 8
    && data[0] === 0x89
    && data[1] === 0x50
    && data[2] === 0x4e
    && data[3] === 0x47
    && data[4] === 0x0d
    && data[5] === 0x0a
    && data[6] === 0x1a
    && data[7] === 0x0a) {
    return 'image/png';
  }
  if (data.length >= 6) {
    const signature = String.fromCharCode(...data.subarray(0, 6));
    if (signature === 'GIF87a' || signature === 'GIF89a') return 'image/gif';
  }
  if (data.length >= 12
    && String.fromCharCode(...data.subarray(0, 4)) === 'RIFF'
    && String.fromCharCode(...data.subarray(8, 12)) === 'WEBP') {
    return 'image/webp';
  }
  if (data.length >= 2 && data[0] === 0x42 && data[1] === 0x4d) return 'image/bmp';
  if (data.length >= 12
    && String.fromCharCode(...data.subarray(4, 8)) === 'ftyp') {
    const brand = String.fromCharCode(...data.subarray(8, 12));
    if (brand === 'avif' || brand === 'avis') return 'image/avif';
  }
  return null;
}

export function createActiveTrackArtworkController(
  options: ActiveTrackArtworkOptions,
): ActiveTrackArtworkController {
  const maxBytes = options.maxBytes ?? MAX_ACTIVE_ARTWORK_BYTES;
  let currentTrackId: string | null = null;
  let generation = 0;
  let requestInFlight = false;
  let disposed = false;
  let state: ActiveArtworkState = { trackId: null, status: 'empty', objectUrl: null };

  function publish(next: ActiveArtworkState): void {
    state = next;
    if (!disposed) options.onChange({ ...next });
  }

  function releaseUrl(): void {
    const objectUrl = state.objectUrl;
    if (!objectUrl) return;
    try {
      options.revokeObjectUrl(objectUrl);
    } catch {
      // A failed cleanup must not prevent the active track from changing.
    }
  }

  function startRequest(): void {
    if (disposed || requestInFlight || !currentTrackId) return;
    const requestTrackId = currentTrackId;
    const requestGeneration = generation;
    requestInFlight = true;

    void (async () => {
      try {
        const bytes = await options.fetchBytes(requestTrackId);
        if (disposed || requestGeneration !== generation || requestTrackId !== currentTrackId) return;
        if (bytes.byteLength === 0) {
          publish({ trackId: requestTrackId, status: 'missing', objectUrl: null });
          return;
        }
        if (bytes.byteLength > maxBytes) {
          publish({ trackId: requestTrackId, status: 'error', objectUrl: null });
          return;
        }
        const mimeType = detectArtworkMimeType(bytes);
        if (!mimeType) {
          publish({ trackId: requestTrackId, status: 'error', objectUrl: null });
          return;
        }
        const objectUrl = options.createObjectUrl(bytes, mimeType);
        publish({ trackId: requestTrackId, status: 'ready', objectUrl });
      } catch {
        if (!disposed && requestGeneration === generation && requestTrackId === currentTrackId) {
          publish({ trackId: requestTrackId, status: 'error', objectUrl: null });
        }
      } finally {
        requestInFlight = false;
        if (!disposed && currentTrackId && requestGeneration !== generation) startRequest();
      }
    })();
  }

  return {
    setTrack(trackId) {
      const nextTrackId = trackId?.trim() || null;
      if (disposed || nextTrackId === currentTrackId) return;

      generation += 1;
      currentTrackId = nextTrackId;
      releaseUrl();
      if (!nextTrackId) {
        publish({ trackId: null, status: 'empty', objectUrl: null });
        return;
      }

      publish({ trackId: nextTrackId, status: 'loading', objectUrl: null });
      startRequest();
    },

    imageFailed(objectUrl) {
      if (disposed || state.objectUrl !== objectUrl) return;
      generation += 1;
      releaseUrl();
      publish({ trackId: currentTrackId, status: 'error', objectUrl: null });
    },

    snapshot() {
      return { ...state };
    },

    dispose() {
      if (disposed) return;
      generation += 1;
      currentTrackId = null;
      releaseUrl();
      disposed = true;
      state = { trackId: null, status: 'empty', objectUrl: null };
    },
  };
}
