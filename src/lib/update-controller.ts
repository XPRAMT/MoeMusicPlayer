import type { UpdateInstallMode, UpdateState } from './ipc';

export interface UpdateControllerPorts {
  subscribe: (receive: (state: UpdateState) => void) => Promise<() => void>;
  getState: () => Promise<UpdateState>;
  check: () => Promise<UpdateState>;
  ignore: () => Promise<UpdateState>;
  download: (mode: UpdateInstallMode) => Promise<UpdateState>;
  onChange: (state: UpdateState, visible: boolean) => void;
}

export const EMPTY_UPDATE_STATE: UpdateState = {
  status: 'idle', currentBuildId: '', latest: null,
  downloadedBytes: 0, totalBytes: null, error: null,
};

/** The backend owns update work. This controller owns notification visibility only. */
export function createUpdateController(ports: UpdateControllerPorts) {
  let state: UpdateState = { ...EMPTY_UPDATE_STATE };
  let visible = false;
  let disposed = false;
  let active = false;
  let unsubscribe: (() => void) | undefined;
  let startPromise: Promise<void> | undefined;
  let eventRevision = 0;

  function publish() {
    if (!disposed) ports.onChange(state, visible);
  }
  function receive(next: UpdateState) {
    if (disposed) return;
    state = next;
    if (next.status === 'available') visible = true;
    if (next.status === 'deferred' || next.status === 'current' || next.status === 'unpublished') visible = false;
    publish();
  }
  function fail(error: unknown) {
    if (disposed) return;
    state = { ...state, status: 'error', error: error instanceof Error ? error.message : String(error) };
    publish();
  }
  async function operate(operation: () => Promise<UpdateState>) {
    if (active || disposed) return;
    active = true;
    try { receive(await operation()); } catch (error) { fail(error); }
    finally { active = false; }
  }
  async function check() {
    if (active || disposed || ['downloading', 'ready', 'deferred', 'restarting'].includes(state.status)) return;
    await operate(ports.check);
  }
  function start() {
    startPromise ??= (async () => {
      try {
        const stop = await ports.subscribe((next) => { eventRevision += 1; receive(next); });
        if (disposed) { stop(); return; }
        unsubscribe = stop;
        const revision = eventRevision;
        const initial = await ports.getState();
        if (disposed) return;
        if (revision === eventRevision) receive(initial);
        await check();
      } catch (error) { fail(error); }
    })();
    return startPromise;
  }
  async function ignore() {
    visible = false;
    publish();
    // Closing a progress notification leaves the backend download running.
    if (!active && !['downloading', 'ready', 'deferred', 'restarting'].includes(state.status)) await operate(ports.ignore);
  }
  async function download(mode: UpdateInstallMode) {
    if (!state.latest || !['available', 'error'].includes(state.status)) return;
    visible = true;
    await operate(() => ports.download(mode));
  }
  function show() {
    if (state.latest && ['available', 'error', 'downloading', 'ready', 'deferred'].includes(state.status)) {
      visible = true;
      publish();
    }
  }
  function dispose() { disposed = true; unsubscribe?.(); }
  return { start, check, ignore, download, show, dispose };
}

export function formatBuildTime(value: string | null | undefined, timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'Asia/Taipei'): string {
  if (!value || !/(?:Z|[+-]\d{2}:\d{2})$/i.test(value)) return '未提供';
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return '未提供';
  return new Intl.DateTimeFormat('zh-TW', {
    timeZone, year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hourCycle: 'h23', timeZoneName: 'shortOffset',
  }).format(date);
}

export function updateStatusText(state: UpdateState): string {
  switch (state.status) {
    case 'checking': return '正在檢查更新…';
    case 'current': return '目前已是最新版本。';
    case 'available': return `發現新版本 ${state.latest?.version ?? ''}`.trim();
    case 'downloading': return '正在下載更新…';
    case 'ready': return '下載完成，正在驗證更新…';
    case 'deferred': return '更新已準備完成，下次啟動會完成更新。';
    case 'restarting': return '更新已準備完成，正在重新啟動播放器…';
    case 'unpublished': return '目前沒有可用的已發布版本。';
    case 'unsupported': return '目前平台不適用 Windows 自動更新。';
    case 'error': return state.error || '更新失敗，請重試。';
    default: return '尚未檢查更新。';
  }
}

export function updateProgressPercent(state: UpdateState): number | null {
  if (state.totalBytes == null || state.totalBytes <= 0) return null;
  return Math.min(100, Math.max(0, Math.floor(state.downloadedBytes / state.totalBytes * 100)));
}
