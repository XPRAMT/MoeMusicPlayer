(() => {
  const selectedByTrack = new Map([
    ['track-one', true],
    ['track-two', true],
    ['track-three', false],
  ]);
  const calls = [];
  const playlists = [{
    sourceId: 'playlist-one',
    displayName: '測試播放清單',
    isMyLove: true,
    writable: true,
  }];
  globalThis.isTauri = true;
  window.__favoriteDockHarness = { calls };
  window.__TAURI_INTERNALS__ = {
    async invoke(command, args = {}) {
      if (command === 'favorites_get_playlists') {
        calls.push(args.trackId);
        if (args.trackId === 'track-two') await new Promise((resolve) => setTimeout(resolve, 300));
        return {
          playlists: playlists.map((playlist) => ({
            ...playlist,
            selected: selectedByTrack.get(args.trackId) ?? false,
          })),
          myLoveSetupRequired: false,
        };
      }
      if (command === 'favorites_create_playlist') {
        calls.push('create-playlist');
        playlists.push({
          sourceId: 'playlist-created',
          displayName: '新播放清單',
          isMyLove: false,
          writable: true,
        });
        return '新播放清單';
      }
      throw new Error(`Unexpected command: ${command}`);
    },
  };
})();
