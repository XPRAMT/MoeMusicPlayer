<script module lang="ts">
  // The settings page and the quick-settings drawer each mount this page and must always show the same thing, so what
  // it reads and the result of its buttons live here once.
  import type { AboutInfo } from './ipc';

  const about = $state({
    backupStatus: null as string | null,
    backupError: null as string | null,
    backupBusy: false,
    /** Parts of an import waiting for the next start. */
    backupPending: [] as string[],
    /** Remote accounts from an import that still wait for their passphrase. */
    remotePending: false,
    info: null as AboutInfo | null,
    error: null as string | null,
    linkError: null as string | null,
    associationStatus: null as string | null,
    associationError: null as string | null,
    loading: true,
  });
</script>

<script lang="ts">
  import { tr } from './i18n.js';
  import { onMount } from 'svelte';
  import { IconBrandGithub, IconExternalLink, IconRefresh } from '@tabler/icons-svelte-runes';
  import { getErrorText, invokeCommand, type RuntimeCapabilities, type UpdateState } from './ipc';
  import { formatBuildTime, updateProgressPercent, updateStatusText } from './update-controller';
  import { buildBackupSelection, getBackupExportProblem } from './backup-export.js';

  const LICENSE_URL = 'https://github.com/XPRAMT/MoeMusicPlayer/blob/master/LICENSE';

  let { platform, updateState, checkUpdate, showUpdate, idPrefix = '' }: {
    platform: RuntimeCapabilities['platform'] | undefined;
    updateState: UpdateState;
    checkUpdate: () => void;
    showUpdate: () => void;
    idPrefix?: string;
  } = $props();
  const progress = $derived(updateProgressPercent(updateState));
  const updateBusy = $derived(['checking', 'downloading', 'ready', 'restarting', 'deferred'].includes(updateState.status));

  async function loadInfo() {
    about.loading = true;
    about.error = null;
    try { about.info = await invokeCommand('about_get_info', {}); }
    catch (cause) { about.error = getErrorText(cause); }
    finally { about.loading = false; }
  }
  const BACKUP_PART_NAMES: Record<string, string> = {
    settings: tr('設定'),
    library: tr('曲庫'),
    favourites: tr('我的最愛'),
  };
  const partNames = (parts: string[]) => parts.map((part) => BACKUP_PART_NAMES[part] ?? part).join(tr('、'));
  // ---- export panel (what to put into the backup) ----
  let exportDialog = $state<HTMLDialogElement>();
  let exportOpen = $state(false);
  let pick = $state({ library: true, settings: true, accounts: true, favourites: true });
  let passphrase = $state('');
  let confirm = $state('');
  let accountCount = $state(0);
  const exportProblem = $derived(getBackupExportProblem({ ...pick, passphrase, confirm }));

  async function openExport() {
    about.backupStatus = null;
    about.backupError = null;
    try { accountCount = (await invokeCommand('remote_accounts_list', {})).length; }
    catch { accountCount = 0; }
    pick = { library: true, settings: true, accounts: accountCount > 0, favourites: true };
    passphrase = '';
    confirm = '';
    exportOpen = true;
    exportDialog?.showModal();
  }
  function closeExport() {
    exportDialog?.close();
    exportOpen = false;
    passphrase = '';
    confirm = '';
  }
  async function exportBackup() {
    if (exportProblem || about.backupBusy) return;
    const selection = buildBackupSelection({ ...pick, passphrase });
    about.backupStatus = null;
    about.backupError = null;
    about.backupBusy = true;
    closeExport();
    try {
      const result = await invokeCommand('backup_export', { selection });
      if (result) about.backupStatus = tr('已匯出 {0}（{1} MiB）。', [result.fileName, (result.sizeBytes / 1048576).toFixed(1)]);
    } catch (cause) { about.backupError = tr('匯出失敗：{0}', [getErrorText(cause)]); }
    finally { about.backupBusy = false; }
  }

  // ---- remote accounts of an import: they wait for the passphrase chosen at export ----
  let accountsDialog = $state<HTMLDialogElement>();
  let accountsOpen = $state(false);
  let accountsPassphrase = $state('');
  let accountsError = $state<string | null>(null);
  let accountsBusy = $state(false);

  function openAccountsPrompt() {
    accountsPassphrase = '';
    accountsError = null;
    accountsOpen = true;
    accountsDialog?.showModal();
  }
  function closeAccountsPrompt() {
    accountsDialog?.close();
    accountsOpen = false;
    accountsPassphrase = '';
  }
  async function unlockAccounts() {
    if (accountsBusy) return;
    if ([...accountsPassphrase].length === 0) { accountsError = tr('請輸入匯出時設定的密碼。'); return; }
    accountsBusy = true;
    accountsError = null;
    try {
      const result = await invokeCommand('backup_remote_accounts_import', { passphrase: accountsPassphrase });
      about.remotePending = false;
      about.backupError = null;
      about.backupStatus = tr('已還原遠端帳號：新增 {0}、更新 {1}。', [result.added, result.updated]);
      closeAccountsPrompt();
    } catch (cause) {
      accountsError = getErrorText(cause);
    } finally { accountsBusy = false; }
  }
  async function skipAccounts() {
    try { await invokeCommand('backup_remote_accounts_discard', {}); } catch { /* nothing waiting */ }
    about.remotePending = false;
    closeAccountsPrompt();
  }

  async function importBackup() {
    about.backupStatus = null;
    about.backupError = null;
    about.backupBusy = true;
    try {
      const result = await invokeCommand('backup_import', {});
      if (result) {
        about.backupPending = result.pending;
        if (result.pending.length > 0) about.backupStatus = tr('已準備匯入：{0}。請完全關閉並重新開啟程式以套用。', [partNames(result.pending)]);
        if (result.remoteAccounts === 'needsPassphrase') {
          about.remotePending = true;
          openAccountsPrompt();
        }
      }
    } catch (cause) { about.backupError = tr('匯入失敗：{0}', [getErrorText(cause)]); }
    finally { about.backupBusy = false; }
  }
  async function openLink(event: MouseEvent, url: string) {
    event.preventDefault();
    about.linkError = null;
    try { await invokeCommand('app_open_external_url', { url }); }
    catch (cause) { about.linkError = getErrorText(cause); }
  }
  async function registerFileAssociations() {
    about.associationStatus = null;
    about.associationError = null;
    try {
      await invokeCommand('windows_register_audio_associations', {});
      about.associationStatus = tr('已登錄播放器支援的音訊格式。請在 Windows 預設應用程式設定中自行選擇 MoeMusicPlayer。');
    } catch (cause) { about.associationError = getErrorText(cause); }
  }
  async function openDefaultAppsSettings() {
    about.associationStatus = null;
    about.associationError = null;
    try { await invokeCommand('windows_open_default_apps_settings', {}); }
    catch (cause) { about.associationError = getErrorText(cause); }
  }
  async function loadPendingBackup() {
    try {
      about.backupPending = await invokeCommand('backup_pending', {});
      about.remotePending = await invokeCommand('backup_remote_pending', {});
    }
    catch { /* not available (a browser preview) */ }
  }
  onMount(() => { void loadInfo(); void loadPendingBackup(); });
