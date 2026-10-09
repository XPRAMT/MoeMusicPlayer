<script lang="ts">
  import { onMount } from 'svelte';
  import { IconBrandGithub, IconExternalLink, IconRefresh } from '@tabler/icons-svelte-runes';
  import { getErrorText, invokeCommand, type AboutInfo, type RuntimeCapabilities, type UpdateState } from './ipc';
  import { formatBuildTime, updateProgressPercent, updateStatusText } from './update-controller';

  let { platform, updateState, checkUpdate, showUpdate, idPrefix = '' }: {
    platform: RuntimeCapabilities['platform'] | undefined;
    updateState: UpdateState;
    checkUpdate: () => void;
    showUpdate: () => void;
    idPrefix?: string;
  } = $props();
  let info = $state<AboutInfo | null>(null);
  let error = $state<string | null>(null);
  let linkError = $state<string | null>(null);
  let loading = $state(true);
  const progress = $derived(updateProgressPercent(updateState));
  const updateBusy = $derived(['checking', 'downloading', 'ready', 'restarting', 'deferred'].includes(updateState.status));

  async function loadInfo() {
    loading = true;
    error = null;
    try { info = await invokeCommand('about_get_info', {}); }
    catch (cause) { error = getErrorText(cause); }
    finally { loading = false; }
  }
  async function openLink(event: MouseEvent, url: string) {
    event.preventDefault();
    linkError = null;
    try { await invokeCommand('app_open_external_url', { url }); }
    catch (cause) { linkError = getErrorText(cause); }
  }
  onMount(() => { void loadInfo(); });
</script>

<div class="about-page">
  <div class="settings-panel-header"><div><h3>關於 MoeMusicPlayer</h3></div></div>
  {#if loading}<p role="status">正在讀取版本資訊…</p>
  {:else if error}<p class="source-error-message" role="alert">{error}</p><button class="soft-button" type="button" onclick={() => void loadInfo()}>重新讀取</button>
  {:else if info}
    <dl class="about-details">
      <div><dt>作者</dt><dd><a class="about-link" href={info.authorUrl} onclick={(event) => void openLink(event, info!.authorUrl)}><IconBrandGithub size={16} aria-hidden="true" />{info.author}</a></dd></div>
      <div><dt>版本</dt><dd>{info.buildVersion}</dd></div>
      <div><dt>建置時間</dt><dd>{formatBuildTime(info.buildTimestampUtc)}</dd></div>
      <div><dt>提交</dt><dd><code>{info.gitCommit || '未提供'}</code></dd></div>
      <div><dt>架構</dt><dd>{info.architecture}</dd></div>
      <div><dt>專案</dt><dd><a class="about-link" href={info.repositoryUrl} onclick={(event) => void openLink(event, info!.repositoryUrl)}>GitHub<IconExternalLink size={14} aria-hidden="true" /></a></dd></div>
    </dl>
  {/if}
  <section class="about-updates" aria-labelledby="{idPrefix}about-updates-heading">
    <div class="settings-panel-header">
      <h3 id="{idPrefix}about-updates-heading">軟體更新</h3>
      {#if platform === 'windows'}<button class="soft-button" type="button" disabled={updateBusy} onclick={checkUpdate}><IconRefresh size={16} aria-hidden="true" />檢查更新</button>{/if}
    </div>
    {#if platform === 'windows'}
      <p role="status" aria-live="polite">{updateStatusText(updateState)}</p>
      {#if updateState.status === 'downloading'}
        <progress aria-label="更新下載進度" max="100" value={progress ?? undefined}></progress>
        <p>{progress == null ? '正在下載' : `${progress}%`} · {(updateState.downloadedBytes / 1048576).toFixed(1)} MiB{updateState.totalBytes ? ` / ${(updateState.totalBytes / 1048576).toFixed(1)} MiB` : ''}</p>
      {/if}
      {#if updateState.latest && ['available', 'error', 'downloading', 'ready', 'deferred'].includes(updateState.status)}<button class="soft-button" type="button" onclick={showUpdate}>查看更新</button>{/if}
    {:else if platform === 'android'}<p>Android 請安裝新版 APK；Windows 自動更新不適用。</p>
    {:else}<p>自動更新適用於 Windows 桌面程式。</p>{/if}
  </section>
  {#if info}
    <section class="about-credits" aria-labelledby="{idPrefix}about-credits-heading">
      <div class="settings-panel-header"><div><h3 id="{idPrefix}about-credits-heading">開源感謝</h3><p>感謝以下開源專案與貢獻者。授權與原始碼請參閱各專案連結。</p></div></div>
      <ul>
        {#each info.credits as credit (`${credit.name}:${credit.version}`)}
          <li><a class="about-link" href={credit.repositoryUrl} onclick={(event) => void openLink(event, credit.repositoryUrl)}>{credit.name}<IconExternalLink size={14} aria-hidden="true" /></a><span>{credit.version || '—'}</span><span>{credit.license}</span></li>
        {/each}
      </ul>
    </section>
  {/if}
  {#if linkError}<p class="source-error-message" role="alert">無法開啟連結：{linkError}</p>{/if}
</div>
