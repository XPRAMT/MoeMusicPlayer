(() => {
  const selectedByTrack = new Map([
    ['track-one', true],
    ['track-two', true],
    ['track-three', false],
  ]);
  const calls = [];
  globalThis.isTauri = true;
  window.__favoriteDockHarness = { calls };
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      if (command === 'favorites_get_playlists') {
        calls.push(args.trackId);
        if (args.trackId === 'track-two') await new Promise((resolve) => setTimeout(resolve, 300));
        return {
          playlists: [{
            sourceId: 'playlist-one',
            displayName: '測試播放清單',
            selected: selectedByTrack.get(args.trackId) ?? false,
            isMyLove: true,
            writable: true,
          }],
          myLoveSetupRequired: false,
        };
      }
      throw new Error(`Unexpected command: ${command}`);
    },
  };
})();
