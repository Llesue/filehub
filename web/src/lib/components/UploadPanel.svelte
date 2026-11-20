<script lang="ts">
  import Icon from './Icon.svelte';
  import { language, t } from '../i18n';
  import { uploadManager, type UploadItem } from '../uploads.svelte';

  let items = $derived(uploadManager.items);
  let totalBytes = $derived(items.reduce((sum, item) => sum + item.size, 0));
  let doneBytes = $derived(
    items.reduce((sum, item) => sum + item.receivedBytes + item.currentBytes, 0),
  );
  let overall = $derived(
    totalBytes > 0
      ? Math.min(100, Math.round((doneBytes / totalBytes) * 100))
      : items.length > 0
        ? 100
        : 0,
  );
  let paused = $derived(items.filter((item) => item.status === 'paused').length);
  let active = $derived(items.filter((item) => item.status === 'uploading').length);
  let queued = $derived(items.filter((item) => item.status === 'queued').length);

  function percentage(item: UploadItem): number {
    if (item.size === 0) return item.status === 'queued' || item.status === 'error' ? 0 : 100;
    return Math.min(100, Math.round(((item.receivedBytes + item.currentBytes) / item.size) * 100));
  }

  function statusLabel(item: UploadItem): string {
    if (item.status === 'queued') return t($language, '排队');
    if (item.status === 'paused') return t($language, '暂停');
    if (item.status === 'error') return t($language, '失败');
    return `${percentage(item)}%`;
  }

  function pauseLabel(item: UploadItem): string {
    return item.status === 'uploading' || item.status === 'queued'
      ? t($language, '暂停')
      : t($language, '继续');
  }
</script>

{#if items.length > 0}
  <aside class="upload-panel" aria-live="polite">
    <header class="upload-head">
      <div class="upload-summary">
        <strong>{t($language, '上传文件')}</strong>
        <span
          >{overall}% · {active > 0
            ? `${active} ${t($language, '上传中')}`
            : `${queued} ${t($language, '排队')}`}</span
        >
      </div>
      <div class="upload-actions">
        {#if paused > 0 || active > 0 || queued > 0}
          <button
            type="button"
            onclick={() => (paused > 0 ? uploadManager.resumeAll() : uploadManager.pauseAll())}
          >
            {paused > 0 ? t($language, '全部继续') : t($language, '全部暂停')}
          </button>
        {/if}
        <button type="button" class="danger" onclick={() => uploadManager.cancelAll()}>
          {t($language, '全部取消')}
        </button>
      </div>
    </header>

    <ul class="upload-list">
      {#each items as item (item.id)}
        <li class={`upload-row ${item.status}`} title={item.error || item.name}>
          <span class="upload-name">{item.name}</span>
          <span class="upload-track" aria-hidden="true">
            <span class="upload-fill" style={`width:${percentage(item)}%`}></span>
          </span>
          <span class="upload-status">{statusLabel(item)}</span>
          <span class="upload-row-actions">
            {#if item.status === 'error'}
              <button
                type="button"
                class="upload-retry"
                onclick={() => uploadManager.retry(item.id)}>{t($language, '重试')}</button
              >
            {:else}
              <button
                type="button"
                class="upload-icon"
                aria-label={pauseLabel(item)}
                title={pauseLabel(item)}
                onclick={() =>
                  item.status === 'paused'
                    ? uploadManager.resume(item.id)
                    : uploadManager.pause(item.id)}
              >
                <Icon name={item.status === 'paused' ? 'play' : 'pause'} size={13} />
              </button>
            {/if}
            <button
              type="button"
              class="upload-icon"
              aria-label={t($language, '取消')}
              title={t($language, '取消')}
              onclick={() => uploadManager.cancel(item.id)}
            >
              <Icon name="close" size={13} />
            </button>
          </span>
        </li>
      {/each}
    </ul>
  </aside>
{/if}

<style>
  .upload-panel {
    position: fixed;
    right: 20px;
    bottom: calc(20px + env(safe-area-inset-bottom, 0px));
    z-index: 86;
    display: grid;
    width: min(460px, calc(100vw - 32px));
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .upload-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }

  .upload-summary {
    display: flex;
    min-width: 0;
    align-items: baseline;
    gap: 8px;
  }

  .upload-summary strong {
    font-size: 13px;
  }

  .upload-summary span {
    color: var(--text-2);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .upload-actions {
    display: flex;
    flex: none;
    gap: 4px;
  }

  .upload-actions button,
  .upload-retry {
    padding: 2px 6px;
    border: 0;
    background: none;
    color: var(--text-2);
    font-size: 11.5px;
    cursor: pointer;
  }

  .upload-actions button:hover,
  .upload-retry:hover {
    color: var(--text);
  }

  .upload-actions .danger:hover {
    color: var(--danger);
  }

  .upload-list {
    display: grid;
    max-height: 240px;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    list-style: none;
  }

  .upload-row {
    display: grid;
    min-height: 44px;
    align-items: center;
    grid-template-columns: minmax(0, 1fr) minmax(64px, 1.1fr) 4.4em 48px;
    gap: 8px;
    border-top: 1px solid var(--border);
  }

  .upload-row:first-child {
    border-top: 0;
  }

  .upload-name {
    overflow: hidden;
    color: var(--text);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .upload-track {
    height: 6px;
    overflow: hidden;
    border-radius: 99px;
    background: var(--border);
  }

  .upload-fill {
    display: block;
    height: 100%;
    border-radius: 99px;
    background: var(--accent);
    transition: width 180ms var(--ease);
  }

  .upload-row.paused .upload-fill {
    background: var(--warning);
  }

  .upload-row.queued .upload-fill {
    background: var(--text-2);
  }

  .upload-row.error .upload-fill {
    background: var(--danger);
  }

  .upload-status {
    overflow: hidden;
    color: var(--text-2);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .upload-row.error .upload-status {
    color: var(--danger);
  }

  .upload-row-actions {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
  }

  .upload-icon {
    display: grid;
    width: 22px;
    height: 22px;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .upload-icon:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }

  .upload-icon:disabled {
    opacity: 0.4;
  }
</style>
