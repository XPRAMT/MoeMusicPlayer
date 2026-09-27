async page => {
  await page.addInitScript(() => {
    window.__progressCalls = [];
    window.__progressListeners = {};
    window.__progressHandlerId = 0;
    window.__progressRunNumber = 0;
    window.__syncCallCount = 0;
    window.__startupProgressSent = false;
    window.__startupProgressMissed = false;
    window.__startupListenerEvents = [];
    window.__startupSyncListenerEvents = [];
    window.__sourcePickerResponse = null;
    window.isTauri = true;
    window.__TAURI_INTERNALS__ = {
      transformCallback: handler => {
        const id = ++window.__progressHandlerId;
        window.__progressListeners[id] = handler;
        return id;
      },
      invoke: async (command, args) => {
        window.__progressCalls.push({ command, args });
        const ready = { state: 'ready', detail: null };
        if (command === 'plugin:event|listen') {
          window.__progressEventHandlers ??= {};
          window.__progressEventHandlers[args.event] = args.handler;
          return window.__progressCalls.length;
        }
        if (command === 'plugin:event|unlisten') return null;
        switch (command) {
          case 'get_runtime_capabilities': {
            if (!window.__startupProgressSent) {
              window.__startupProgressSent = true;
              window.__startupListenerEvents = Object.keys(window.__progressEventHandlers ?? {}).sort();
              const callbackId = window.__progressEventHandlers?.['library-sync-progress'];
              const handler = callbackId ? window.__progressListeners[callbackId] : null;
              if (!handler) {
                window.__startupProgressMissed = true;
              } else {
                handler({
                  event: 'library-sync-progress', id: 1,
                  payload: {
                    runId: 'startup-probe', sourceId: 'startup-source', sourceIndex: 0,
                    sourceCount: 1, displayName: '啟動來源', stage: 'enumerating',
                    processed: 7, total: null, unit: 'filesystemEntries', observed: 0,
                    metadataReads: 0, unchanged: 0, errorCount: 0, outcome: null,
                  },
                });
              }
            }
            return {
              platform: 'windows', desktopRuntime: ready, library: ready, sourceSync: ready,
              playback: ready, playbackNavigation: ready, playbackModes: ready,
              playlistExchange: ready, systemMediaControls: ready,
            };
          }
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
            window.__syncCallCount += 1;
            if (window.__syncCallCount === 1) {
              window.__startupSyncListenerEvents = Object.keys(window.__progressEventHandlers ?? {}).sort();
            }
            if (window.__syncCallCount > 1) {
              window.__queueProgressRun(window.__nextSyncDisplayName ?? '手動同步來源');
              window.__nextSyncDisplayName = null;
            }
            return { sources: [] };
          case 'library_pick_windows_folder':
            if (window.__sourcePickerResponse) {
              window.__nextSyncDisplayName = window.__sourcePickerResponse.displayName;
            }
            return window.__sourcePickerResponse;
          default:
            return null;
        }
      },
    };
    window.__emitSyncProgress = payload => {
      const callbackId = window.__progressEventHandlers?.['library-sync-progress'];
      if (!callbackId) throw new Error('The app has not subscribed to library-sync-progress.');
      const handler = window.__progressListeners[callbackId];
      if (!handler) throw new Error('The sync progress callback was not registered.');
      handler({ event: 'library-sync-progress', id: 1, payload });
    };
    window.__queueProgressRun = displayName => {
      const run = ++window.__progressRunNumber;
      const base = {
        runId: `run-${run}`, sourceId: `source-${run}`, sourceIndex: 0, sourceCount: 1,
        displayName, processed: 2, total: null, unit: 'filesystemEntries',
        observed: 0, metadataReads: 0, unchanged: 0, errorCount: 0, outcome: null,
      };
      window.__emitSyncProgress({ ...base, stage: 'enumerating' });
      window.__finishCurrentProgress = () => window.__emitSyncProgress({
        ...base, stage: 'finished', processed: 1, total: 1, unit: 'tracks',
        observed: 1, metadataReads: 1, errorCount: 0, outcome: 'complete',
      });
    };
  });

  await page.reload();
  await page.waitForSelector('.app-shell');
  const progressListenerReady = await page.waitForFunction(
    () => Boolean(window.__progressEventHandlers?.['library-sync-progress']),
    undefined,
    { timeout: 1500 },
  ).then(() => true, () => false);
  if (!progressListenerReady) {
    throw new Error('The app must subscribe to library-sync-progress for startup and manual scans.');
  }
  await page.waitForFunction(() => window.__startupProgressSent, undefined, { timeout: 1500 });
  const startupListenerEvents = await page.evaluate(() => window.__startupListenerEvents);
  if (!startupListenerEvents.includes('library-sync-progress') || !startupListenerEvents.includes('library-sync-finished')) {
    throw new Error(`Startup synchronization began before both listeners were ready: ${startupListenerEvents.join(', ')}`);
  }
  if (await page.evaluate(() => window.__startupProgressMissed)) {
    throw new Error('The startup progress event was emitted before the renderer subscribed.');
  }
  await page.waitForFunction(() => window.__syncCallCount > 0, undefined, { timeout: 1500 });
  const startupSyncListenerEvents = await page.evaluate(() => window.__startupSyncListenerEvents);
  if (!startupSyncListenerEvents.includes('library-sync-progress') || !startupSyncListenerEvents.includes('library-sync-finished')) {
    throw new Error(`Startup synchronization command began before both listeners were ready: ${startupSyncListenerEvents.join(', ')}`);
  }
  const startupBanner = page.locator('[data-testid="sync-progress-banner"]');
  await startupBanner.waitFor({ timeout: 2000 });
  if (!(await startupBanner.innerText()).includes('啟動來源')) {
    throw new Error('The startup progress event emitted during capability loading must remain visible.');
  }

  const first = {
    runId: 'run-acceptance', sourceId: 'source-a', sourceIndex: 0, sourceCount: 2,
    displayName: '音樂 🌸', stage: 'enumerating', processed: 17, total: null,
    unit: 'filesystemEntries', observed: 0, metadataReads: 0, unchanged: 0,
    errorCount: 0, outcome: null,
  };
  await page.evaluate(payload => window.__emitSyncProgress(payload), first);
  const banner = page.locator('[data-testid="sync-progress-banner"]');
  await banner.waitFor({ timeout: 2000 });
  const liveText = await banner.innerText();
  if (!liveText.includes('17') || !liveText.includes('音樂 🌸')) {
    throw new Error(`Expected unknown-total file count and source label in visible progress: ${liveText}`);
  }
  if (liveText.includes('%')) throw new Error(`Unknown-total scanning must not show a percentage: ${liveText}`);

  await page.getByRole('button', { name: '來源設定' }).click();
  if (!(await page.locator('[data-testid="sync-progress-banner"]').isVisible())) {
    throw new Error('The global sync status must remain visible on the source settings page.');
  }

  await page.evaluate(payload => window.__emitSyncProgress(payload), {
    ...first, sourceId: 'source-b', sourceIndex: 1, displayName: '第二個來源', processed: 23,
  });
  const aggregatedText = await banner.innerText();
  if (!aggregatedText.includes('音樂 🌸') || !aggregatedText.includes('第二個來源')) {
    throw new Error(`Progress should aggregate independent source IDs: ${aggregatedText}`);
  }

  await page.evaluate(payload => window.__emitSyncProgress(payload), {
    ...first, stage: 'finished', processed: 1, total: 1, unit: 'tracks',
    observed: 1, metadataReads: 1, errorCount: 1, outcome: 'complete',
  });
  await page.evaluate(payload => window.__emitSyncProgress(payload), {
    ...first, sourceId: 'source-b', sourceIndex: 1, displayName: '第二個來源',
    stage: 'finished', processed: 4, total: 4, unit: 'tracks', observed: 4,
    metadataReads: 3, errorCount: 2, outcome: 'incomplete',
  });
  const finishedText = await banner.innerText();
  if (!finishedText.includes('3') || (!finishedText.includes('完成') && !finishedText.includes('需要留意'))) {
    throw new Error(`Completed summary should retain source and error counts: ${finishedText}`);
  }

  await page.getByRole('button', { name: '曲庫' }).click();
  if (!(await page.locator('[data-testid="sync-progress-banner"]').isVisible())) {
    throw new Error('The same sync summary should remain visible on the library page.');
  }

  await page.getByRole('button', { name: '重新整理' }).click();
  const manualProgress = page.locator('[data-testid="sync-progress-banner"]');
  await page.getByText('手動同步來源').waitFor({ timeout: 2000 });
  if (!(await manualProgress.innerText()).includes('背景同步進行中')) {
    throw new Error('The manual refresh entry point should show active progress.');
  }
  await page.evaluate(() => window.__finishCurrentProgress());
  await page.getByText('最近一次同步摘要').waitFor({ timeout: 2000 });

  await page.getByRole('button', { name: '來源設定' }).click();
  await page.evaluate(() => {
    window.__sourcePickerResponse = {
      id: 'selected-source', kind: 'windowsFilesystem', displayName: '新增的音樂資料夾',
      enabled: true, syncState: null, lastAttemptUtcMs: null, lastSuccessUtcMs: null, errorCount: 0,
    };
  });
  await page.getByRole('button', { name: '選擇資料夾並同步' }).click();
  await page.getByText('新增的音樂資料夾', { exact: true }).waitFor({ timeout: 2000 });
  const selectedSourceProgress = page.locator('[data-testid="sync-progress-banner"]');
  if (!(await selectedSourceProgress.innerText()).includes('背景同步進行中')) {
    throw new Error('The selected-folder entry point should show active progress.');
  }
  await page.evaluate(() => window.__finishCurrentProgress());
  await page.getByText('最近一次同步摘要').waitFor({ timeout: 2000 });

  const registeredEvents = await page.evaluate(() => Object.keys(window.__progressEventHandlers ?? {}));
  if (!registeredEvents.includes('library-sync-progress')) {
    throw new Error(`Progress event subscription missing: ${registeredEvents.join(', ')}`);
  }
  return { result: 'pass', liveText, aggregatedText, finishedText };
}
