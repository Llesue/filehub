<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type AuditEntry } from '../api';
  import { language, localeOf, t } from '../i18n';
  import { showNotice } from '../notices';
  import Icon from './Icon.svelte';

  let { onBack }: { onBack: () => void } = $props();
  let entries = $state<AuditEntry[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let nextCursor = $state<string | null>(null);
  let sentinel = $state<HTMLElement | null>(null);

  async function load(cursor?: string): Promise<void> {
    if (cursor) loadingMore = true;
    else loading = true;
    try {
      const result = await api.listAudit(100, cursor);
      entries = cursor ? [...entries, ...result.entries] : result.entries;
      nextCursor = result.next_cursor;
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '审计日志加载失败'));
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  onMount(() => {
    void load();
    const onScroll = (): void => {
      if (!sentinel || !nextCursor || loadingMore) return;
      if (sentinel.getBoundingClientRect().top < window.innerHeight + 160) void load(nextCursor);
    };
    window.addEventListener('scroll', onScroll, true);
    return () => window.removeEventListener('scroll', onScroll, true);
  });

  function date(value: number): string {
    return new Date(value * 1000).toLocaleString(localeOf($language));
  }
</script>

<section class="audit-page">
  <header class="audit-header">
    <div>
      <h1>{t($language, '审计日志')}</h1>
      <p>{t($language, '操作记录，滚动自动加载')}</p>
    </div>
    <button type="button" class="back" onclick={onBack}
      ><Icon name="close" size={16} />{t($language, '返回管理')}</button
    >
  </header>
  {#if loading}<p class="muted">{t($language, '加载中…')}</p>{:else}
    <div class="audit-list">
      {#each entries as entry (entry.id)}
        <div class="audit-row">
          <span class="time">{date(entry.created_at)}</span>
          <span class="user">{entry.username ?? '—'}</span>
          <span class="ip">{entry.ip ?? '—'}</span>
          <span class="pill">{entry.action}</span>
          <span class="path">{entry.path ?? ''}</span>
          <span class="detail">{entry.detail ?? ''}</span>
        </div>
      {:else}<p class="muted">{t($language, '暂无记录。')}</p>{/each}
    </div>
    <div bind:this={sentinel} class="sentinel">
      {loadingMore ? t($language, '加载中…') : nextCursor ? t($language, '继续滚动加载') : ''}
    </div>
  {/if}
</section>

<style>
  .audit-page {
    display: flex;
    width: min(100%, 1200px);
    margin: 0 auto;
    flex-direction: column;
    gap: 18px;
  }
  .audit-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 24px;
    font-weight: 500;
  }
  p {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }
  .audit-list {
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .audit-row {
    display: grid;
    grid-template-columns: 150px 110px 130px 130px minmax(120px, 1fr) minmax(100px, 1fr);
    gap: 10px;
    padding: 9px 12px;
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
  }
  .audit-row:last-child {
    border-bottom: 0;
  }
  .time,
  .ip,
  .detail {
    color: var(--text-2);
    font-family: var(--font-mono);
  }
  .user {
    font-weight: 500;
  }
  .path {
    overflow: hidden;
    font-family: var(--font-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pill {
    width: fit-content;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 11.5px;
  }
  .sentinel {
    min-height: 32px;
    color: var(--text-2);
    font-size: 12.5px;
    text-align: center;
  }
  .muted {
    padding: 16px;
    color: var(--text-2);
  }
</style>
