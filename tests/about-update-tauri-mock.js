(() => {
  globalThis.isTauri = true;
  const callbacks = new Map();
  const events = new Map();
  let nextId = 0;
  const base = { currentBuildId: '20261008T120000Z', latest: null, downloadedBytes: 0, totalBytes: null, error: null };
  let current = { ...base, status: 'idle' };
  const latest = { buildId: '20261009T120000Z', version: '0.1.0+20261009T120000Z', publishedAt: '2026-10-09T12:00:00Z', notes: '更新測試說明\n第二行', downloadBytes: 100 };
  const harness = window.__updateHarness = {
    calls: [], nextStatus: 'unpublished', failDownload: false, pending: null,
    emit(status, extra = {}) {
      current = { ...current, status, ...extra };
      const callbackId = events.get('update-state-changed');
      callbacks.get(callbackId)?.({ event: 'update-state-changed', id: 1, payload: current });
    },
    finish() { harness.emit('deferred'); harness.pending?.(current); harness.pending = null; },
  };
  window.__TAURI_INTERNALS__ = {
    transformCallback(callback) { const id = ++nextId; callbacks.set(id, callback); return id; },
    unregisterCallback(id) { callbacks.delete(id); },
    async invoke(command, args = {}) {
      harness.calls.push({ command, args });
      if (command === 'plugin:event|listen') { events.set(args.event, args.handler); return 1; }
      if (command === 'plugin:event|unlisten') return;
      if (command === 'about_get_info') return {
        name: 'MoeMusicPlayer', author: 'XPRAMT', authorUrl: 'https://github.com/XPRAMT', repositoryUrl: 'https://github.com/XPRAMT/MoeMusicPlayer',
        buildVersion: '0.1.0+20261008T120000Z', buildTimestampUtc: '2026-10-08T12:00:00Z', gitCommit: '0123456789abcdef', architecture: 'x86_64',
        credits: [{ name: 'Svelte', version: '5.57.1', license: 'MIT', repositoryUrl: 'https://github.com/sveltejs/svelte' }],
      };
      if (command === 'update_get_state') return current;
      if (command === 'update_check') { harness.emit('checking'); harness.emit(harness.nextStatus, { latest: harness.nextStatus === 'available' ? latest : null }); return current; }
      if (command === 'update_ignore') { harness.emit('idle'); return current; }
      if (command === 'update_download') {
        if (harness.failDownload) { harness.failDownload = false; throw new Error('下載中斷，請重試。'); }
        harness.emit('downloading', { downloadedBytes: 0, totalBytes: 100, error: null });
        return await new Promise((resolve) => { harness.pending = resolve; });
      }
      if (command === 'app_open_external_url') return;
      throw new Error(`Unexpected command: ${command}`);
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event) { events.delete(event); },
  };
})();
