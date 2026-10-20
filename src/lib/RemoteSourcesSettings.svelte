<script lang="ts">
  // Remote music sources (ADR 0026): the servers (accounts) and the folders of them that join the library.
  // A password is only ever typed here and sent to the backend, which seals it; the page never gets it back.
  import { onMount, type Snippet } from 'svelte';
  import {
    getErrorText,
    invokeCommand,
    type RemoteAccountInput,
    type LibrarySource,
    type RemoteAccountView,
    type RemoteExcludedView,
    type RemoteFolderView,
  } from './ipc';
  import { tr } from './i18n.js';

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

  let excluded = $state<RemoteExcludedView[]>([]);

  async function loadExcluded(): Promise<void> {
    try {
      excluded = await invokeCommand('remote_excluded_list', {});
    } catch (cause) {
      error = getErrorText(cause);
    }
  }

  onMount(() => { void loadAccounts(); void loadExcluded(); });

  // ---- the account form ----
  type Form = {
    id: string | null;
    name: string;
    protocol: 'smb' | 'webdav';
    host: string;
    port: string;
    path: string;
    username: string;
    /** Empty while editing keeps the stored password. */
    password: string;
    hasPassword: boolean;
    clearPassword: boolean;
    useHttp: boolean;
  };

  let formDialog = $state<HTMLDialogElement>();
  let form = $state<Form | null>(null);
  let formError = $state<string | null>(null);
  let testing = $state(false);
  let testResult = $state<string | null>(null);
  let saving = $state(false);

  function blankForm(): Form {
    return { id: null, name: '', protocol: 'smb', host: '', port: '', path: '', username: '', password: '', hasPassword: false, clearPassword: false, useHttp: false };
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
        }
      : blankForm();
    formError = null;
    testResult = null;
   
    formDialog?.showModal();
  }

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
      tls: current.protocol === 'webdav' && current.useHttp ? { mode: 'off' } : { mode: 'verify' },
      onlyOnWifi: false,
    };
  }

  async function testConnection(): Promise<void> {
    if (!form || testing) return;
    const input = inputOf(form);
    if (typeof input === 'string') { formError = input; return; }
    testing = true;
    formError = null;
    testResult = null;
    try {
      const result = await invokeCommand('remote_account_test', { input });
      testResult = result.folders.length === 0
        ? tr('連線成功，但這個位置沒有資料夾。')
        : tr('連線成功。最上層的資料夾：{0}', [result.folders.slice(0, 8).join('、')]);
    } catch (cause) {
      formError = getErrorText(cause);
    } finally {
      testing = false;
    }
  }

  // ---- the path picker of the form: the shares of the server, then the folders inside one ----
  let shareDialog = $state<HTMLDialogElement>();
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
      const result = await invokeCommand('remote_browse_draft', { input, path });
      if (serial !== pickSerial) return;
      pickFolders = result;
      pickPath = path;
    } catch (cause) {
      if (serial === pickSerial) pickError = getErrorText(cause);
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
    pickPath = '';
    pickError = null;
    shareDialog?.showModal();
    // Start where the form already points, else at the shares.
    void loadPick(form.path);
  }

  function useCurrentPick(): void {
    if (form && pickPath !== '') form.path = pickPath;
    shareDialog?.close();
  }

  async function saveForm(): Promise<void> {
    if (!form || saving) return;
    const input = inputOf(form);
    if (typeof input === 'string') { formError = input; return; }
    saving = true;
    formError = null;
    try {
      const saved = await invokeCommand('remote_account_save', { input });
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
  let browseMode = $state<'add' | 'exclude'>('add');
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
      const result = await invokeCommand('remote_browse', { accountId: account.id, path });
      if (serial !== browseSerial) return;
      folders = result;
      browsePath = path;
    } catch (cause) {
      if (serial === browseSerial) browseError = getErrorText(cause);
    } finally {
      if (serial === browseSerial) browseLoading = false;
    }
  }

  function startBrowse(mode: 'add' | 'exclude', account: RemoteAccountView): void {
    browseMode = mode;
    browseAccount = account;
    browsePath = '';
    folders = [];
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

  async function removeExcluded(entry: RemoteExcludedView): Promise<void> {
    error = null;
    try {
      excluded = await invokeCommand('remote_excluded_remove', { id: entry.id });
    } catch (cause) {
      error = getErrorText(cause);
    }
  }

  // ---- export and import ----
  let passDialog = $state<HTMLDialogElement>();
  let passMode = $state<'export' | 'import' | null>(null);
  let passphrase = $state('');
  let passError = $state<string | null>(null);
  let passBusy = $state(false);

  function openPassphrase(mode: 'export' | 'import'): void {
    passMode = mode;
    passphrase = '';
    passError = null;
    passDialog?.showModal();
  }

  function closePassphrase(): void {
    passDialog?.close();
    passMode = null;
    passphrase = '';
  }

  async function confirmPassphrase(): Promise<void> {
    if (!passMode || passBusy) return;
    if (passphrase.length < 8) { passError = tr('密碼至少需要 8 個字元。'); return; }
    passBusy = true;
    passError = null;
    try {
      if (passMode === 'export') {
        const name = await invokeCommand('remote_accounts_export', { passphrase });
        if (name) message = tr('已匯出遠端帳號：{0}', [name]);
      } else {
        const result = await invokeCommand('remote_accounts_import', { passphrase });
        if (result) {
          message = tr('已匯入遠端帳號：新增 {0}、更新 {1}。', [result.added, result.updated]);
          await loadAccounts();
        }
      }
      closePassphrase();
    } catch (cause) {
      passError = getErrorText(cause);
    } finally {
      passBusy = false;
    }
  }

  function protocolLabel(account: RemoteAccountView): string {
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
  <button class="soft-button settings-tab" type="button" aria-label={tr("新增遠端伺服器")} title={tr("新增遠端伺服器")} {disabled} onclick={() => openForm(null)}>＋</button>
</div>

{#if tab === 'local'}
  <div class="source-pane" role="tabpanel">
    {@render localPane()}
    {#if loading}
      <p class="source-muted-note">{tr("正在讀取遠端伺服器…")}</p>
    {:else if accounts.length === 0}
      <p class="source-muted-note">{tr("可加入 SMB（網路芳鄰）與 WebDAV 伺服器上的音樂資料夾：按上方的「＋」新增遠端伺服器。")}</p>
    {/if}
    <div class="source-action-row source-action-buttons source-folder-picker">
      <button class="soft-button" type="button" disabled={disabled || accounts.length === 0} onclick={() => openPassphrase('export')}>{tr("匯出帳號")}</button>
      <button class="soft-button" type="button" {disabled} onclick={() => openPassphrase('import')}>{tr("匯入帳號")}</button>
    </div>
    {#if error}<div class="source-error-message" role="status">{error}</div>{/if}
    {#if message}<div class="source-result-message" role="status">{message}</div>{/if}
  </div>
{:else if activeAccount}
  {@const account = activeAccount}
  {@const folderList = remoteSources.filter((source) => source.remoteAccountId === account.id)}
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

    <div class="source-action-row source-action-buttons source-folder-picker">
      <button class="soft-button" type="button" {disabled} onclick={() => startBrowse('add', account)}>{tr("選擇資料夾")}</button>
    </div>
    <div class="configured-sources" aria-live="polite">
      <div class="configured-sources-heading"><strong>{tr("資料夾")}</strong><span>{folderList.length}</span></div>
      {#if folderList.length === 0}
        <p class="source-muted-note">{tr("尚未加入資料夾。加入後會自動開始同步。")}</p>
      {:else}
        {#each folderList as source (source.id)}
          <div class="configured-source">
            <div class="configured-source-copy"><strong>{source.displayName}</strong><small class="configured-source-location" title={source.location}>{source.location}</small></div>
            <div class="configured-source-actions">
              <label><input type="checkbox" checked={source.enabled} {disabled} onchange={(event) => onToggleSource(source, event.currentTarget.checked)} />{tr("啟用")}</label>
              <button class="text-button" type="button" {disabled} onclick={() => onRemoveSource(source)}>{tr("移除")}</button>
            </div>
          </div>
        {/each}
      {/if}
    </div>

    <div class="source-action-row source-action-buttons source-folder-picker excluded-folder-picker">
      <button class="soft-button" type="button" {disabled} onclick={() => startBrowse('exclude', account)}>{tr("選擇排除資料夾")}</button>
    </div>
    <div class="configured-sources excluded-folders" aria-live="polite">
      <div class="configured-sources-heading"><strong>{tr("已排除的來源")}</strong><span>{excludedList.length}</span></div>
      {#if excludedList.length === 0}
        <p class="source-muted-note">{tr("尚未排除資料夾。排除後，同步會略過該資料夾內的曲目。")}</p>
      {:else}
        {#each excludedList as entry (entry.id)}
          <div class="configured-source">
            <div class="configured-source-copy"><strong>{entry.path.split('/').pop()}</strong><small class="configured-source-location" title={entry.path}>{entry.path}</small></div>
            <div class="configured-source-actions">
              <button class="text-button" type="button" {disabled} onclick={() => void removeExcluded(entry)}>{tr("移除")}</button>
            </div>
          </div>
        {/each}
      {/if}
    </div>
    {#if error}<div class="source-error-message" role="status">{error}</div>{/if}
    {#if message}<div class="source-result-message" role="status">{message}</div>{/if}
  </div>
{/if}

<dialog bind:this={formDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-form-title" oncancel={(event) => { event.preventDefault(); closeForm(); }}>
  {#if form}
    <div class="update-dialog-content">
      <h2 id="remote-form-title">{form.id ? tr('編輯遠端伺服器') : tr('新增遠端伺服器')}</h2>
      <div class="remote-protocol" role="group" aria-label={tr("通訊協定")}>
        <button class="soft-button" type="button" aria-pressed={form.protocol === 'smb'} disabled={Boolean(form.id)} onclick={() => { if (form) { form.protocol = 'smb'; form.path = ''; } }}>SMB</button>
        <button class="soft-button" type="button" aria-pressed={form.protocol === 'webdav'} disabled={Boolean(form.id)} onclick={() => { if (form) { form.protocol = 'webdav'; form.path = ''; } }}>WebDAV</button>
      </div>
      <label class="remote-field"><span>{tr("名稱")}</span><input type="text" bind:value={form.name} placeholder={tr('例如：家用 NAS')} /></label>
      <div class="remote-row">
        <label class="remote-field remote-grow"><span>{tr("主機")}</span><input type="text" bind:value={form.host} placeholder="192.168.0.10" autocapitalize="off" autocorrect="off" spellcheck="false" /></label>
        <label class="remote-field remote-port"><span>{tr("連接埠")}</span><input type="text" inputmode="numeric" bind:value={form.port} placeholder={form.protocol === 'smb' ? '445' : '443'} /></label>
      </div>
      {#if form.protocol === 'smb'}
        <div class="remote-field">
          <span>{tr("共用資料夾")}</span>
          <div class="remote-row">
            <div class="remote-picked" class:is-empty={form.path === ''} title={form.path}>{form.path || tr('尚未選擇')}</div>
            <button class="soft-button" type="button" disabled={form.host.trim() === ''} onclick={openSharePicker}>{tr('選擇資料夾')}</button>
          </div>
        </div>
      {:else}
        <label class="remote-field"><span>{tr("WebDAV 路徑")}</span><input type="text" bind:value={form.path} placeholder="/remote.php/dav/files/me" autocapitalize="off" spellcheck="false" /></label>
        <label class="remote-check"><input type="checkbox" bind:checked={form.useHttp} /><span>{tr("使用未加密的 http（不建議）")}</span></label>
      {/if}
      <label class="remote-field"><span>{tr("帳號")}</span><input type="text" bind:value={form.username} autocapitalize="off" autocorrect="off" spellcheck="false" autocomplete="off" /></label>
      <label class="remote-field">
        <span>{tr("密碼")}</span>
        <input type="password" bind:value={form.password} autocomplete="new-password" placeholder={form.hasPassword && !form.clearPassword ? tr('已儲存；留空表示不變') : ''} disabled={form.clearPassword} />
      </label>
      {#if form.hasPassword}
        <label class="remote-check"><input type="checkbox" bind:checked={form.clearPassword} /><span>{tr("移除已儲存的密碼")}</span></label>
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
    <h2 id="remote-share-title">{tr("選擇共用資料夾")}</h2>
    <div class="folder-browser-path" title={pickPath}>{form?.host ?? ''} / {pickPath || '…'}</div>
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
        <p class="folder-browser-empty">{tr("這個資料夾沒有子資料夾")}</p>
      {/if}
    </div>
    <div class="update-dialog-actions">
      <button class="soft-button" type="button" onclick={() => shareDialog?.close()}>{tr("取消")}</button>
      <button class="soft-button" type="button" disabled={pickLoading || pickPath === ''} onclick={useCurrentPick}>{tr("使用這個資料夾")}</button>
    </div>
  </div>
</dialog>

<dialog bind:this={browseDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-browse-title" oncancel={(event) => { event.preventDefault(); closeBrowser(); }}>
  <div class="update-dialog-content">
    <h2 id="remote-browse-title">{browseMode === 'add' ? tr('選擇遠端資料夾') : tr('選擇排除遠端資料夾')}</h2>
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
        {#if folders.length === 0 && !browseLoading && !browseError}
          <p class="folder-browser-empty">{tr("這個資料夾沒有子資料夾")}</p>
        {/if}
      </div>
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeBrowser}>{tr("取消")}</button>
        <button class="soft-button" type="button" disabled={browseLoading || (browseMode === 'exclude' && browsePath === '')} onclick={() => void confirmFolder()}>{browseMode === 'add' ? tr('使用這個資料夾') : tr('排除這個資料夾')}</button>
      </div>
    {/if}
  </div>
</dialog>

<dialog bind:this={passDialog} class="update-dialog remote-dialog floating-surface" aria-labelledby="remote-pass-title" oncancel={(event) => { event.preventDefault(); closePassphrase(); }}>
  {#if passMode}
    <div class="update-dialog-content">
      <h2 id="remote-pass-title">{passMode === 'export' ? tr('匯出遠端帳號') : tr('匯入遠端帳號')}</h2>
      <p>{passMode === 'export'
        ? tr('帳號（含密碼）會用你設定的密碼加密成一個檔案，可帶到另一台裝置匯入。請記住這個密碼，忘記就無法開啟。')
        : tr('請輸入匯出檔案時設定的密碼。匯入後，密碼會改用這台裝置的金鑰加密。')}</p>
      <label class="remote-field"><span>{tr("檔案密碼（至少 8 個字元）")}</span><input type="password" bind:value={passphrase} autocomplete="new-password" /></label>
      {#if passError}<p class="error-text" role="alert">{passError}</p>{/if}
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closePassphrase}>{tr("取消")}</button>
        <button class="soft-button" type="button" disabled={passBusy} onclick={() => void confirmPassphrase()}>{passMode === 'export' ? tr('匯出') : tr('匯入')}</button>
      </div>
    </div>
  {/if}
</dialog>

<style>
  .remote-sources { display: flex; flex-direction: column; gap: 10px; margin-top: 18px; }
  .source-tabs { margin-top: 4px; flex-wrap: wrap; }
  .source-pane { margin-top: 12px; }
  .remote-server-head { border-bottom: 0; padding-top: 0; }
  .remote-dialog .remote-field { display: grid; gap: 4px; font-size: 0.9em; }
  .remote-dialog .remote-field > span { opacity: 0.75; }
  .remote-dialog input[type='text'], .remote-dialog input[type='password'] {
    min-width: 0; width: 100%; box-sizing: border-box; font: inherit; padding: 7px 10px; border-radius: 8px;
    border: 1px solid rgb(127 127 127 / 0.4); background: rgb(127 127 127 / 0.1); color: inherit;
  }
  .remote-dialog input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
  .remote-row { display: flex; gap: 8px; align-items: flex-end; }
  .remote-grow { flex: 1 1 auto; min-width: 0; }
  .remote-port { flex: 0 0 96px; }
  .remote-protocol { display: flex; gap: 8px; }
  .remote-check { display: flex; align-items: center; gap: 8px; font-size: 0.9em; }
  .remote-picked { flex: 1 1 auto; min-width: 0; padding: 7px 10px; border-radius: 8px; border: 1px solid rgb(127 127 127 / 0.4); background: rgb(127 127 127 / 0.1); overflow-wrap: anywhere; }
  .remote-picked.is-empty { opacity: 0.55; }
  .remote-ok { color: var(--status-success, #5fcf8f); }
</style>
