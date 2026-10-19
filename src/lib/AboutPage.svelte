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
    background: tr('背景圖片'),
  };
  const partNames = (parts: string[]) => parts.map((part) => BACKUP_PART_NAMES[part] ?? part).join(tr('、'));
  async function exportBackup() {
    about.backupStatus = null;
    about.backupError = null;
    about.backupBusy = true;
    try {
      const result = await invokeCommand('backup_export', {});
      if (result) about.backupStatus = tr('已匯出 {0}（{1} MiB）。', [result.fileName, (result.sizeBytes / 1048576).toFixed(1)]);
    } catch (cause) { about.backupError = tr('匯出失敗：{0}', [getErrorText(cause)]); }
    finally { about.backupBusy = false; }
  }
  async function importBackup() {
    about.backupStatus = null;
    about.backupError = null;
    about.backupBusy = true;
    try {
      const result = await invokeCommand('backup_import', {});
      if (result) {
        about.backupPending = result.pending;
        about.backupStatus = tr('已準備匯入：{0}。請完全關閉並重新開啟程式以套用。', [partNames(result.pending)]);
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
    try { about.backupPending = await invokeCommand('backup_pending', {}); }
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
      <div class="settings-panel-header"><div><h3 id="{idPrefix}about-backup-heading">{tr("備份與還原")}</h3><p>{tr("匯出設定與曲庫（不含專輯封面）成一個壓縮檔；匯入時可以選這個壓縮檔，或其中的單一檔案。匯入會在重新啟動後套用，原本的資料會保留在 before-import 資料夾。")}</p></div></div>
      <div class="about-association-actions">
        <button class="soft-button" type="button" disabled={about.backupBusy} onclick={() => void exportBackup()}>{tr("匯出")}</button>
        <button class="soft-button" type="button" disabled={about.backupBusy} onclick={() => void importBackup()}>{tr("匯入")}</button>
      </div>
      {#if about.backupStatus}<p role="status" aria-live="polite">{about.backupStatus}</p>{/if}
      {#if about.backupError}<p class="source-error-message" role="alert">{about.backupError}</p>{/if}
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
