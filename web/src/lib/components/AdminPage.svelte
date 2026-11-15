<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type AdminRoot, type ServerPath, type SettingsResponse, type User } from '../api';
  import { showNotice } from '../notices';
  import AssignPathDialog from './AssignPathDialog.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import ExternalPathDialog from './ExternalPathDialog.svelte';
  import Icon from './Icon.svelte';
  import NameDialog from './NameDialog.svelte';
  import NewUserDialog from './NewUserDialog.svelte';
  import QuotaDialog from './QuotaDialog.svelte';

  let { csrf, onAudit }: { csrf: string; onAudit: () => void } = $props();

  let users = $state<User[]>([]);
  let paths = $state<ServerPath[]>([]);
  let settings = $state<SettingsResponse | null>(null);
  let usage = $state<Record<number, number>>({});
  let roots = $state<Record<number, AdminRoot[]>>({});
  let loading = $state(true);
  let createOpen = $state(false);
  let pathOpen = $state(false);
  let quotaUser = $state<User | null>(null);
  let passwordUser = $state<User | null>(null);
  let assignUser = $state<User | null>(null);
  let deleteUser = $state<User | null>(null);
  let editingPathId = $state<number | null>(null);
  let editingPathAlias = $state('');

  function password(): string {
    const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%';
    return Array.from(
      crypto.getRandomValues(new Uint8Array(20)),
      (v) => alphabet[v % alphabet.length],
    ).join('');
  }

  async function load(): Promise<void> {
    loading = true;
    try {
      const [userResult, pathResult, settingsResult] = await Promise.all([
        api.listUsers(),
        api.listServerPaths(),
        api.getSettings(),
      ]);
      users = userResult.users;
      paths = pathResult.paths;
      settings = settingsResult;
      const usageEntries = await Promise.all(
        users.map(async (user) => [user.id, (await api.userUsage(user.id)).used_bytes] as const),
      );
      usage = Object.fromEntries(usageEntries);
      const rootEntries = await Promise.all(
        users.map(async (user) => [user.id, (await api.listUserRoots(user.id)).roots] as const),
      );
      roots = Object.fromEntries(rootEntries);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '管理数据加载失败');
    } finally {
      loading = false;
    }
  }

  onMount(() => void load());

  function formatBytes(value: number | null): string {
    if (value === null) return '不限制';
    if (value < 1024 ** 2) return `${(value / 1024).toFixed(0)} KB`;
    if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
    return `${(value / 1024 ** 3).toFixed(1)} GB`;
  }

  function formatDate(value: number | null): string {
    return value ? new Date(value * 1000).toLocaleString('zh-CN') : '从未登录';
  }

  async function createUser(
    username: string,
    initialPassword: string,
    quotaBytes: number,
  ): Promise<void> {
    await api.createUser(
      { username, password: initialPassword, role: 'user', quota_bytes: quotaBytes },
      csrf,
    );
    await load();
  }

  async function saveQuota(value: number | null): Promise<void> {
    if (!quotaUser) return;
    await api.updateUser(
      quotaUser.id,
      value === null ? { clear_quota: true } : { quota_bytes: value },
      csrf,
    );
    await load();
  }

  async function resetPassword(value: string): Promise<void> {
    if (!passwordUser) return;
    await api.updateUser(passwordUser.id, { password: value }, csrf);
    showNotice('密码已重置', 'info', '操作成功');
  }

  async function toggleRole(user: User): Promise<void> {
    const role = user.role === 'admin' ? 'user' : 'admin';
    try {
      await api.updateUser(user.id, { role }, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '角色切换失败');
    }
  }

  async function toggleDisabled(user: User): Promise<void> {
    try {
      await api.updateUser(user.id, { disabled: !user.disabled }, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '状态切换失败');
    }
  }

  async function confirmDelete(): Promise<void> {
    if (!deleteUser) return;
    try {
      await api.deleteUser(deleteUser.id, csrf);
      deleteUser = null;
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '删除用户失败');
    }
  }

  async function assignPath(path: ServerPath, readOnly: boolean): Promise<void> {
    if (!assignUser) return;
    await api.addRoot(
      assignUser.id,
      { path: path.real_path, alias: path.alias, read_only: readOnly },
      csrf,
    );
    await load();
  }

  async function addPath(path: string, alias: string): Promise<void> {
    await api.addServerPath({ path, alias }, csrf);
    await load();
  }

  function beginPathAlias(path: ServerPath): void {
    editingPathId = path.id;
    editingPathAlias = path.alias;
  }

  async function savePathAlias(): Promise<void> {
    if (editingPathId === null) return;
    try {
      await api.updateServerPath(editingPathId, editingPathAlias.trim(), csrf);
      editingPathId = null;
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '修改别名失败');
    }
  }

  async function deletePath(path: ServerPath): Promise<void> {
    try {
      await api.deleteServerPath(path.id, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '删除外部路径失败');
    }
  }

  async function toggleRoot(user: User, root: AdminRoot): Promise<void> {
    try {
      await api.updateRoot(
        user.id,
        root.id,
        { alias: root.alias, read_only: !root.read_only, sort_order: root.sort_order },
        csrf,
      );
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '权限修改失败');
    }
  }

  async function removeRoot(user: User, root: AdminRoot): Promise<void> {
    try {
      await api.deleteRoot(user.id, root.id, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '移除授权失败');
    }
  }

  function setting(key: string): unknown {
    let current: unknown = settings?.settings;
    for (const part of key.split('.')) {
      if (!current || typeof current !== 'object') return undefined;
      current = (current as Record<string, unknown>)[part];
    }
    return current;
  }

  async function setSetting(key: string, value: unknown): Promise<void> {
    try {
      settings = await api.updateSetting(key, value, csrf);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : '系统设置保存失败');
    }
  }

  let defaultQuotaGb = $state(15);
  let settingReady = $state(false);
  $effect(() => {
    if (settings && !settingReady) {
      defaultQuotaGb = Number(setting('default_quota_bytes') ?? 15 * 1024 ** 3) / 1024 ** 3;
      settingReady = true;
    }
  });

  async function saveDefaultQuota(): Promise<void> {
    await setSetting('default_quota_bytes', Math.round(defaultQuotaGb * 1024 ** 3));
  }
