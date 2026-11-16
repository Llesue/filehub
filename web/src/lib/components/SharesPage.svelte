<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ShareInfo } from '../api';
  import { copyText } from '../clipboard';
  import { language, localeOf, t } from '../i18n';
  import { showNotice } from '../notices';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import FileTypeIcon from './FileTypeIcon.svelte';
  import Icon from './Icon.svelte';

  let { csrf }: { csrf: string } = $props();
  let shares = $state<ShareInfo[]>([]);
  let nextCursor = $state<string | null>(null);
  let loading = $state(true);
  let loadingMore = $state(false);
  let sentinel = $state<HTMLElement | null>(null);
  let deleteTarget = $state<ShareInfo | null>(null);
  let passwordEditId = $state<number | null>(null);
  let passwordDraft = $state('');
  let refreshing = $state(false);
  let refreshTimer: ReturnType<typeof setInterval> | null = null;

  function fileName(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
  }

  function shareLabel(share: ShareInfo): string {
    return share.kind === 'collection'
      ? t($language, '{count} 个文件', { count: share.item_count ?? 0 })
      : fileName(share.real_path);
  }

  async function load(cursor?: string): Promise<void> {
    if (cursor) loadingMore = true;
    else loading = true;
    try {
      const result = await api.listShares(cursor, 100);
      shares = cursor ? [...shares, ...result.shares] : result.shares;
      nextCursor = result.next_cursor;
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '分享列表加载失败'));
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  async function refresh(): Promise<void> {
    if (loading || loadingMore || refreshing || document.visibilityState === 'hidden') return;
    refreshing = true;
    try {
      const limit = Math.min(Math.max(shares.length, 100), 200);
      const result = await api.listShares(undefined, limit);
      if (shares.length <= 200) {
        shares = result.shares;
        nextCursor = result.next_cursor;
      } else {
        // Keep already loaded pages; update the first page in place and add new shares.
        const updates = new Map(result.shares.map((share) => [share.id, share]));
        const known = new Set(shares.map((share) => share.id));
        const added = result.shares.filter((share) => !known.has(share.id));
        shares = [...added, ...shares.map((share) => updates.get(share.id) ?? share)];
      }
    } catch (error) {
      console.warn('share list refresh failed', error);
    } finally {
      refreshing = false;
    }
  }

  onMount(() => {
    void load();
    const onScroll = (): void => {
      if (!sentinel || !nextCursor || loadingMore) return;
      if (sentinel.getBoundingClientRect().top < window.innerHeight + 160) void load(nextCursor);
    };
    const onFocus = (): void => void refresh();
    const onVisibility = (): void => {
      if (document.visibilityState === 'visible') void refresh();
    };
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('focus', onFocus);
    document.addEventListener('visibilitychange', onVisibility);
    refreshTimer = setInterval(() => void refresh(), 15_000);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('focus', onFocus);
      document.removeEventListener('visibilitychange', onVisibility);
      if (refreshTimer) clearInterval(refreshTimer);
    };
  });

  async function update(
    share: ShareInfo,
    patch: Parameters<typeof api.updateShare>[1],
  ): Promise<void> {
    try {
      await api.updateShare(share.id, patch, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '修改分享失败'));
    }
  }

  async function savePassword(share: ShareInfo): Promise<void> {
    await update(share, { password: passwordDraft, clear_password: passwordDraft === '' });
    passwordEditId = null;
    passwordDraft = '';
  }

  async function copyAddress(share: ShareInfo): Promise<void> {
    try {
      const result = await api.rotateShare(share.id, csrf);
      await copyText(`${window.location.origin}${result.url}`);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '生成分享地址失败'));
    }
  }

  function expiryDays(share: ShareInfo): string {
    if (!share.expires_at) return '0';
    const days = Math.max(0, Math.round((share.expires_at - Date.now() / 1000) / 86400));
    return [1, 7, 30, 90].includes(days) ? String(days) : '0';
  }

  async function setExpiry(share: ShareInfo, value: string): Promise<void> {
    const days = Number(value);
    if (days === 0) await update(share, { clear_expires: true });
    else await update(share, { expires_at: Math.floor(Date.now() / 1000) + days * 86400 });
  }

  function date(value: number | null): string {
    return value
      ? new Date(value * 1000).toLocaleString(localeOf($language))
      : t($language, '永久');
  }
</script>