</script>

<div class="about-page">
  <div class="settings-panel-header"><div><h3>{tr("關於 MoeMusicPlayer")}</h3></div></div>
  {#if about.loading}<p role="status">{tr("正在讀取版本資訊…")}</p>
  {:else if about.error}<p class="source-error-message" role="alert">{about.error}</p><button class="soft-button" type="button" onclick={() => void loadInfo()}>{tr("重新讀取")}</button>
  {:else if about.info}
    <dl class="about-details">
      <div><dt>{tr("作者")}</dt><dd><a class="about-link" href={about.info.authorUrl} onclick={(event) => void openLink(event, about.info!.authorUrl)}><IconBrandGithub size={16} aria-hidden="true" />{about.info.author}</a></dd></div>
      <div><dt>{tr("版本")}</dt><dd>{about.info.buildVersion}</dd></div>
      <div><dt>{tr("建置時間")}</dt><dd>{formatBuildTime(about.info.buildTimestampUtc)}</dd></div>
      <div><dt>{tr("提交")}</dt><dd><code>{about.info.gitCommit || tr('未提供')}</code></dd></div>
      <div><dt>{tr("架構")}</dt><dd>{about.info.architecture}</dd></div>
      <div><dt>{tr("專案")}</dt><dd><a class="about-link" href={about.info.repositoryUrl} onclick={(event) => void openLink(event, about.info!.repositoryUrl)}>GitHub<IconExternalLink size={14} aria-hidden="true" /></a></dd></div>
      <div><dt>{tr("授權")}</dt><dd><a class="about-link" href={LICENSE_URL} onclick={(event) => void openLink(event, LICENSE_URL)}>PolyForm Noncommercial 1.0.0<IconExternalLink size={14} aria-hidden="true" /></a>{tr("（原始碼公開，禁止商用）")}</dd></div>
    </dl>
  {/if}
  {#if platform === 'windows'}
    <section class="about-file-associations" aria-labelledby="about-file-associations-heading">
      <div class="settings-panel-header"><div><h3 id="about-file-associations-heading">{tr("預設播放器")}</h3><p>{tr("登錄播放器支援的音訊格式，再由 Windows 設定中選擇預設應用程式。")}</p></div></div>
      <div class="about-association-actions">
        <button class="soft-button" type="button" onclick={() => void registerFileAssociations()}>{tr("登錄音訊格式")}</button>
        <button class="soft-button" type="button" onclick={() => void openDefaultAppsSettings()}>{tr("開啟 Windows 預設應用程式設定")}</button>
      </div>
      {#if about.associationStatus}<p role="status">{about.associationStatus}</p>{/if}
      {#if about.associationError}<p class="source-error-message" role="alert">{about.associationError}</p>{/if}
    </section>
  {/if}
  <section class="about-updates" aria-labelledby="{idPrefix}about-updates-heading">
    <div class="settings-panel-header"><h3 id="{idPrefix}about-updates-heading">{tr("軟體更新")}</h3></div>
    {#if platform === 'windows' || platform === 'android'}
      <p role="status" aria-live="polite">{updateStatusText(updateState)}</p>
      <button class="soft-button" type="button" disabled={updateBusy} onclick={checkUpdate}><IconRefresh size={16} aria-hidden="true" />{tr("檢查更新")}</button>
      {#if updateState.status === 'downloading'}
        <progress aria-label={tr("更新下載進度")} max="100" value={progress ?? undefined}></progress>
        <p>{progress == null ? tr('正在下載') : `${progress}%`} · {(updateState.downloadedBytes / 1048576).toFixed(1)} MiB{updateState.totalBytes ? ` / ${(updateState.totalBytes / 1048576).toFixed(1)} MiB` : ''}</p>
      {/if}
      {#if updateState.latest && ['available', 'ignored', 'error', 'downloading', 'ready', 'deferred', 'installPermission', 'installing'].includes(updateState.status)}<button class="soft-button" type="button" onclick={showUpdate}>{tr("查看更新")}</button>{/if}
    {:else}<p>{tr("自動更新適用於 Windows 桌面程式。")}</p>{/if}
  </section>
  {#if platform === 'windows' || platform === 'android'}
    <section class="about-backup" aria-labelledby="{idPrefix}about-backup-heading">
      <div class="settings-panel-header"><div><h3 id="{idPrefix}about-backup-heading">{tr("備份與還原")}</h3><p>{tr("選擇要匯出的內容（曲庫、設定、遠端帳號、我的最愛）成一個壓縮檔，不含專輯封面與背景圖片；匯入時可以選這個壓縮檔，或其中的單一檔案，只會替換備份裡有的部分。匯入會在重新啟動後套用，原本的資料會保留在 before-import 資料夾。")}</p></div></div>
      <div class="about-association-actions">
        <button class="soft-button" type="button" disabled={about.backupBusy} onclick={() => void openExport()}>{tr("匯出")}</button>
        <button class="soft-button" type="button" disabled={about.backupBusy} onclick={() => void importBackup()}>{tr("匯入")}</button>
      </div>
      {#if about.backupStatus}<p role="status" aria-live="polite">{about.backupStatus}</p>{/if}
      {#if about.backupError}<p class="source-error-message" role="alert">{about.backupError}</p>{/if}
      {#if about.remotePending}
        <p role="status">{tr('備份裡的遠端帳號還在等待密碼。')}</p>
        <div class="about-association-actions"><button class="soft-button" type="button" onclick={openAccountsPrompt}>{tr("輸入帳號密碼")}</button></div>
      {/if}
      {#if !about.backupStatus && about.backupPending.length > 0}<p role="status">{tr('已準備匯入：{0}。請完全關閉並重新開啟程式以套用。', [partNames(about.backupPending)])}</p>{/if}
    </section>
  {/if}
  {#if about.info}
    <section class="about-credits" aria-labelledby="{idPrefix}about-credits-heading">
      <div class="settings-panel-header"><div><h3 id="{idPrefix}about-credits-heading">{tr("開源感謝")}</h3><p>{tr("感謝以下開源專案與貢獻者。授權與原始碼請參閱各專案連結。")}</p></div></div>
      <ul>
        {#each about.info.credits as credit (`${credit.name}:${credit.version}`)}
          <li><a class="about-link" href={credit.repositoryUrl} onclick={(event) => void openLink(event, credit.repositoryUrl)}>{credit.name}<IconExternalLink size={14} aria-hidden="true" /></a><span>{credit.version || '—'}</span><span>{credit.license}</span></li>
        {/each}
      </ul>
    </section>
  {/if}
  {#if about.linkError}<p class="source-error-message" role="alert">{tr("無法開啟連結：")}{about.linkError}</p>{/if}
</div>

<dialog bind:this={exportDialog} class="update-dialog backup-dialog floating-surface" aria-labelledby="{idPrefix}backup-export-title" oncancel={(event) => { event.preventDefault(); closeExport(); }}>
  {#if exportOpen}
    <div class="update-dialog-content">
      <h2 id="{idPrefix}backup-export-title">{tr("匯出備份")}</h2>
      <p>{tr("選擇要放進備份壓縮檔的內容。")}</p>
      <div class="backup-picks">
        <label class="backup-check"><input type="checkbox" bind:checked={pick.library} /><span>{tr("曲庫資料庫")}</span></label>
        <label class="backup-check"><input type="checkbox" bind:checked={pick.settings} /><span>{tr("設定")}</span></label>
        <label class="backup-check"><input type="checkbox" bind:checked={pick.accounts} disabled={accountCount === 0} /><span>{tr("帳號（遠端伺服器，含密碼）")}</span></label>
        <label class="backup-check"><input type="checkbox" bind:checked={pick.favourites} /><span>{tr("我的最愛清單")}</span></label>
      </div>
      {#if accountCount === 0}
        <p class="backup-note">{tr("尚未新增遠端伺服器帳號。")}</p>
      {:else if pick.accounts}
        <p class="backup-note">{tr("帳號密碼會用你設定的備份密碼加密。請記住這個密碼，忘記就無法還原帳號。")}</p>
        <label class="backup-field"><span>{tr("備份密碼（至少 8 個字元）")}</span><input type="text" class="secret-input" bind:value={passphrase} autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" /></label>
        <label class="backup-field"><span>{tr("再輸入一次密碼")}</span><input type="text" class="secret-input" bind:value={confirm} autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" /></label>
        {#if exportProblem === 'short' && passphrase.length > 0}<p class="error-text" role="alert">{tr("密碼至少需要 8 個字元。")}</p>{/if}
        {#if exportProblem === 'mismatch' && confirm.length > 0}<p class="error-text" role="alert">{tr("兩次輸入的密碼不一致。")}</p>{/if}
      {:else}
        <p class="backup-note">{tr("未勾選「帳號」時，設定裡的遠端伺服器仍會保留，但不含密碼，還原後需要重新輸入密碼。")}</p>
      {/if}
      {#if exportProblem === 'nothing'}<p class="error-text" role="alert">{tr("請至少選擇一項要匯出的內容。")}</p>{/if}
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={closeExport}>{tr("取消")}</button>
        <button class="soft-button" type="button" disabled={exportProblem !== null || about.backupBusy} onclick={() => void exportBackup()}>{tr("匯出")}</button>
      </div>
    </div>
  {/if}
</dialog>

<dialog bind:this={accountsDialog} class="update-dialog backup-dialog floating-surface" aria-labelledby="{idPrefix}backup-accounts-title" oncancel={(event) => { event.preventDefault(); closeAccountsPrompt(); }}>
  {#if accountsOpen}
    <div class="update-dialog-content">
      <h2 id="{idPrefix}backup-accounts-title">{tr("還原遠端帳號")}</h2>
      <p>{tr("這份備份含有遠端伺服器帳號。請輸入匯出時設定的備份密碼；還原後，密碼會改用這台裝置的金鑰加密。")}</p>
      <label class="backup-field"><span>{tr("備份密碼")}</span><input type="text" class="secret-input" bind:value={accountsPassphrase} autocomplete="off" autocapitalize="off" autocorrect="off" spellcheck="false" onkeydown={(event) => { if (event.key === 'Enter') void unlockAccounts(); }} /></label>
      {#if accountsError}<p class="error-text" role="alert">{accountsError}</p>{/if}
      <div class="update-dialog-actions">
        <button class="soft-button" type="button" onclick={() => void skipAccounts()}>{tr("略過帳號")}</button>
        <button class="soft-button" type="button" onclick={closeAccountsPrompt}>{tr("稍後")}</button>
        <button class="soft-button" type="button" disabled={accountsBusy} onclick={() => void unlockAccounts()}>{tr("還原帳號")}</button>
      </div>
    </div>
  {/if}
</dialog>

<style>
  .backup-picks { display: grid; gap: 8px; }
  .backup-check { display: flex; align-items: center; gap: 8px; }
  .backup-check input { accent-color: var(--accent); }
  .backup-note { opacity: 0.75; font-size: 0.9em; }
  .backup-field { display: grid; gap: 4px; font-size: 0.9em; }
  .backup-field > span { opacity: 0.75; }
  /* A plain text field drawn as dots: a real password field makes some Android keyboards restart their session on every key. */
  .backup-dialog .secret-input { -webkit-text-security: disc; }
  .backup-dialog input.secret-input {
    min-width: 0; width: 100%; box-sizing: border-box; font: inherit; padding: 7px 10px; border-radius: 8px;
    border: 1px solid rgb(127 127 127 / 0.4); background: rgb(127 127 127 / 0.1); color: inherit;
  }
  .backup-dialog input.secret-input:focus-visible { outline: 2px solid var(--accent); outline-offset: 1px; }
</style>