</script>

<section class="admin-page">
  <header class="admin-header">
    <div>
      <h1>系统管理</h1>
      <p>用户、外部路径与系统设置</p>
    </div>
    <button type="button" class="audit-button" onclick={onAudit}
      ><Icon name="terminal" size={17} />审计日志</button
    >
  </header>

  {#if loading}<p class="muted">加载中…</p>{:else}
    <section class="admin-section">
      <div class="section-heading">
        <h2>用户</h2>
        <button type="button" class="add-button" onclick={() => (createOpen = true)}
          >+ 新建用户</button
        >
      </div>
      {#each users as user (user.id)}
        <article class="user-card">
          <div class="user-line">
            <strong>{user.username}</strong>
            <button type="button" class="pill-button" onclick={() => void toggleRole(user)}
              >{user.role === 'admin' ? '管理员' : '成员'}</button
            >
            <button type="button" class="quota-button" onclick={() => (quotaUser = user)}
              >配额 {formatBytes(user.quota_bytes)}</button
            >
            <span class="meta">已用 {formatBytes(usage[user.id] ?? 0)}</span>
            <button type="button" class="text-button" onclick={() => (passwordUser = user)}
              >重置密码</button
            >
            <button type="button" class="text-button" onclick={() => void toggleDisabled(user)}
              >{user.disabled ? '启用' : '禁用'}</button
            >
            <button type="button" class="text-button danger" onclick={() => (deleteUser = user)}
              >删除</button
            >
          </div>
          <div class="user-meta">
            <span>创建 {formatDate(user.created_at)}</span>
            <span>最后登录 {formatDate(user.last_login_at)}</span>
            {#if user.disabled}<span class="danger-text">已禁用</span>{/if}
          </div>
          <div class="roots">
            {#each roots[user.id] ?? [] as root (root.id)}
              <div class="root-row">
                <span class="root-alias">{root.alias}</span>
                <span class="root-path">{root.real_path}</span>
                <button type="button" class="permission" onclick={() => void toggleRoot(user, root)}
                  >{root.read_only ? '只读' : '读写'}</button
                >
                <button
                  type="button"
                  class="text-button danger"
                  onclick={() => void removeRoot(user, root)}>删除</button
                >
              </div>
            {/each}
            <button type="button" class="assign-button" onclick={() => (assignUser = user)}
              >+ 分配外部路径</button
            >
          </div>
        </article>
      {/each}
    </section>

    <section class="admin-section">
      <div class="section-heading">
        <h2>外部路径</h2>
        <button type="button" class="add-button" onclick={() => (pathOpen = true)}
          >+ 添加外部路径</button
        >
      </div>
      {#each paths as path (path.id)}
        <div class="path-card">
          {#if editingPathId === path.id}
            <form
              class="path-alias-form"
              onsubmit={(event) => {
                event.preventDefault();
                void savePathAlias();
              }}
            >
              <input
                bind:value={editingPathAlias}
                aria-label="外部路径别名"
                onkeydown={(event) => {
                  if (event.key === 'Escape') editingPathId = null;
                }}
              />
              <button type="submit">保存</button>
            </form>
          {:else}
            <button type="button" class="path-alias" onclick={() => beginPathAlias(path)}
              >{path.alias}</button
            >
          {/if}
          <span class="root-path">{path.real_path}</span>
          <button type="button" class="text-button danger" onclick={() => void deletePath(path)}
            >删除</button
          >
        </div>
      {:else}
        <p class="muted">还没有外部路径。</p>
      {/each}
    </section>

    <section class="admin-section">
      <h2>系统设置</h2>
      <div class="setting-grid">
        <label class="setting-row"
          ><span>缩略图</span><input
            type="checkbox"
            checked={Boolean(setting('thumbnails.enabled'))}
            onchange={(event) => void setSetting('thumbnails.enabled', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>文本编辑器</span><input
            type="checkbox"
            checked={Boolean(setting('editor.enabled'))}
            onchange={(event) => void setSetting('editor.enabled', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>终端</span><input
            type="checkbox"
            checked={Boolean(setting('terminal_enabled'))}
            onchange={(event) => void setSetting('terminal_enabled', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>默认显示隐藏文件</span><input
            type="checkbox"
            checked={Boolean(setting('show_hidden_default'))}
            onchange={(event) =>
              void setSetting('show_hidden_default', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>默认主题</span><select
            value={String(setting('ui.default_theme') ?? 'system')}
            onchange={(event) => void setSetting('ui.default_theme', event.currentTarget.value)}
            ><option value="system">跟随系统</option><option value="light">浅色</option><option
              value="dark">深色</option
            ></select
          ></label
        >
        <label class="setting-row"
          ><span>默认语言</span><select
            value={String(setting('ui.default_locale') ?? 'zh-CN')}
            onchange={(event) => void setSetting('ui.default_locale', event.currentTarget.value)}
            ><option value="zh-CN">中文</option><option value="en">English</option></select
          ></label
        >
        <label class="setting-row"
          ><span>上传同名文件</span><select
            value={String(setting('upload_conflict_default') ?? 'rename')}
            onchange={(event) =>
              void setSetting('upload_conflict_default', event.currentTarget.value)}
            ><option value="rename">自动重命名</option><option value="overwrite">覆盖</option
            ><option value="skip">跳过</option></select
          ></label
        >
        <label class="setting-row"
          ><span>默认配额</span>
          <div class="quota-setting">
            <input bind:value={defaultQuotaGb} type="number" min="0" step="1" /><span>GB</span
            ><button type="button" class="small" onclick={() => void saveDefaultQuota()}
              >保存</button
            >
          </div></label
        >
      </div>
    </section>
  {/if}
</section>

{#if createOpen}
  <NewUserDialog
    defaultQuotaBytes={Number(setting('default_quota_bytes') ?? 15 * 1024 ** 3)}
    onClose={() => (createOpen = false)}
    onSave={createUser}
  />
{/if}
{#if pathOpen}<ExternalPathDialog onClose={() => (pathOpen = false)} onSave={addPath} />{/if}
{#if quotaUser}<QuotaDialog
    value={quotaUser.quota_bytes}
    onClose={() => (quotaUser = null)}
    onSave={saveQuota}
  />{/if}
{#if passwordUser}<NameDialog
    open
    title="重置密码"
    label={`${passwordUser.username} 的新密码`}
    initialValue={password()}
    onConfirm={(value) => void resetPassword(value)}
    onClose={() => (passwordUser = null)}
  />{/if}
{#if assignUser}<AssignPathDialog
    {paths}
    assigned={(roots[assignUser.id] ?? []).map((root) => root.real_path)}
    onClose={() => (assignUser = null)}
    onAssign={assignPath}
  />{/if}
{#if deleteUser}<ConfirmDialog
    open
    title="删除用户"
    message={`删除用户「${deleteUser.username}」？默认保留其 home 文件。`}
    confirmLabel="删除"
    danger
    onConfirm={() => void confirmDelete()}
    onCancel={() => (deleteUser = null)}
  />{/if}

<style>
  .admin-page {
    display: flex;
    width: min(100%, 1100px);
    margin: 0 auto;
    flex-direction: column;
    gap: 24px;
  }
  .admin-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 24px;
    font-weight: 500;
  }
  .admin-header p {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
  }
  .audit-button,
  .add-button {
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }
  .audit-button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .admin-section {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .section-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .user-card,
  .path-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .user-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
  }
  .user-line strong {
    font-size: 16px;
  }
  .meta,
  .user-meta,
  .muted {
    color: var(--text-2);
    font-size: 12.5px;
  }
  .user-meta {
    display: flex;
    gap: 16px;
  }
  .pill-button,
  .quota-button,
  .permission {
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text);
    font-size: 12px;
  }
  .quota-button {
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .text-button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 12.5px;
  }
  .text-button:hover {
    color: var(--accent-text);
  }
  .text-button.danger {
    color: var(--danger);
  }
  .roots {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
  .root-row,
  .path-card {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .root-alias {
    width: 90px;
    font-weight: 400;
  }
  .root-path {
    min-width: 0;
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .root-row .root-path {
    flex: 1;
  }
  .permission {
    flex: 0 0 auto;
  }
  .assign-button {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: 12.5px;
  }
  .path-card {
    flex-direction: row;
  }
  .path-alias {
    width: 100px;
    overflow: hidden;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-weight: 400;
    text-align: left;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .path-alias:hover {
    color: var(--accent-text);
  }

  .path-alias-form {
    display: flex;
    width: 230px;
    gap: 6px;
  }

  .path-alias-form input {
    min-width: 0;
    flex: 1;
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  .path-alias-form button {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .path-card .root-path {
    flex: 1;
  }
  .setting-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .setting-row {
    display: flex;
    min-height: 54px;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding: 8px 16px;
    border-left: 1px solid var(--border);
    color: var(--text);
    font-size: 13.5px;
  }
  .setting-row:nth-child(3n + 1) {
    border-left: 0;
  }
  @media (max-width: 800px) {
    .setting-grid {
      grid-template-columns: 1fr;
    }
    .setting-row,
    .setting-row:nth-child(3n + 1) {
      border-top: 1px solid var(--border);
      border-left: 0;
    }
    .setting-row:first-child {
      border-top: 0;
    }
  }
  .setting-row select {
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .setting-row input[type='checkbox'] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  .quota-setting {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .quota-setting input {
    width: 76px;
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .small {
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }
  .danger-text {
    color: var(--danger);
  }
</style>
