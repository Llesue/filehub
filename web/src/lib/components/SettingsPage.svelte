<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type ApiTokenInfo, type ShareInfo, type User } from '../api';
  import { setLanguage, type Lang } from '../i18n';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import Icon from './Icon.svelte';
  import { showNotice } from '../notices';

  let {
    user,
    csrf,
    theme,
    lang,
    onTheme,
    initialTab = 'profile',
  }: {
    user: User;
    csrf: string;
    theme: 'light' | 'dark';
    lang: Lang;
    onTheme: (theme: 'light' | 'dark') => void;
    initialTab?: 'profile' | 'preferences' | 'shares' | 'tokens';
  } = $props();

  let tab = $state<'profile' | 'preferences' | 'shares' | 'tokens'>('profile');
  let currentPassword = $state('');
  let newPassword = $state('');
  let confirmPassword = $state('');
  let passwordMessage = $state('');
  let passwordError = $state('');
  let shares = $state<ShareInfo[]>([]);
  let shareToRevoke = $state<ShareInfo | null>(null);
  let tokens = $state<ApiTokenInfo[]>([]);
  let tokenName = $state('');
  let tokenScope = $state<'read' | 'write'>('read');
  let createdToken = $state('');
  let tokenToRevoke = $state<ApiTokenInfo | null>(null);

  onMount(() => {
    tab = initialTab;
    void loadShares();
    void loadTokens();
  });

  async function changePassword(): Promise<void> {
    passwordError = '';
    passwordMessage = '';
    if (newPassword !== confirmPassword) {
      passwordError = '两次输入的新密码不一致';
      return;
    }
    try {
      await api.changePassword(currentPassword, newPassword, csrf);
      currentPassword = '';
      newPassword = '';
      confirmPassword = '';
      passwordMessage = '密码已更新';
    } catch (err) {
      passwordError = err instanceof Error ? err.message : '修改密码失败';
    }
  }

  async function loadShares(): Promise<void> {
    try {
      shares = (await api.listShares()).shares;
    } catch (err) {
      showNotice(err instanceof Error ? err.message : '加载分享失败');
    }
  }

  async function loadTokens(): Promise<void> {
    try {
      tokens = (await api.listTokens()).tokens;
    } catch (err) {
      showNotice(err instanceof Error ? err.message : '加载 API Token 失败');
    }
  }

  async function createToken(): Promise<void> {
    const name = tokenName.trim();
    if (!name) return;
    createdToken = '';
    try {
      const created = await api.createToken(name, tokenScope, csrf);
      createdToken = created.token;
      tokenName = '';
      await loadTokens();
    } catch (err) {
      showNotice(err instanceof Error ? err.message : '创建 API Token 失败');
    }
  }

  async function revokeToken(): Promise<void> {
    if (!tokenToRevoke) return;
    try {
      await api.revokeToken(tokenToRevoke.id, csrf);
      tokenToRevoke = null;
      await loadTokens();
    } catch (err) {
      showNotice(err instanceof Error ? err.message : '撤销 API Token 失败');
    }
  }

  async function revokeShare(): Promise<void> {
    if (!shareToRevoke) return;
    try {
      await api.revokeShare(shareToRevoke.id, csrf);
      shareToRevoke = null;
      await loadShares();
    } catch (err) {
      showNotice(err instanceof Error ? err.message : '撤销分享失败');
    }
  }

  function formatBytes(bytes: number | null): string {
    if (bytes === null) return '不限制';
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
    return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  function formatDate(seconds: number | null): string {
    if (!seconds) return '永久';
    return new Intl.DateTimeFormat('zh-CN', {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(seconds * 1000));
  }
</script>

<section class="settings-page">
  <header class="page-header">
    <div>
      <h1>设置</h1>
      <p>个人资料、偏好与共享管理</p>
    </div>
    <Icon name="settings" size={24} />
  </header>

  <nav class="tabs" aria-label="设置分类">
    <button type="button" class:active={tab === 'profile'} onclick={() => (tab = 'profile')}
      >个人资料</button
    >
    <button type="button" class:active={tab === 'preferences'} onclick={() => (tab = 'preferences')}
      >偏好设置</button
    >
    <button type="button" class:active={tab === 'shares'} onclick={() => (tab = 'shares')}
      >共享设置</button
    >
    <button type="button" class:active={tab === 'tokens'} onclick={() => (tab = 'tokens')}
      >API Token</button
    >
  </nav>

  {#if tab === 'profile'}
    <section class="block">
      <h2>账号信息</h2>
      <dl>
        <dt>用户名</dt>
        <dd>{user.username}</dd>
        <dt>角色</dt>
        <dd>{user.role === 'admin' ? '管理员' : '用户'}</dd>
        <dt>配额</dt>
        <dd>{formatBytes(user.quota_bytes)}</dd>
      </dl>
    </section>
    <section class="block">
      <h2>修改密码</h2>
      <form
        class="password-form"
        onsubmit={(event) => {
          event.preventDefault();
          void changePassword();
        }}
      >
        <label
          ><span>当前密码</span><input
            bind:value={currentPassword}
            type="password"
            autocomplete="current-password"
          /></label
        >
        <label
          ><span>新密码</span><input
            bind:value={newPassword}
            type="password"
            autocomplete="new-password"
          /></label
        >
        <label
          ><span>确认新密码</span><input
            bind:value={confirmPassword}
            type="password"
            autocomplete="new-password"
          /></label
        >
        {#if passwordError}<p class="error">{passwordError}</p>{/if}
        {#if passwordMessage}<p class="success">{passwordMessage}</p>{/if}
        <button type="submit">更新密码</button>
      </form>
    </section>
  {:else if tab === 'preferences'}
    <section class="block">
      <h2>外观</h2>
      <label class="row">
        <span>主题</span>
        <select
          value={theme}
          onchange={(event) =>
            onTheme((event.currentTarget as HTMLSelectElement).value as 'light' | 'dark')}
        >
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </label>
      <label class="row">
        <span>语言</span>
        <select
          value={lang}
          onchange={(event) =>
            setLanguage((event.currentTarget as HTMLSelectElement).value as Lang)}
        >
          <option value="zh-CN">中文</option>
          <option value="en">English</option>
        </select>
      </label>
    </section>
    <section class="block">
      <h2>文件列表</h2>
      <p class="muted">列表/网格视图与排序会按目录自动记忆。</p>
    </section>
  {:else if tab === 'tokens'}
    <section class="block">
      <h2>API Token</h2>
      <p class="muted">Token 只显示一次，请立即复制保存。read 只能读取，write 可修改文件。</p>
      <form
        class="token-form"
        onsubmit={(event) => {
          event.preventDefault();
          void createToken();
        }}
      >
        <input bind:value={tokenName} placeholder="Token 名称" maxlength="64" />
        <select bind:value={tokenScope} aria-label="权限">
          <option value="read">read</option>
          <option value="write">write</option>
        </select>
        <button type="submit" disabled={!tokenName.trim()}>创建</button>
      </form>
      {#if createdToken}
        <p class="token-created">新 Token：<code>{createdToken}</code></p>
      {/if}
      <div class="share-list">
        {#each tokens as token (token.id)}
          <div class="share-row">
            <span class="mono">{token.name}</span>
            <span class="mono">{token.prefix}…</span>
            <span>{token.scope}</span>
            <span>{token.last_used_at ? formatDate(token.last_used_at) : '未使用'}</span>
            <span>{formatDate(token.expires_at)}</span>
            <button type="button" class="danger" onclick={() => (tokenToRevoke = token)}
              >撤销</button
            >
          </div>
        {/each}
      </div>
    </section>
  {:else}
    <section class="block">
      <h2>我的分享</h2>
      {#if shares.length === 0}
        <p class="muted">还没有分享。右键文件或目录即可创建分享。</p>
      {:else}
        <div class="share-list">
          {#each shares as share (share.id)}
            <div class="share-row">
              <span class="mono">{share.real_path}</span>
              <span>{share.is_dir ? '目录' : '文件'}</span>
              <span>{share.has_password ? '有密码' : '无密码'}</span>
              <span>{share.read_only ? '只读' : '读写'}</span>
              <span>{formatDate(share.expires_at)}</span>
              <span class:revoked={share.revoked}>{share.revoked ? '已撤销' : '有效'}</span>
              {#if !share.revoked}
                <button type="button" class="danger" onclick={() => (shareToRevoke = share)}
                  >撤销</button
                >
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}
</section>

<ConfirmDialog
  open={tokenToRevoke !== null}
  title="撤销 API Token"
  message={tokenToRevoke ? `撤销后该 Token 立即失效：${tokenToRevoke.name}` : ''}
  confirmLabel="撤销"
  danger
  onConfirm={() => void revokeToken()}
  onCancel={() => (tokenToRevoke = null)}
/>

<ConfirmDialog
  open={shareToRevoke !== null}
  title="撤销分享"
  message={shareToRevoke ? `撤销后链接立即失效：${shareToRevoke.real_path}` : ''}
  confirmLabel="撤销"
  danger
  onConfirm={() => void revokeShare()}
  onCancel={() => (shareToRevoke = null)}
/>

<style>
  .settings-page {
    width: min(100%, 1000px);
    margin: 0 auto;
  }

  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 18px;
    color: var(--text-2);
  }

  h1 {
    margin: 0 0 4px;
    color: var(--text);
    font-size: 24px;
    font-weight: 500;
  }

  .page-header p {
    margin: 0;
    font-size: 13.5px;
  }

  .tabs {
    display: flex;
    gap: 4px;
    margin-bottom: 18px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }

  .tabs button {
    height: 32px;
    padding: 0 12px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
    font-size: 13.5px;
  }

  .tabs button.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 500;
  }

  .block {
    margin-bottom: 24px;
  }

  h2 {
    margin: 0 0 10px;
    color: var(--text);
    font-size: 16px;
    font-weight: 500;
  }

  dl {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: 8px 16px;
    margin: 0;
    color: var(--text-2);
    font-size: 14.5px;
  }

  dd {
    margin: 0;
    color: var(--text);
  }

  .password-form {
    display: flex;
    max-width: 360px;
    flex-direction: column;
    gap: 10px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 5px;
    color: var(--text-2);
    font-size: 13.5px;
  }

  input,
  select {
    height: 36px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .password-form button,
  .share-row button {
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .row {
    max-width: 320px;
    margin-bottom: 10px;
  }

  .muted {
    color: var(--text-2);
    font-size: 13.5px;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13.5px;
  }

  .success {
    margin: 0;
    color: var(--success);
    font-size: 13.5px;
  }

  .share-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .token-form {
    display: grid;
    grid-template-columns: minmax(160px, 1fr) 90px auto;
    gap: 8px;
    margin: 12px 0;
  }

  .token-form input,
  .token-form select {
    height: 38px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  .token-form button {
    height: 38px;
    padding: 0 14px;
  }

  .token-created {
    overflow-wrap: anywhere;
    padding: 10px;
    border-radius: var(--radius-sm);
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .share-row {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) 50px 60px 50px 110px 50px auto;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    border-top: 1px solid var(--border);
    font-size: 12.5px;
  }

  .mono {
    overflow: hidden;
    font-family: var(--font-mono);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .revoked {
    color: var(--text-2);
  }

  .danger {
    color: var(--danger);
  }
</style>
