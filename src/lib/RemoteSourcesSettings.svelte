<script lang="ts">
  // Remote music sources (ADR 0026): the servers (accounts) and the folders of them that join the library.
  // A password is only ever typed here and sent to the backend, which seals it; the page never gets it back.
  import { onMount, tick, type Snippet } from 'svelte';
  import {
    getErrorText,
    invokeCommand,
    type RemoteAccountInput,
    type LibrarySource,
    type RemoteAccountView,
    type RemoteCacheView,
    type RemoteExcludedView,
    type RemoteFolderView,
  } from './ipc';
  import {
    ensureAndroidLocalNetworkAccess,
    shouldShowAndroidLocalNetworkAuthorizationPanel,
  } from './android-local-network-permission.js';
  import { tr } from './i18n.js';
  import { WEBDAV_ENTRY_VISIBLE } from './remote-protocols.js';
  import SourceSections from './SourceSections.svelte';

  let { disabled = false, remoteSources = [], localPane, onSourceAdded, onToggleSource, onRemoveSource }: {
    disabled?: boolean;
    /** The folders on this device (the "local" tab). */
    localPane: Snippet;
    /** The remote folders already in the library (shown in their own list). */
    remoteSources?: LibrarySource[];
    /** Called after a folder was added, so the list of sources reloads and the library syncs. */
    onSourceAdded: () => Promise<void> | void;
    onToggleSource: (source: LibrarySource, enabled: boolean) => void;
    onRemoveSource: (source: LibrarySource) => void;
  } = $props();

  let accounts = $state<RemoteAccountView[]>([]);
  let loading = $state(true);
  let message = $state<string | null>(null);
  let error = $state<string | null>(null);

  async function loadAccounts(): Promise<void> {
    try {
      accounts = await invokeCommand('remote_accounts_list', {});
    } catch (cause) {
      error = getErrorText(cause);
    } finally {
      loading = false;
    }
  }

  /** The open tab: `local` or a server's id. */
  let tab = $state<string>('local');
  const activeAccount = $derived(accounts.find((account) => account.id === tab) ?? null);
  // A tab whose server is gone falls back to the local one.
  $effect(() => {
    if (tab !== 'local' && !accounts.some((account) => account.id === tab)) tab = 'local';
  });

  // ---- the disk caches: played remote files and album cover thumbnails ----
  type CacheKind = 'remote' | 'cover';
  const CACHE_COMMANDS = {
    remote: { get: 'remote_cache_get', set: 'remote_cache_set_limit', clear: 'remote_cache_clear' },
    cover: { get: 'album_artwork_cache_stats', set: 'album_artwork_cache_set_limit', clear: 'album_artwork_cache_clear' },
  } as const;
  let caches = $state<Record<CacheKind, RemoteCacheView | null>>({ remote: null, cover: null });
  /** The sliders while they are dragged (the setting is saved when one is released). */
  let cacheLimitsMb = $state<Record<CacheKind, number>>({ remote: 1000, cover: 500 });
  let cacheBusy = $state(false);

  async function loadCache(): Promise<void> {
    for (const kind of ['remote', 'cover'] as const) {
      try {
        const view = await invokeCommand(CACHE_COMMANDS[kind].get, {});
        caches[kind] = view;
        cacheLimitsMb[kind] = view.limitMb;
      } catch (cause) {
        error = getErrorText(cause);
      }
    }
  }

  async function saveCacheLimit(kind: CacheKind): Promise<void> {
    if (cacheBusy) return;
    cacheBusy = true;
    try {
      const view = await invokeCommand(CACHE_COMMANDS[kind].set, { limitMb: cacheLimitsMb[kind] });
      caches[kind] = view;
      cacheLimitsMb[kind] = view.limitMb;
    } catch (cause) {
      error = getErrorText(cause);
    } finally {
      cacheBusy = false;
    }
  }

  async function clearCache(kind: CacheKind): Promise<void> {
    if (cacheBusy) return;
    cacheBusy = true;
    try {
      caches[kind] = await invokeCommand(CACHE_COMMANDS[kind].clear, {});
      // The pages hold the covers they already loaded; they are made again from the files after a reload.
      if (kind === 'cover') setTimeout(() => window.location.reload(), 400);
    } catch (cause) {
      error = getErrorText(cause);
    } finally {
      cacheBusy = false;
    }
  }

  /** `0.5 GB`, `1.0 GB`: the limit in decimal gigabytes. */
  function gigabytes(megabytes: number): string {
    return `${(megabytes / 1000).toFixed(1)} GB`;
  }

  /** What the cache holds, in a unit that reads well. */
  function usedText(bytes: number): string {
    if (bytes < 1_000_000) return '0 MB';
    if (bytes < 1_000_000_000) return `${Math.round(bytes / 1_000_000)} MB`;
    return `${(bytes / 1_000_000_000).toFixed(2)} GB`;
  }

  let excluded = $state<RemoteExcludedView[]>([]);

  async function loadExcluded(): Promise<void> {
    try {
      excluded = await invokeCommand('remote_excluded_list', {});
    } catch (cause) {
      error = getErrorText(cause);
    }
  }

  onMount(() => { void loadAccounts(); void loadExcluded(); void loadCache(); });

  // ---- the account form ----
  type Form = {
    id: string | null;
    name: string;
    protocol: 'smb' | 'webdav' | 'unsupported';
    host: string;
    port: string;
    path: string;
    username: string;
    /** Empty while editing keeps the stored password. */
    password: string;
    hasPassword: boolean;
    clearPassword: boolean;
    useHttp: boolean;
    /** The SHA-256 of the self-signed certificate the user trusts (WebDAV over https), or null to verify it normally. */
    pin: string | null;
  };

  let formDialog = $state<HTMLDialogElement>();
  let form = $state<Form | null>(null);
  let formError = $state<string | null>(null);
  let testing = $state(false);
  let localNetworkPermissionDialog = $state<HTMLDialogElement>();
  let localNetworkPermissionPanelOpen = $state(false);
  let localNetworkPermissionBusy = $state(false);
  let localNetworkPermissionDeniedMessage = $state<string | null>(null);
  let testResult = $state<string | null>(null);
  let saving = $state(false);

  function localNetworkPermissionError(): string {
    return tr('請允許 Android 的「本機網路」權限，MoeMusicPlayer 才能連線到區域網路伺服器。若已拒絕，請到 Android 設定中的應用程式權限開啟本機網路存取。');
  }

  function blankForm(): Form {
    return { id: null, name: '', protocol: 'smb', host: '', port: '', path: '', username: '', password: '', hasPassword: false, clearPassword: false, useHttp: false, pin: null };
  }

  function openForm(account: RemoteAccountView | null): void {
    form = account
      ? {
          id: account.id,
          name: account.name,
          protocol: account.protocol,
          host: account.host,
          port: account.port === null ? '' : String(account.port),
          path: account.path,
          username: account.username,
          password: '',
          hasPassword: account.hasPassword,
          clearPassword: false,
          useHttp: account.tls.mode === 'off',
          pin: account.tls.mode === 'pinned' ? account.tls.sha256 : null,
        }
      : blankForm();
    formError = null;
    testResult = null;
   
    formDialog?.showModal();
  }

  function requestAddRemoteServer(): void {
    if (!shouldShowAndroidLocalNetworkAuthorizationPanel()) {
      openForm(null);
      return;
    }
    localNetworkPermissionDeniedMessage = null;
    localNetworkPermissionPanelOpen = true;
  }

  function closeLocalNetworkPermissionPanel(): void {
    localNetworkPermissionPanelOpen = false;
    localNetworkPermissionDeniedMessage = null;
  }

  async function authorizeLocalNetworkAndAddServer(): Promise<void> {
    if (localNetworkPermissionBusy) return;
    localNetworkPermissionBusy = true;
    localNetworkPermissionDeniedMessage = null;
    try {
      if (await ensureAndroidLocalNetworkAccess()) {
        localNetworkPermissionPanelOpen = false;
        openForm(null);
      } else {
        localNetworkPermissionDeniedMessage = tr('尚未授予本機網路權限；允許後才能新增遠端伺服器。');
      }
    } finally {
      localNetworkPermissionBusy = false;
    }
  }

  $effect(() => {
    const dialog = localNetworkPermissionDialog;
    if (!dialog) return;
    if (localNetworkPermissionPanelOpen && !dialog.open) dialog.showModal();
    else if (!localNetworkPermissionPanelOpen && dialog.open) dialog.close();
  });

  function closeForm(): void {
    formDialog?.close();
    form = null;
  }

  /** The command input the form describes, or an error text for the first problem. */
  function inputOf(current: Form): RemoteAccountInput | string {
    const port = current.port.trim() === '' ? null : Number(current.port);
    if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) return tr('連接埠必須在 1 到 65535 之間。');
    // Null keeps the stored password; an empty string removes it.
    const password = current.clearPassword ? '' : current.password === '' && current.hasPassword ? null : current.password;
    return {
      id: current.id,
      name: current.name.trim() || current.host.trim(),
      protocol: current.protocol,
      host: current.host,
      port,
      path: current.path,
      username: current.username,
      password,
      tls: current.protocol !== 'webdav' ? { mode: 'verify' } : current.useHttp ? { mode: 'off' } : current.pin ? { mode: 'pinned', sha256: current.pin } : { mode: 'verify' },
      onlyOnWifi: false,
    };
  }

  // ---- trusting a self-signed certificate (WebDAV over https): asked once, when the form's own connection fails on
  // the certificate. An account that already trusts a certificate is not asked again when it changes: the error says
  // so, and the account has to be edited (or recreated) on purpose.
  let trustDialog = $state<HTMLDialogElement>();
  let trustFingerprint = $state<string | null>(null);
  let trustResolve: ((trusted: boolean) => void) | null = null;

  function isUntrustedCertificate(text: string): boolean {
    return text.startsWith('憑證不受信任') && !text.includes('與你先前接受的不同');
  }

  /** `ab12…` → `AB:12:…`, the way certificate viewers show it. */
  function formatFingerprint(sha256: string): string {
    return (sha256.toUpperCase().match(/.{1,2}/g) ?? []).join(':');
  }

  /** Ask whether to trust the certificate `current`'s server presents; true when the user accepted it (the form then pins it). */
  async function offerCertificateTrust(current: Form, errorText: string): Promise<boolean> {
    if (current.protocol !== 'webdav' || current.useHttp || current.pin || !isUntrustedCertificate(errorText)) return false;
    const port = current.port.trim() === '' ? null : Number(current.port);
    let sha256: string;
    try {
      sha256 = await invokeCommand('remote_certificate_probe', { host: current.host, port });
    } catch {
      return false;
    }
    trustFingerprint = sha256;
    trustDialog?.showModal();
    const trusted = await new Promise<boolean>((resolve) => { trustResolve = resolve; });
    trustDialog?.close();
    trustFingerprint = null;
    if (trusted && form === current) current.pin = sha256;
    return trusted && form === current;
  }

  function answerTrust(trusted: boolean): void {
    const resolve = trustResolve;
    trustResolve = null;
    resolve?.(trusted);
  }

  async function testConnection(): Promise<void> {
    if (!form || testing) return;
    const input = inputOf(form);
    if (typeof input === 'string') { formError = input; return; }
    testing = true;
    formError = null;
    testResult = null;
    try {
      if (!await ensureAndroidLocalNetworkAccess()) {
        formError = localNetworkPermissionError();
        return;
      }
      let result;
      try {
        result = await invokeCommand('remote_account_test', { input });
      } catch (cause) {
        const current = form;
        if (!current || !await offerCertificateTrust(current, getErrorText(cause))) throw cause;
        const trusted = inputOf(current);
        if (typeof trusted === 'string') throw cause;
        result = await invokeCommand('remote_account_test', { input: trusted });
      }
      testResult = result.folders.length === 0
        ? tr('連線成功，但這個位置沒有資料夾。')
        : tr('連線成功。最上層的資料夾：{0}', [result.folders.slice(0, 8).join('、')]);
    } catch (cause) {
      formError = getErrorText(cause);
    } finally {
      testing = false;
    }
  }

  // ---- the root picker of the form: browse the shares of the server and the folders inside them; "use this folder" takes the one the picker is in ----
  let shareDialog = $state<HTMLDialogElement>();
  /** Where the picker is: empty is the list of shares, otherwise `share/folder/...`. */
  let pickPath = $state('');
  let pickFolders = $state<RemoteFolderView[]>([]);
  let pickError = $state<string | null>(null);
  let pickLoading = $state(false);
  let pickSerial = 0;

  async function loadPick(path: string): Promise<void> {
    if (!form) return;
    const input = inputOf(form);
    if (typeof input === 'string') { pickError = input; return; }
    const serial = ++pickSerial;
    pickLoading = true;
    pickError = null;
    try {
      if (!await ensureAndroidLocalNetworkAccess()) {
        if (serial === pickSerial) pickError = localNetworkPermissionError();
        return;
      }
      const result = await invokeCommand('remote_browse_draft', { input, path });
      if (serial !== pickSerial) return;
      pickFolders = result;
      pickPath = path;
    } catch (cause) {
      const text = getErrorText(cause);
      if (serial === pickSerial && form && await offerCertificateTrust(form, text)) {
        pickLoading = false;
        return loadPick(path);
      }
      if (serial === pickSerial) pickError = text;
    } finally {
      if (serial === pickSerial) pickLoading = false;
    }
  }

  function openSharePicker(): void {
    if (!form) return;
    const input = inputOf(form);
    if (typeof input === 'string') { formError = input; return; }
    formError = null;
    pickFolders = [];
    pickError = null;
    // WebDAV: start in the folder already typed (relative to the server's /); SMB: at the list of shares.
    const start = form.protocol === 'webdav' ? form.path.replace(/^\/+|\/+$/g, '') : '';
    pickPath = start;
    shareDialog?.showModal();
    void loadPick(start);
  }

  /** The folder the picker is in becomes the root of the account (not one of the folders listed in it). */
  function usePickedFolder(): void {
    if (!form) return;
    if (form.protocol === 'webdav') {
      form.path = pickPath === '' ? '' : `/${pickPath}`;
    } else {
      if (pickPath === '') return;
      form.path = pickPath;
    }
    shareDialog?.close();
  }

  async function saveForm(): Promise<void> {
    if (!form || saving) return;
    const input = inputOf(form);
    if (typeof input === 'string') { formError = input; return; }
    saving = true;
    formError = null;
    try {
      let toSave = input;
      if (form.protocol === 'webdav' && !form.useHttp && !form.pin) {
        // Saving alone does not connect: try once so a self-signed certificate is offered before the account is stored
        // without it (any other failure is left to the folder browser to report, as for SMB).
        const current = form;
        try {
          await invokeCommand('remote_account_test', { input });
        } catch (cause) {
          if (await offerCertificateTrust(current, getErrorText(cause))) {
            const trusted = inputOf(current);
            if (typeof trusted !== 'string') toSave = trusted;
          }
        }
      }
      const saved = await invokeCommand('remote_account_save', { input: toSave });
      await loadAccounts();
      tab = saved.id;
      closeForm();
    } catch (cause) {
      formError = getErrorText(cause);
    } finally {
      saving = false;
    }
  }

  async function deleteAccount(account: RemoteAccountView): Promise<void> {
    error = null;
    message = null;
    try {
      await invokeCommand('remote_account_delete', { id: account.id });
      await loadAccounts();
      await loadExcluded();
    } catch (cause) {
      error = getErrorText(cause);
    }
  }

  // ---- the folder browser: pick a server (when there are several), then a folder on it ----
  let browseDialog = $state<HTMLDialogElement>();
  let browseMode = $state<'add' | 'exclude' | 'playlist'>('add');
  /** The playlist files of the folder being browsed (the 'playlist' mode lists them with the folders). */
  let playlistFiles = $state<RemoteFolderView[]>([]);
  let browseAccount = $state<RemoteAccountView | null>(null);
  let browsePath = $state('');
  let folders = $state<RemoteFolderView[]>([]);
  let browseError = $state<string | null>(null);
  let browseLoading = $state(false);
  let browseSerial = 0;

  async function loadFolders(path: string): Promise<void> {
    const account = browseAccount;
    if (!account) return;
    const serial = ++browseSerial;
    browseLoading = true;
    browseError = null;
    try {
      if (!await ensureAndroidLocalNetworkAccess()) {
        if (serial === browseSerial) browseError = localNetworkPermissionError();
        return;
      }
      if (browseMode === 'playlist') {
        const result = await invokeCommand('remote_browse_playlists', { accountId: account.id, path });
        if (serial !== browseSerial) return;
        folders = result.folders;
        playlistFiles = result.files;
      } else {
        const result = await invokeCommand('remote_browse', { accountId: account.id, path });
        if (serial !== browseSerial) return;
        folders = result;
        playlistFiles = [];
      }
      browsePath = path;
    } catch (cause) {
      if (serial === browseSerial) browseError = getErrorText(cause);
    } finally {
      if (serial === browseSerial) browseLoading = false;
    }
  }

  function startBrowse(mode: 'add' | 'exclude' | 'playlist', account: RemoteAccountView): void {
    browseMode = mode;
    browseAccount = account;
    browsePath = '';
    folders = [];
    playlistFiles = [];
    browseError = null;
    browseDialog?.showModal();
    void loadFolders('');
  }

  function closeBrowser(): void {
    browseDialog?.close();
    browseAccount = null;
  }

  function parentOf(path: string): string {
    const index = path.lastIndexOf('/');
    return index < 0 ? '' : path.slice(0, index);
  }

  async function confirmFolder(): Promise<void> {
    const account = browseAccount;
    if (!account || browseLoading) return;
    browseLoading = true;
    browseError = null;
    try {
      if (browseMode === 'add') {
        await invokeCommand('remote_source_add', { accountId: account.id, path: browsePath, displayName: null });
        closeBrowser();
        message = tr('已加入遠端資料夾，開始同步。');
        await onSourceAdded();
      } else {
        excluded = await invokeCommand('remote_excluded_add', { accountId: account.id, path: browsePath });
        closeBrowser();
        message = tr('已排除這個遠端資料夾；下次同步會略過它，並從曲庫移除其中的曲目。');
      }
    } catch (cause) {
      browseError = getErrorText(cause);
    } finally {
      browseLoading = false;
    }
  }

  /** Add the playlist file picked in the browser. */
  async function pickPlaylist(file: RemoteFolderView): Promise<void> {
    const account = browseAccount;
    if (!account || browseLoading) return;
    browseLoading = true;
    browseError = null;
    try {
      await invokeCommand('remote_playlist_add', { accountId: account.id, path: file.path });
      closeBrowser();
      message = tr('已加入遠端播放清單，開始同步。');
      await onSourceAdded();
    } catch (cause) {
      browseError = getErrorText(cause);
    } finally {
      browseLoading = false;
    }
  }

  async function removeExcluded(entry: RemoteExcludedView): Promise<void> {
    error = null;
    try {
      excluded = await invokeCommand('remote_excluded_remove', { id: entry.id });
    } catch (cause) {
      error = getErrorText(cause);
    }
  }

  function protocolLabel(account: RemoteAccountView): string {
    if (account.protocol === 'unsupported') return tr('不支援的通訊協定');
    return account.protocol === 'smb' ? 'SMB' : 'WebDAV';
  }

  function locationOf(account: RemoteAccountView): string {
    const port = account.port === null ? '' : `:${account.port}`;
    const path = account.path ? (account.protocol === 'smb' ? `/${account.path}` : account.path) : '';
    return `${account.host}${port}${path}`;
  }
