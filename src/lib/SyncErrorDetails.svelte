<script lang="ts">
  import type { LibrarySource, SourceSyncResult } from './ipc';

  let { results, sources }: { results: SourceSyncResult[]; sources: LibrarySource[] } = $props();

  const resultsWithErrors = $derived(results.filter((result) => result.errorCount > 0));
  const currentErrorDetails = $derived(
    resultsWithErrors.map((result) => ({ result, errors: result.errors ?? [] })),
  );
  const savedErrorFallbacks = $derived(
    sources.filter((source) =>
      Boolean(source.lastError?.trim())
      && !results.some((result) =>
        result.sourceId === source.id && (result.errors ?? []).length > 0
      )
    ),
  );
</script>

{#if resultsWithErrors.length > 0 || savedErrorFallbacks.length > 0}
  <section class="sync-error-details" aria-label="來源同步錯誤明細">
    {#each currentErrorDetails as entry (entry.result.sourceId)}
      <details class="sync-error-group">
        <summary>
          <span>{entry.result.displayName}</span>
          <strong>{entry.result.errorCount.toLocaleString()} 個錯誤</strong>
          {#if entry.result.state === 'complete'}
            <span class="sync-complete-with-errors">掃描已完成，部分項目有錯誤</span>
          {/if}
          <span class="sync-error-disclosure">查看錯誤明細</span>
        </summary>
        {#if entry.errors.length > 0}
          <ul>
            {#each entry.errors as error, index (`${entry.result.sourceId}:${index}`)}
              <li>
                <code>{error.item ?? '來源層級'}</code>
                <span>{error.message}</span>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="sync-error-note">本次同步回報了 {entry.result.errorCount.toLocaleString()} 個錯誤，但沒有可用的逐項明細。</p>
        {/if}
        {#if entry.result.errorsTruncated ?? false}
          <p class="sync-error-note">
            為控制回傳量，僅顯示前 {entry.errors.length.toLocaleString()} 筆；本次錯誤總數為 {entry.result.errorCount.toLocaleString()} 筆。
          </p>
        {/if}
      </details>
    {/each}

    {#each savedErrorFallbacks as source (source.id)}
      <div class="sync-saved-error">
        <strong>{source.displayName}：上次記錄的同步錯誤原因</strong>
        <p>未保存逐項明細，可能來自先前同步。</p>
        <code>{source.lastError?.trim()}</code>
      </div>
    {/each}
  </section>
{/if}

<style>
  .sync-error-details {
    display: grid;
    gap: 7px;
    margin-top: 8px;
  }

  .sync-error-group,
  .sync-saved-error {
    min-width: 0;
    padding: 8px 10px;
    border: 1px solid rgba(var(--text-rgb), 0.1);
    border-radius: 8px;
    color: var(--text-soft);
    background: rgba(var(--text-rgb), 0.025);
    font-size: 12px;
    line-height: 1.5;
  }

  .sync-error-group summary {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 8px;
    cursor: pointer;
    font-size: 12px;
  }

  .sync-error-group summary:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: 4px;
  }

  .sync-error-disclosure {
    margin-left: auto;
    color: var(--accent-text);
  }

  .sync-error-group summary strong,
  .sync-saved-error strong {
    color: var(--status-danger);
    font-weight: 600;
  }

  .sync-complete-with-errors {
    color: var(--text-soft);
  }

  .sync-error-group ul {
    display: grid;
    gap: 6px;
    max-height: 240px;
    overflow: auto;
    margin: 8px 0 0;
    padding: 0 0 0 18px;
  }

  .sync-error-group li {
    min-width: 0;
    padding-left: 2px;
    overflow-wrap: anywhere;
  }

  .sync-error-group code,
  .sync-saved-error code {
    color: var(--text);
    font-family: var(--font-mono, ui-monospace, monospace);
    overflow-wrap: anywhere;
    font-size: 12px;
  }

  .sync-error-group li > span {
    display: block;
  }

  .sync-error-note,
  .sync-saved-error p {
    margin: 7px 0 0;
    color: var(--text-soft);
  }

  .sync-saved-error code {
    display: block;
    margin-top: 5px;
  }
</style>
