(() => {
  const ready = { state: 'ready', detail: null };
  const unavailable = { state: 'unavailable', detail: 'Not included in the volume interaction test.' };
  const track = {
    id: 'volume-slider-test-track',
    title: 'Volume Slider Test',
    artist: 'hanser feat. 合作演出者',
    album: 'hanser Cover',
    albumArtist: 'Harness',
    trackNumber: 1,
    discNumber: 1,
    durationMs: 60_000,
    codec: 'flac',
    bitrateBps: null,
    sampleRateHz: 48_000,
    bitDepth: 24,
  };
  const libraryTracks = Array.from({ length: 120 }, (_, index) => ({
    ...track,
    id: `volume-slider-test-track-${index + 1}`,
    title: index === 0 ? track.title : `Volume Harness Track ${index + 1}`,
    artist: index === 0 ? track.artist : `Other artist ${index + 1}`,
    album: index === 0 ? track.album : `Other album ${index + 1}`,
    trackNumber: index + 1,
  }));
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
  let storedAppearance = JSON.parse(localStorage.getItem('__appearancePreferences') || 'null') ?? {
    backgroundBlurPx: 20,
    backgroundBrightnessPercent: 40,
  };
  let storedNowPlayingLayout = localStorage.getItem('__nowPlayingLayout') === 'b' ? 'b' : 'a';
  let storedLyricsPreferences = JSON.parse(localStorage.getItem('__lyricsPreferences') || 'null') ?? {
    showTranslation: false,
    showRomanization: false,
    inactiveOpacityPercent: 70,
    primaryFontSizePx: 14,
    auxiliaryFontSizePx: 10,
    lineGapPx: 24,
  };
  let artworkPromise;
  const appearanceHarness = {
    requests: [],
    holdAcks: false,
    pendingAcks: [],
    failNext: false,
    releaseAck() {
      appearanceHarness.pendingAcks.shift()?.();
    },
  };
  async function brightArtworkBytes() {
    if (!artworkPromise) {
      artworkPromise = new Promise((resolve, reject) => {
        const canvas = document.createElement('canvas');
        canvas.width = 800;
        canvas.height = 800;
        const context = canvas.getContext('2d');
        if (!context) return reject(new Error('Canvas is unavailable'));
        const gradient = context.createLinearGradient(0, 0, 800, 800);
        gradient.addColorStop(0, '#fff18a');
        gradient.addColorStop(0.38, '#ff9d67');
        gradient.addColorStop(0.72, '#ff7bc1');
        gradient.addColorStop(1, '#a9e9ff');
        context.fillStyle = gradient;
        context.fillRect(0, 0, 800, 800);
        context.fillStyle = 'rgba(255,255,255,.72)';
        context.beginPath();
        context.arc(250, 270, 170, 0, Math.PI * 2);
        context.fill();
        context.fillStyle = '#fffbdc';
        context.fillRect(330, 450, 310, 48);
        canvas.toBlob(async (blob) => {
          if (!blob) return reject(new Error('Canvas image encoding failed'));
          resolve(await blob.arrayBuffer());
        }, 'image/png');
      });
    }
    return artworkPromise;
  }
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
    lyricsGetCount: 0,
    libraryRequests: [],
    playRequests: [],
  };

  const clone = (value) => structuredClone(value);
  const delay = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds));
  globalThis.isTauri = true;
  window.__volumeHarness = volumeHarness;
  window.__appearanceHarness = appearanceHarness;
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
        case 'settings_get_track_list_columns':
          return { columns: [
            { id: 'title', visible: true },
            { id: 'artist', visible: true },
            { id: 'album', visible: true },
            { id: 'year', visible: true },
            { id: 'audioFormat', visible: true },
            { id: 'duration', visible: true },
          ] };
        case 'settings_get_now_playing_layout':
          return storedNowPlayingLayout;
        case 'settings_set_now_playing_layout':
          storedNowPlayingLayout = args.layout === 'b' ? 'b' : 'a';
          localStorage.setItem('__nowPlayingLayout', storedNowPlayingLayout);
          return storedNowPlayingLayout;
        case 'settings_get_now_playing_appearance_preferences':
          return clone(storedAppearance);
        case 'settings_set_now_playing_appearance_preferences': {
          const preferences = clone(args.preferences);
          appearanceHarness.requests.push(preferences);
          if (appearanceHarness.holdAcks) {
            await new Promise((resolve) => appearanceHarness.pendingAcks.push(resolve));
          } else {
            await delay(45);
          }
          if (appearanceHarness.failNext) {
            appearanceHarness.failNext = false;
            throw new Error('appearance settings unavailable');
          }
          storedAppearance = preferences;
          localStorage.setItem('__appearancePreferences', JSON.stringify(storedAppearance));
          return clone(storedAppearance);
        }
        case 'settings_get_lyrics_preferences':
          return clone(storedLyricsPreferences);
        case 'settings_set_lyrics_preferences':
          storedLyricsPreferences = clone(args.preferences);
          localStorage.setItem('__lyricsPreferences', JSON.stringify(storedLyricsPreferences));
          return clone(storedLyricsPreferences);
        case 'library_list_sources':
          return [];
        case 'library_sync':
          return { sources: [] };
        case 'library_get_page': {
          volumeHarness.libraryRequests.push(clone(args));
          let matches = libraryTracks;
          if (args.fieldFilter) matches = matches.filter((item) => item[args.fieldFilter.field] === args.fieldFilter.value);
          if (args.query) matches = matches.filter((item) => [item.title, item.artist, item.album].some((value) => value?.toLocaleLowerCase().includes(args.query.toLocaleLowerCase())));
          return {
            items: matches.slice(args.offset ?? 0, (args.offset ?? 0) + (args.limit ?? 40)),
            offset: args.offset ?? 0,
            limit: args.limit ?? 40,
            totalCount: matches.length,
          };
        }
        case 'playlist_list':
          return [];
        case 'playback_get_queue_page':
          return {
            revision: 0,
            total: 0,
            offset: args.offset ?? 0,
            cursor: null,
            currentEntryPosition: null,
            items: [],
          };
        case 'lyrics_get_track':
          volumeHarness.lyricsGetCount += 1;
          return {
            lyrics: {
              trackId: args.trackId,
              source: 'local',
              title: track.title,
              artist: track.artist,
              album: track.album,
              durationMs: track.durationMs,
              offsetMs: 0,
              synced: true,
              lines: [
                { startMs: 0, text: '晨光落在窗沿', translation: 'Morning light rests on the window', romanization: 'chen guang luo zai chuang yan' },
                { startMs: 8_000, text: '微風輕輕唱著歌，在漫長的午後沿著旋律慢慢走向遠方', translation: 'A long translated lyric line for the drawer layout check', romanization: 'wei feng qing qing chang zhe ge' },
                { startMs: 16_000, text: '沿著旋律慢慢前行', translation: null, romanization: 'yan zhe xuan lv man man qian xing' },
                { startMs: 24_000, text: '把今天交給遠方', translation: null, romanization: null },
              ],
            },
            candidates: [],
            status: 'ready',
            error: null,
          };
        case 'playback_get_snapshot':
          return clone(currentSnapshot);
        case 'playback_play': {
          volumeHarness.playRequests.push(clone(args));
          const selected = libraryTracks.find((item) => item.id === args.trackId);
          if (selected) currentSnapshot = { ...currentSnapshot, currentTrack: selected };
          return clone(currentSnapshot);
        }
        case 'library_get_track_artwork':
          return brightArtworkBytes();
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
