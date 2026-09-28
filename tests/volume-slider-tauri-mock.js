(() => {
  const ready = { state: 'ready', detail: null };
  const unavailable = { state: 'unavailable', detail: 'Not included in the volume interaction test.' };
  const track = {
    id: 'volume-slider-test-track',
    title: 'Volume Slider Test',
    artist: 'Harness',
    album: 'Harness',
    albumArtist: 'Harness',
    trackNumber: 1,
    discNumber: 1,
    durationMs: 60_000,
    codec: 'wav',
    bitrateBps: null,
    sampleRateHz: 48_000,
  };
  let currentSnapshot = {
    currentTrack: track,
    state: 'playing',
    isPlaying: true,
    positionMs: 12_000,
    durationMs: 60_000,
    volume: 0.5,
    lastError: null,
    repeatMode: 'off',
    shuffle: false,
    canNext: false,
    canPrevious: false,
  };
  let nextCallbackId = 0;
  const callbacks = new Map();
  let activeVolumeCommands = 0;
  const volumeHarness = {
    volumeRequests: [],
    inputCount: 0,
    maxConcurrentVolumeCommands: 0,
    holdVolumeAcks: false,
    pendingVolumeAcks: [],
    releaseVolumeAck() {
      volumeHarness.pendingVolumeAcks.shift()?.();
    },
    isVolumeCommandInFlight: () => activeVolumeCommands > 0,
    snapshot: () => ({ ...currentSnapshot }),
  };

  const clone = (value) => structuredClone(value);
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  globalThis.isTauri = true;
  window.__volumeHarness = volumeHarness;
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      switch (command) {
        case 'plugin:event|listen':
          return 1;
        case 'plugin:event|unlisten':
          return null;
        case 'get_runtime_capabilities':
          return {
            platform: 'windows',
            desktopRuntime: ready,
            library: ready,
            sourceSync: ready,
            playback: ready,
            playbackNavigation: ready,
            playbackModes: ready,
            playlistExchange: ready,
            systemMediaControls: unavailable,
          };
        case 'settings_get_recovery_warning':
          return null;
        case 'settings_source_registry_authoritative':
          return true;
        case 'theme_get_preferences':
          return { backgroundHex: '#000000', accentHex: '#55D9FF' };
        case 'theme_set_preferences':
          return args;
        case 'library_list_sources':
          return [];
        case 'library_sync':
          return { sources: [] };
        case 'library_get_page':
          return { items: [], offset: args.offset ?? 0, limit: args.limit ?? 40, totalCount: 0 };
        case 'playlist_list':
          return [];
        case 'playback_get_snapshot':
          return clone(currentSnapshot);
        case 'library_get_track_artwork':
          return new ArrayBuffer(0);
        case 'playback_set_volume': {
          const volume = Number(args.volume);
          volumeHarness.volumeRequests.push(volume);
          activeVolumeCommands += 1;
          volumeHarness.maxConcurrentVolumeCommands = Math.max(
            volumeHarness.maxConcurrentVolumeCommands,
            activeVolumeCommands,
          );
          if (volumeHarness.holdVolumeAcks) {
            await new Promise((resolve) => volumeHarness.pendingVolumeAcks.push(resolve));
          } else {
            await delay(160);
          }
          currentSnapshot = { ...currentSnapshot, volume };
          activeVolumeCommands -= 1;
          return clone(currentSnapshot);
        }
        default:
          throw new Error(`Unexpected command in volume harness: ${command}`);
      }
    },
    transformCallback(callback) {
      const id = String(++nextCallbackId);
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(String(id));
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener() {},
  };
})();