<section class="shares-page">
  <header>
    <div>
      <h1>{t($language, '分享管理')}</h1>
      <p>{t($language, '这里只显示你创建的分享。')}</p>
    </div>
    <Icon name="share" size={24} />
  </header>
  {#if loading}<p class="muted">{t($language, '加载中…')}</p>{:else}
    <div class="share-list">
      {#each shares as share (share.id)}
        <article class="share-card" class:revoked={share.revoked}>
          <div class="share-head">
            <FileTypeIcon name={fileName(share.real_path)} isDir={share.is_dir} size={24} />
            <strong>{shareLabel(share)}</strong>
            <div class="share-actions">
              {#if passwordEditId === share.id}
                <form
                  class="inline-password"
                  onsubmit={(event) => {
                    event.preventDefault();
                    void savePassword(share);
                  }}
                >
                  <input
                    bind:value={passwordDraft}
                    placeholder={t($language, '新密码，留空清除')}
                  />
                  <button type="submit" class="small">{t($language, '保存')}</button>
                  <button type="button" class="small" onclick={() => (passwordEditId = null)}
                    >{t($language, '取消')}</button
                  >
                </form>
              {:else}
                <button
                  type="button"
                  class="inline-control"
                  onclick={() => {
                    passwordEditId = share.id;
                    passwordDraft = '';
                  }}>{share.has_password ? t($language, '有密码') : t($language, '无密码')}</button
                >
              {/if}
              <select
                value={share.read_only ? 'read' : 'write'}
                aria-label={t($language, '权限')}
                onchange={(event) =>
                  void update(share, { read_only: event.currentTarget.value === 'read' })}
              >
                <option value="write">{t($language, '读写')}</option>
                <option value="read">{t($language, '只读')}</option>
              </select>
              <select
                value={expiryDays(share)}
                aria-label={t($language, '过期时间')}
                onchange={(event) => void setExpiry(share, event.currentTarget.value)}
              >
                <option value="0">{t($language, '永久')}</option>
                <option value="1">{t($language, '1 天')}</option>
                <option value="7">{t($language, '7 天')}</option>
                <option value="30">{t($language, '30 天')}</option>
                <option value="90">{t($language, '90 天')}</option>
              </select>
              <button
                type="button"
                class="small"
                onclick={() => void update(share, { revoked: !share.revoked })}
              >
                {share.revoked ? t($language, '启用共享') : t($language, '停止共享')}
              </button>
              <button type="button" class="small danger" onclick={() => (deleteTarget = share)}
                >{t($language, '删除')}</button
              >
              <button type="button" class="small" onclick={() => void copyAddress(share)}
                >{t($language, '复制地址')}</button
              >
            </div>
          </div>
          <div class="share-meta">
            <span>{t($language, '分享日期 {date}', { date: date(share.created_at) })}</span>
            <span>{t($language, '访问次数 {count}', { count: share.visit_count })}</span>
            <span>{share.is_dir ? t($language, '目录') : t($language, '文件')}</span>
            <span>{t($language, '过期 {date}', { date: date(share.expires_at) })}</span>
            {#if share.revoked}<span class="danger-text">{t($language, '已停止')}</span>{/if}
          </div>
        </article>
      {:else}<p class="muted">{t($language, '还没有分享。')}</p>{/each}
    </div>
    <div bind:this={sentinel} class="sentinel">
      {loadingMore ? t($language, '加载中…') : nextCursor ? t($language, '继续滚动加载') : ''}
    </div>
  {/if}
</section>

{#if deleteTarget}
  <ConfirmDialog
    open
    title={t($language, '删除分享记录')}
    message={t($language, '永久删除「{name}」的分享记录？', {
      name: fileName(deleteTarget.real_path),
    })}
    confirmLabel={t($language, '删除')}
    danger
    onConfirm={() => {
      const target = deleteTarget;
      deleteTarget = null;
      if (target) {
        void (async () => {
          try {
            await api.purgeShare(target.id, csrf);
            await load();
          } catch (error) {
            showNotice(error instanceof Error ? error.message : t($language, '删除分享失败'));
          }
        })();
      }
    }}
    onCancel={() => (deleteTarget = null)}
  />
{/if}

<style>
  .shares-page {
    display: flex;
    width: min(100%, 1100px);
    margin: 0 auto;
    flex-direction: column;
    gap: 18px;
  }
  header {
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
  .share-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .share-card {
    display: grid;
    gap: 8px;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .share-card.revoked {
    opacity: 0.65;
  }
  .share-head {
    display: grid;
    width: 100%;
    min-width: 0;
    grid-template-columns: auto minmax(0, 1fr) auto;
    align-items: center;
    gap: 8px;
  }
  .share-head strong {
    overflow: hidden;
    min-width: 0;
    font-size: 14px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .share-actions {
    display: flex;
    min-width: 0;
    flex-wrap: nowrap;
    align-items: center;
    gap: 4px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .share-actions::-webkit-scrollbar {
    display: none;
  }
  .share-card {
    overflow: hidden;
  }
  .share-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    color: var(--text-2);
    font-size: 12.5px;
  }
  .small,
  .inline-control,
  select {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font-size: 12.5px;
  }
  .inline-control {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
  }
  .small.danger {
    color: var(--danger);
  }
  .inline-password {
    display: flex;
    gap: 4px;
  }
  .inline-password input {
    width: 150px;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .danger-text {
    color: var(--danger);
  }
  .sentinel {
    min-height: 30px;
    color: var(--text-2);
    font-size: 12.5px;
    text-align: center;
  }
  .muted {
    padding: 16px;
    color: var(--text-2);
  }
</style>
