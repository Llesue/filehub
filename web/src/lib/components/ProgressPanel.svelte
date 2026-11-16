<script lang="ts">
  import { language, t } from '../i18n';
  import { progress } from '../progress';

  let percentage = $derived(
    $progress.total > 0
      ? Math.min(100, Math.round(($progress.current / $progress.total) * 100))
      : null,
  );
</script>

{#if $progress.visible}
  <aside class={`progress-panel ${$progress.status}`} aria-live="polite">
    <div class="progress-title">
      <strong>{$progress.title}</strong>
      <span
        >{$progress.status === 'done'
          ? t($language, '完成')
          : $progress.status === 'error'
            ? t($language, '失败')
            : percentage !== null
              ? `${percentage}%`
              : t($language, '处理中')}</span
      >
    </div>
    <p>{$progress.detail || t($language, '请稍候…')}</p>
    {#if $progress.onCancel && $progress.status === 'running'}
      <button type="button" class="cancel-button" onclick={() => $progress.onCancel?.()}
        >{t($language, '取消')}</button
      >
    {/if}
    <div class="progress-track">
      <div class="progress-fill" style={`width:${percentage ?? 100}%`}></div>
    </div>
  </aside>
{/if}

<style>
  .progress-panel {
    position: fixed;
    right: 20px;
    bottom: 20px;
    z-index: 85;
    width: min(360px, calc(100vw - 40px));
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .progress-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    font-size: 13.5px;
  }

  .progress-title span {
    color: var(--text-2);
    font-size: 12px;
  }

  p {
    overflow: hidden;
    margin: 5px 0 9px;
    color: var(--text-2);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cancel-button {
    margin: 0 0 9px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text-2);
    font-size: 12px;
  }

  .progress-track {
    height: 7px;
    overflow: hidden;
    border-radius: 99px;
    background: var(--border);
  }

  .progress-fill {
    height: 100%;
    border-radius: 99px;
    background: var(--accent);
    transition: width 180ms var(--ease);
  }

  .progress-panel.error .progress-fill {
    background: var(--danger);
  }

  .progress-panel.done .progress-fill {
    background: var(--file-sheet);
  }
</style>
