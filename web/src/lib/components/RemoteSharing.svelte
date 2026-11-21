<script lang="ts">
  import { onMount } from 'svelte';
  import {
    api,
    ApiError,
    type RemoteGrantInfo,
    type RemoteTokenInfo,
    type Root,
  } from '../api';
  import { language, t } from '../i18n';

  let { csrf }: { csrf: string } = $props();

  let grants = $state<RemoteGrantInfo[]>([]);
  let revokedGrants = $state<RemoteGrantInfo[]>([]);
  let tokens = $state<RemoteTokenInfo[]>([]);
  let activity = $state<
    Array<{ op: string; path: string | null; status: number; bytes: number; created_at: number }>
  >([]);
  let roots = $state<Root[]>([]);
  let rootId = $state<number | null>(null);
  let grantPath = $state('');
  let label = $state('');
  let mode = $state<'ro' | 'rw'>('ro');
  let tokenName = $state('');
  let selectedGrants = $state<number[]>([]);
  let expiresDays = $state(0);
  let newToken = $state('');
  let copiedKind = $state<'token' | 'invite' | null>(null);
  let browseOpen = $state(false);
  let browsePath = $state('');
  let browseDirs = $state<string[]>([]);
  let browseBusy = $state(false);
  let busy = $state(false);
  let error = $state('');

  onMount(() => {
    void load();
  });

  function message(value: unknown): string {
    if (value instanceof ApiError) return value.message;
    if (value instanceof Error) return value.message;
    return String(value);
  }

  async function load(): Promise<void> {
    try {
      const [grantResult, tokenResult, rootResult, activityResult] = await Promise.all([
        api.remoteGrants(),
        api.remoteTokens(),
        api.roots(),
        api.remoteActivity(),
      ]);
      grants = grantResult.grants.filter((grant) => !grant.revoked);
      revokedGrants = grantResult.grants.filter((grant) => grant.revoked);
      tokens = tokenResult.tokens.filter((token) => !token.revoked);
      activity = activityResult.activity.slice(0, 10);
      roots = rootResult.roots.filter((root) => root.kind !== 'remote');
      if (rootId === null && roots[0]) rootId = roots[0].id;
    } catch (value) {
      error = message(value);
    }
  }

  async function openBrowser(): Promise<void> {
    if (rootId === null) return;
    browseOpen = true;
    browsePath = grantPath.trim().replace(/^\/+|\/+$/g, '');
    await loadBrowser();
  }

  async function loadBrowser(): Promise<void> {
    if (rootId === null) return;
    browseBusy = true;
    error = '';
    try {
      const result = await api.list(rootId, browsePath, 'name', 'asc', false);
      browseDirs = result.entries.filter((entry) => entry.is_dir).map((entry) => entry.name);
    } catch (value) {
      error = message(value);
    } finally {
      browseBusy = false;
    }
  }

  function browseInto(name: string): void {
    browsePath = browsePath ? `${browsePath}/${name}` : name;
    void loadBrowser();
  }

  function browseUp(): void {
    const index = browsePath.lastIndexOf('/');
    browsePath = index < 0 ? '' : browsePath.slice(0, index);
    void loadBrowser();
  }

  function chooseBrowse(): void {
    grantPath = browsePath;
    browseOpen = false;
  }

  async function createGrant(): Promise<void> {
    if (rootId === null || !label.trim()) return;
    busy = true;
    error = '';
    try {
      await api.remoteCreateGrant(rootId, grantPath.trim(), label.trim(), mode, csrf);
      grantPath = '';
      label = '';
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function revokeGrant(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      await api.remoteRevokeGrant(id, csrf);
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function createToken(): Promise<void> {
    if (!tokenName.trim() || selectedGrants.length === 0) return;
    busy = true;
    error = '';
    newToken = '';
    copiedKind = null;
    try {
      const expiresAt =
        expiresDays > 0 ? Math.floor(Date.now() / 1000) + expiresDays * 86400 : null;
      const result = await api.remoteCreateToken(
        tokenName.trim(),
        selectedGrants,
        expiresAt,
        csrf,
      );
      newToken = result.token;
      tokenName = '';
      selectedGrants = [];
      expiresDays = 0;
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function revokeToken(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      await api.remoteRevokeToken(id, csrf);
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function copyToken(): Promise<void> {
    try {
      await navigator.clipboard.writeText(newToken);
      copiedKind = 'token';
    } catch {
      copiedKind = null;
    }
  }

  function inviteText(token: string): string {
    return `FileHub 远程目录\n地址: ${window.location.origin}\n令牌: ${token}`;
  }

  async function copyInvite(): Promise<void> {
    try {
      await navigator.clipboard.writeText(inviteText(newToken));
      copiedKind = 'invite';
    } catch {
      copiedKind = null;
    }
  }

  async function rotateToken(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      const result = await api.remoteRotateToken(id, csrf);
      newToken = result.token;
      copiedKind = null;
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  function formatDate(value: number | null): string {
    if (!value) return t($language, '从未使用');
    return new Date(value * 1000).toLocaleString();
  }
</script>

<section class="pref-card">
  <h2>{t($language, '远程授权')}</h2>
  <p class="hint">{t($language, '允许其他 FileHub 实例挂载你授权的目录。')}</p>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <div class="list">
    {#each grants as grant (grant.id)}
      <div class="row">
        <div class="meta">
          <strong>{grant.label}</strong>
          <span class="path" title={grant.path}>{grant.path}</span>
          <em class:rw={grant.mode === 'rw'}>
            {grant.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
          </em>
          <span class="count">{grant.token_count} {t($language, '个令牌')}</span>
        </div>
        <button type="button" class="danger" disabled={busy} onclick={() => void revokeGrant(grant.id)}>
          {t($language, '撤销')}
        </button>
      </div>
    {/each}
  </div>

  <div class="form">
    <label>
      <span>{t($language, '根目录')}</span>
      <select bind:value={rootId}>
        {#each roots as root (root.id)}
          <option value={root.id}>{root.alias}</option>
        {/each}
      </select>
    </label>
    <label>
      <span>{t($language, '路径')}</span>
      <input bind:value={grantPath} placeholder="/photos" />
    </label>
    <button type="button" disabled={busy || rootId === null} onclick={() => void openBrowser()}>
      {t($language, '浏览')}
    </button>
    <label>
      <span>{t($language, '名称')}</span>
      <input bind:value={label} />
    </label>
    <label>
      <span>{t($language, '权限')}</span>
      <select bind:value={mode}>
        <option value="ro">{t($language, '只读')}</option>
        <option value="rw">{t($language, '读写')}</option>
      </select>
    </label>
    <button type="button" disabled={busy || !label.trim()} onclick={() => void createGrant()}>
      {t($language, '授权')}
    </button>
  </div>

  <h3>{t($language, '访问令牌')}</h3>
  <div class="list">
    {#each tokens as token (token.id)}
      <div class="row">
        <div class="meta">
          <strong>{token.name}</strong>
          <code>{token.prefix}…</code>
          <span class="count">
            {token.grant_ids.length} {t($language, '个目录')} ·
            {t($language, '最近使用')} {formatDate(token.last_used_at)}
          </span>
        </div>
        <div class="row-actions">
          <button type="button" disabled={busy} onclick={() => void rotateToken(token.id)}>
            {t($language, '轮换')}
          </button>
          <button type="button" class="danger" disabled={busy} onclick={() => void revokeToken(token.id)}>
            {t($language, '撤销')}
          </button>
        </div>
      </div>
    {/each}
  </div>

  <div class="form">
    <label>
      <span>{t($language, '名称')}</span>
      <input bind:value={tokenName} />
    </label>
    <label>
      <span>{t($language, '有效期（天，0 为永久）')}</span>
      <input type="number" min="0" bind:value={expiresDays} />
    </label>
  </div>

  {#if grants.length > 0}
    <div class="grants">
      {#each grants as grant (grant.id)}
        <label class="grant">
          <input type="checkbox" bind:group={selectedGrants} value={grant.id} />
          <span>{grant.label}</span>
          <em class:rw={grant.mode === 'rw'}>
            {grant.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
          </em>
        </label>
      {/each}
    </div>
  {/if}

  <button
    type="button"
    disabled={busy || !tokenName.trim() || selectedGrants.length === 0}
    onclick={() => void createToken()}
  >
    {t($language, '新建令牌')}
  </button>

  {#if browseOpen}
    <div class="browser">
      <div class="browser-head">
        <code>/{browsePath}</code>
        <button type="button" disabled={browseBusy || !browsePath} onclick={browseUp}>
          {t($language, '上一级')}
        </button>
        <button type="button" disabled={browseBusy} onclick={chooseBrowse}>
          {t($language, '选择此目录')}
        </button>
        <button type="button" onclick={() => (browseOpen = false)}>
          {t($language, '取消')}
        </button>
      </div>
      <div class="browser-list">
        {#each browseDirs as name (name)}
          <button type="button" disabled={browseBusy} onclick={() => browseInto(name)}>
            {name}/
          </button>
        {/each}
        {#if browseDirs.length === 0}
          <span class="hint">{t($language, '没有子目录')}</span>
        {/if}
      </div>
    </div>
  {/if}

  {#if revokedGrants.length > 0}
    <details class="revoked">
      <summary>{t($language, '已撤销的授权')} ({revokedGrants.length})</summary>
      {#each revokedGrants as grant (grant.id)}
        <div class="row muted">
          <div class="meta">
            <strong>{grant.label}</strong>
            <span class="path" title={grant.path}>{grant.path}</span>
          </div>
        </div>
      {/each}
    </details>
  {/if}

  <h3>{t($language, '最近访问')}</h3>
  {#if activity.length === 0}
    <p class="hint">{t($language, '还没有其他实例访问过授权的目录。')}</p>
  {:else}
    <div class="activity">
      {#each activity as entry, index (index)}
        <div class="activity-row">
          <code>{entry.op}</code>
          <span class="path" title={entry.path ?? ''}>{entry.path ?? '—'}</span>
          <span class="count">{formatDate(entry.created_at)}</span>
        </div>
      {/each}
    </div>
  {/if}

  {#if newToken}
    <div class="token">
      <p>{t($language, '令牌只显示这一次')}</p>
      <code>{newToken}</code>
      <div class="token-actions">
        <button type="button" onclick={() => void copyToken()}>
          {copiedKind === 'token' ? t($language, '已复制') : t($language, '复制令牌')}
        </button>
        <button type="button" onclick={() => void copyInvite()}>
          {copiedKind === 'invite' ? t($language, '已复制') : t($language, '复制邀请信息')}
        </button>
      </div>
      <p class="hint">{t($language, '邀请信息包含地址和令牌，对方粘贴后可直接挂载。')}</p>
    </div>
  {/if}
</section>

<style>
  .pref-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  h3 {
    margin: 6px 0 0;
    font-size: 14px;
    font-weight: 600;
  }

  .hint,
  .error {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-2);
  }

  .error {
    color: var(--danger);
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .meta {
    display: flex;
    gap: 10px;
    align-items: center;
    min-width: 0;
    flex-wrap: wrap;
  }

  .meta .path {
    color: var(--text-2);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 320px;
  }

  .meta em,
  .grant em {
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .meta em.rw,
  .grant em.rw {
    color: var(--danger);
  }

  .count {
    font-size: 11.5px;
    color: var(--text-2);
  }

  .form {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: flex-end;
  }

  .form label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 160px;
    flex: 1;
    font-size: 12.5px;
    color: var(--text-2);
  }

  input,
  select {
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  button {
    padding: 7px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .danger {
    color: var(--danger);
  }

  .row-actions {
    display: flex;
    gap: 8px;
  }

  .browser {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
  }

  .browser-head {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .browser-head code {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .browser-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .revoked {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px 10px;
  }

  .revoked summary {
    cursor: pointer;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .row.muted {
    opacity: 0.65;
  }

  .activity {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .activity-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
  }

  .activity-row code {
    min-width: 64px;
    color: var(--text-2);
  }

  .activity-row .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .token-actions {
    display: flex;
    gap: 8px;
  }

  .grants {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .grant {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    font-size: 12.5px;
  }

  .token {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
  }

  .token p {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-2);
  }

  code {
    font-size: 12px;
    word-break: break-all;
  }
</style>
