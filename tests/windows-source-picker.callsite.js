async page => {
  await page.addInitScript(() => {
    window.__sourcePickerCalls = [];
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      invoke: async (command, args) => {
        window.__sourcePickerCalls.push({ command, args });
        const ready = { state: 'ready', detail: null };
        switch (command) {
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
              systemMediaControls: ready,
            };
          case 'library_get_page':
            return { items: [], offset: args.offset, limit: args.limit, totalCount: 0 };
          case 'library_list_sources':
          case 'playlist_list':
            return [];
          case 'playback_get_snapshot':
            return {
              currentTrack: null, state: 'empty', isPlaying: false, positionMs: 0,
              durationMs: null, volume: 0.7, lastError: null, repeatMode: 'off', shuffle: false,
            };
          case 'library_sync':
            return { sources: [] };
          case 'library_pick_windows_folder':
            return null;
          case 'library_add_windows_folder':
            return {
              id: 'test-source', kind: 'windowsSystemIndex', displayName: 'Test',
              enabled: true, syncState: null, lastAttemptUtcMs: null, lastSuccessUtcMs: null,
              errorCount: 0,
            };
          default:
            return null;
        }
      },
    };
  });

  await page.reload();
  await page.waitForSelector('.app-shell');
  await page.getByRole('button', { name: '來源設定' }).click();
  await page.locator('.source-folder-form, .source-folder-picker').first().waitFor();

  const pickerButton = page.getByRole('button', { name: '選擇資料夾並同步' });
  if (await pickerButton.count()) {
    await pickerButton.click();
  } else {
    await page.locator('#windows-folder-path').fill('C:\\Acceptance\\音樂');
    await page.getByRole('button', { name: '加入並掃描' }).click();
  }

  await page.waitForTimeout(250);
  const calls = await page.evaluate(() => window.__sourcePickerCalls);
  const commands = calls.map(call => call.command);
  if (!commands.includes('library_pick_windows_folder')) {
    throw new Error(`Expected source action to invoke library_pick_windows_folder; got: ${commands.join(', ')}`);
  }
  if (commands.includes('library_add_windows_folder') || commands.includes('library_sync')) {
    throw new Error('Canceling the native folder picker must not add a source or start a sync.');
  }
  return { result: 'pass', commands };
}