</script>

<div class="settings-tabs source-tabs" role="tablist" aria-label={tr("音樂來源分頁")}>
  <button class="soft-button settings-tab" type="button" role="tab" aria-selected={tab === 'local'} onclick={() => (tab = 'local')}>{tr("本地")}</button>
  {#each accounts as account (account.id)}
    <button class="soft-button settings-tab" type="button" role="tab" aria-selected={tab === account.id} title={locationOf(account)} onclick={() => (tab = account.id)}>{account.name}</button>
  {/each}
  <button class="soft-button settings-tab" type="button" aria-label={tr("新增遠端伺服器")} title={tr("新增遠端伺服器")} {disabled} onclick={requestAddRemoteServer}>＋</button>
</div>

{#if tab === 'local'}
  <div class="source-pane" role="tabpanel">
    {@render localPane()}
    {#if loading}
      <p class="source-muted-note">{tr("正在讀取遠端伺服器…")}</p>
    {:else if accounts.length === 0}
      <p class="source-muted-note">{WEBDAV_ENTRY_VISIBLE ? tr("可加入 SMB（網路芳鄰）與 WebDAV 伺服器上的音樂資料夾：按上方的「＋」新增遠端伺服器。") : tr("可加入 SMB（網路芳鄰）伺服器上的音樂資料夾：按上方的「＋」新增遠端伺服器。")}</p>
    {/if}
    {#if error}<div class="source-error-message" role="status">{error}</div>{/if}
    {#if message}<div class="source-result-message" role="status">{message}</div>{/if}
  </div>
{:else if activeAccount}
  {@const account = activeAccount}
  {@const folderList = remoteSources.filter((source) => source.remoteAccountId === account.id && source.kind !== 'playlist_file')}
  {@const playlistList = remoteSources.filter((source) => source.remoteAccountId === account.id && source.kind === 'playlist_file')}
  {@const excludedList = excluded.filter((entry) => entry.accountId === account.id)}
  <div class="remote-sources source-pane" role="tabpanel" aria-label={account.name}>
    <div class="configured-source remote-server-head">
      <div class="configured-source-copy">
        <strong>{account.name}</strong>
        <small>{protocolLabel(account)}．{account.username || tr('匿名')}．{account.hasPassword ? tr('已儲存密碼') : tr('未設定密碼')}</small>
        <small class="configured-source-location">{locationOf(account)}</small>
      </div>
      <div class="configured-source-actions">
        <button class="text-button" type="button" {disabled} onclick={() => openForm(account)}>{tr("編輯")}</button>
        <button class="text-button" type="button" {disabled} onclick={() => void deleteAccount(account)}>{tr("刪除")}</button>
      </div>
    </div>

    <SourceSections
      folders={folderList}
      playlists={playlistList}
      excluded={excludedList.map((entry) => ({ id: entry.id, displayName: entry.path.split('/').pop() ?? entry.path, location: entry.path }))}
      {disabled}
      pickFolderDisabled={disabled}
      pickPlaylistDisabled={disabled}
      pickExcludedDisabled={disabled}
      onPickFolder={() => startBrowse('add', account)}
      onPickPlaylist={() => startBrowse('playlist', account)}
      onPickExcluded={() => startBrowse('exclude', account)}
      onToggle={(row, enabled) => { const source = remoteSources.find((item) => item.id === row.id); if (source) onToggleSource(source, enabled); }}
      onRemove={(row) => { const source = remoteSources.find((item) => item.id === row.id); if (source) onRemoveSource(source); }}
      onRemoveExcluded={(row) => { const entry = excludedList.find((item) => item.id === row.id); if (entry) void removeExcluded(entry); }}
    />
    {#if error}<div class="source-error-message" role="status">{error}</div>{/if}
    {#if message}<div class="source-result-message" role="status">{message}</div>{/if}
  </div>
{/if}

<section class="remote-sources remote-cache" aria-labelledby="cache-heading">
  <div class="settings-panel-header"><h3 id="cache-heading">{tr("快取")}</h3></div>
  {#each [
    { kind: 'remote', title: tr("遠端快取"), note: tr("播放過的遠端歌曲會暫存在本機；同一首再播放或拖回播過的位置時，不必再向伺服器讀取。超過上限時，先刪除最早存入的內容（先進先出）。") },
    { kind: 'cover', title: tr("專輯封面快取"), note: tr("專輯卡片與專輯頁的封面縮圖會暫存起來加快顯示。超過上限時，先刪除最早存入的內容；清除後，封面會在顯示時依檔案重新產生。") },
  ] as item (item.kind)}
    {@const kind = item.kind as CacheKind}
    {@const view = caches[kind]}
    <div class="source-action-heading"><strong>{item.title}</strong><span>{view ? usedText(view.usedBytes) : ''}</span></div>
    <label class="lyrics-preference-range">
      <span><strong>{tr("快取容量上限")}</strong><output>{gigabytes(cacheLimitsMb[kind])}</output></span>
      <input
        type="range"
        min="100"
        max="5000"
        step="100"
        value={cacheLimitsMb[kind]}
        aria-label={item.title + ' ' + tr("快取容量上限")}
        oninput={(event) => (cacheLimitsMb[kind] = Number(event.currentTarget.value))}
        onchange={() => void saveCacheLimit(kind)}
      />
    </label>
    <p class="source-muted-note">{item.note}</p>
    <div class="source-action-row source-action-buttons source-folder-picker">
      <button class="soft-button" type="button" disabled={cacheBusy || !view || view.usedBytes === 0} onclick={() => void clearCache(kind)}>{tr("清除快取")}</button>
    </div>
  {/each}
</section>

<dialog
  bind:this={localNetworkPermissionDialog}
  class="update-dialog remote-dialog floating-surface"
  aria-labelledby="remote-network-permission-title"
  aria-describedby="remote-network-permission-description"
  oncancel={(event) => { event.preventDefault(); closeLocalNetworkPermissionPanel(); }}
>
  <div class="update-dialog-content">
    <h2 id="remote-network-permission-title">{tr('需要「本機網路」權限')}</h2>
    <p id="remote-network-permission-description">{tr('MoeMusicPlayer 需要「本機網路」權限，才能搜尋區域網路中的伺服器並連線到遠端 SMB 伺服器。')}</p>
    <p>{tr('按下「前往授權」後，Android 會詢問是否允許。允許後會接著開啟新增遠端伺服器表單；拒絕時不會連線。')}</p>
    {#if localNetworkPermissionDeniedMessage}<p class="error-text" role="alert">{localNetworkPermissionDeniedMessage}</p>{/if}
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" disabled={localNetworkPermissionBusy} onclick={closeLocalNetworkPermissionPanel}>{tr('取消')}</button>
      <button class="soft-button" type="button" disabled={localNetworkPermissionBusy} onclick={() => void authorizeLocalNetworkAndAddServer()}>{localNetworkPermissionBusy ? tr('授權中') : tr('前往授權')}</button>
    </div>
  </div>
</dialog>

<dialog bind:this={formDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-form-title" oncancel={(event) => { event.preventDefault(); closeForm(); }}>
  {#if form}
    <div class="update-dialog-content">
      <!-- While the protocol selector is hidden (flag off), the title names the only protocol: SMB. -->
      <h2 id="remote-form-title">{WEBDAV_ENTRY_VISIBLE || form.protocol === 'webdav' ? (form.id ? tr('編輯遠端伺服器') : tr('新增遠端伺服器')) : (form.id ? tr('編輯 SMB 伺服器') : tr('新增 SMB 伺服器'))}</h2>
      <!-- New WebDAV servers are hidden while WEBDAV_ENTRY_VISIBLE is false; an existing WebDAV account still shows its (read-only) protocol. -->
      {#if WEBDAV_ENTRY_VISIBLE || form.protocol === 'webdav'}
      <div class="remote-protocol" role="group" aria-label={tr("通訊協定")}>
        <button class="soft-button" type="button" aria-pressed={form.protocol === 'smb'} disabled={Boolean(form.id)} onclick={() => { if (form) { form.protocol = 'smb'; form.path = ''; } }}>SMB</button>
        <button class="soft-button" type="button" aria-pressed={form.protocol === 'webdav'} disabled={Boolean(form.id)} onclick={() => { if (form) { form.protocol = 'webdav'; form.path = ''; } }}>WebDAV</button>
      </div>
      {/if}
      <label class="remote-field"><span>{tr("名稱")}</span><input type="text" bind:value={form.name} placeholder={tr('例如：家用 NAS')} /></label>
      <div class="remote-row">
        <label class="remote-field remote-grow"><span>{tr("主機")}</span><input type="text" bind:value={form.host} placeholder="192.168.0.10" autocapitalize="off" autocorrect="off" spellcheck="false" /></label>
        <label class="remote-field remote-port"><span>{tr("連接埠")}</span><input type="text" inputmode="numeric" bind:value={form.port} placeholder={form.protocol === 'smb' ? '445' : '443'} /></label>
      </div>
      {#if form.protocol === 'webdav'}
        <div class="remote-field">
          <span>{tr("WebDAV 路徑")}</span>
          <div class="remote-row">
            <input class="remote-grow" type="text" bind:value={form.path} placeholder="/remote.php/dav/files/me" autocapitalize="off" spellcheck="false" />
            <button class="soft-button" type="button" disabled={form.host.trim() === ''} onclick={openSharePicker}>{tr('選擇資料夾')}</button>
          </div>
        </div>
        <label class="remote-check"><input type="checkbox" bind:checked={form.useHttp} /><span>{tr("使用未加密的 http（不建議）")}</span></label>
        {#if form.pin && !form.useHttp}
          <div class="remote-field">
            <span>{tr("信任的憑證（SHA-256）")}</span>
            <div class="remote-row">
              <div class="remote-picked remote-fingerprint">{formatFingerprint(form.pin)}</div>
              <button class="soft-button" type="button" onclick={() => { if (form) form.pin = null; }}>{tr("不再信任")}</button>
            </div>
          </div>
        {/if}
      {/if}
      <label class="remote-field"><span>{tr("帳號")}</span><input type="text" bind:value={form.username} autocapitalize="off" autocorrect="off" spellcheck="false" autocomplete="off" /></label>
      <label class="remote-field">
        <span>{tr("密碼")}</span>
        <input type="text" class="secret-input" bind:value={form.password} autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" placeholder={form.hasPassword && !form.clearPassword ? tr('已儲存；留空表示不變') : ''} disabled={form.clearPassword} />
      </label>
      {#if form.hasPassword}
        <label class="remote-check"><input type="checkbox" bind:checked={form.clearPassword} /><span>{tr("移除已儲存的密碼")}</span></label>
      {/if}
      {#if form.protocol === 'smb'}
        <div class="remote-field">
          <span>{tr("根目錄")}</span>
          <div class="remote-row">
            <div class="remote-picked" class:is-empty={form.path === ''} title={form.path}>{form.path || tr('尚未選擇')}</div>
            <button class="soft-button" type="button" disabled={form.host.trim() === ''} onclick={openSharePicker}>{tr('選擇資料夾')}</button>
          </div>
        </div>
      {/if}
      {#if testResult}<p class="remote-ok" role="status">{testResult}</p>{/if}
      {#if formError}<p class="error-text" role="alert">{formError}</p>{/if}
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeForm}>{tr("取消")}</button>
        <button class="soft-button" type="button" disabled={testing || form.host.trim() === ''} onclick={() => void testConnection()}>{testing ? tr('測試中') : tr('測試連線')}</button>
        <button class="soft-button" type="button" disabled={saving || form.host.trim() === ''} onclick={() => void saveForm()}>{tr("儲存")}</button>
      </div>
    </div>
  {/if}
</dialog>

<dialog bind:this={shareDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-share-title" oncancel={(event) => { event.preventDefault(); shareDialog?.close(); }}>
  <div class="update-dialog-content">
    <h2 id="remote-share-title">{tr("選擇根目錄")}</h2>
    <div class="folder-browser-path" title={pickPath}>{form?.host ?? ''}{pickPath ? ` / ${pickPath}` : ''}</div>
    {#if pickError}<p class="error-text" role="alert">{pickError}</p>{/if}
    <div class="folder-browser-list" aria-busy={pickLoading}>
      {#if pickPath !== ''}
        <button class="folder-browser-entry" type="button" onclick={() => void loadPick(parentOf(pickPath))}>
          <span aria-hidden="true">⬆</span><span>{tr("上一層")}</span>
        </button>
      {/if}
      {#each pickFolders as folder (folder.path)}
        <button class="folder-browser-entry" type="button" onclick={() => void loadPick(folder.path)}>
          <span aria-hidden="true">📁</span><span>{folder.name}</span>
        </button>
      {/each}
      {#if pickFolders.length === 0 && !pickLoading && !pickError}
        <p class="folder-browser-empty">{pickPath === '' ? tr("沒有可用的根目錄") : tr("這個資料夾沒有子資料夾")}</p>
      {/if}
    </div>
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" onclick={() => shareDialog?.close()}>{tr("取消")}</button>
      {#if pickPath !== '' || form?.protocol === 'webdav'}
        <button class="soft-button" type="button" disabled={pickLoading} onclick={usePickedFolder}>{tr("使用這個資料夾")}</button>
      {/if}
    </div>
  </div>
</dialog>

<dialog bind:this={trustDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-trust-title" oncancel={(event) => { event.preventDefault(); answerTrust(false); }}>
  <div class="update-dialog-content">
    <h2 id="remote-trust-title">{tr("信任這個伺服器的憑證？")}</h2>
    <p>{tr("伺服器使用的憑證不是由受信任的機構簽發（例如 NAS 的自簽憑證）。請確認下方指紋與伺服器上顯示的相同；信任後只接受這張憑證，憑證改變時會停止連線並提示。")}</p>
    {#if trustFingerprint}<div class="remote-picked remote-fingerprint">{formatFingerprint(trustFingerprint)}</div>{/if}
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" onclick={() => answerTrust(false)}>{tr("取消")}</button>
      <button class="soft-button" type="button" onclick={() => answerTrust(true)}>{tr("信任")}</button>
    </div>
  </div>
</dialog>

<dialog bind:this={browseDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-browse-title" oncancel={(event) => { event.preventDefault(); closeBrowser(); }}>
  <div class="update-dialog-content">
    <h2 id="remote-browse-title">{browseMode === 'add' ? tr('選擇遠端資料夾') : browseMode === 'playlist' ? tr('選擇遠端播放清單') : tr('選擇排除遠端資料夾')}</h2>
    {#if browseAccount}
      <div class="folder-browser-path" title={browsePath}>{browseAccount.name} / {browsePath || '…'}</div>
      {#if browseError}<p class="error-text" role="alert">{browseError}</p>{/if}
      <div class="folder-browser-list" aria-busy={browseLoading}>
        {#if browsePath !== ''}
          <button class="folder-browser-entry" type="button" onclick={() => void loadFolders(parentOf(browsePath))}>
            <span aria-hidden="true">⬆</span><span>{tr("上一層")}</span>
          </button>
        {/if}
        {#each folders as folder (folder.path)}
          <button class="folder-browser-entry" type="button" onclick={() => void loadFolders(folder.path)}>
            <span aria-hidden="true">📁</span><span>{folder.name}</span>
          </button>
        {/each}
        {#each playlistFiles as file (file.path)}
          <button class="folder-browser-entry" type="button" disabled={browseLoading} onclick={() => void pickPlaylist(file)}>
            <span aria-hidden="true">🎵</span><span>{file.name}</span>
          </button>
        {/each}
        {#if folders.length === 0 && playlistFiles.length === 0 && !browseLoading && !browseError}
          <p class="folder-browser-empty">{browseMode === 'playlist' ? tr('這個資料夾沒有子資料夾或播放清單') : tr('這個資料夾沒有子資料夾')}</p>
        {/if}
      </div>
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeBrowser}>{tr("取消")}</button>
        {#if browseMode !== 'playlist'}
          <button class="soft-button" type="button" disabled={browseLoading || (browseMode === 'exclude' && browsePath === '')} onclick={() => void confirmFolder()}>{browseMode === 'add' ? tr('使用這個資料夾') : tr('排除這個資料夾')}</button>
        {/if}
      </div>
    {/if}
  </div>
</dialog>

<style>
  .remote-sources { display: flex; flex-direction: column; gap: 10px; margin-top: 18px; }
  .source-tabs { margin-top: 4px; flex-wrap: wrap; }
  .source-pane { margin-top: 12px; }
  .remote-server-head { border-bottom: 0; padding-top: 0; }
  .remote-dialog .remote-field { display: grid; gap: 4px; font-size: 0.9em; }
  .remote-dialog .remote-field > span { opacity: 0.75; }
  /* A plain text field drawn as dots: a real password field makes some Android keyboards and autofill services restart
     their session on every key, which closes the keyboard after each character. */
  .remote-dialog .secret-input { -webkit-text-security: disc; }
  .remote-dialog input[type='text'], .remote-dialog input.secret-input {
    min-width: 0; width: 100%; box-sizing: border-box; font: inherit; padding: 7px 10px; border-radius: 8px;
    border: 1px solid rgb(127 127 127 / 0.4); background: rgb(127 127 127 / 0.1); color: inherit;
  }
  .remote-dialog input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .remote-row { display: flex; gap: 8px; align-items: flex-end; }
  .remote-grow { flex: 1 1 auto; min-width: 0; }
  .remote-port { flex: 0 0 96px; }
  .remote-protocol { display: flex; gap: 8px; }
  .remote-fingerprint { font-family: ui-monospace, monospace; word-break: break-all; white-space: normal; }
  .remote-check { display: flex; align-items: center; gap: 8px; font-size: 0.9em; }
  .remote-picked { flex: 1 1 auto; min-width: 0; padding: 7px 10px; border-radius: 8px; border: 1px solid rgb(127 127 127 / 0.4); background: rgb(127 127 127 / 0.1); overflow-wrap: anywhere; }
  .remote-picked.is-empty { opacity: 0.55; }
  .remote-ok { color: var(--status-success, #5fcf8f); }
</style>
