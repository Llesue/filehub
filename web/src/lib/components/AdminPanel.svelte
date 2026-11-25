<script lang="ts">
  import { onMount } from 'svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import Icon from './Icon.svelte';
  import { language, localeOf, t } from '../i18n';
  import { showNotice } from '../notices';
  import NameDialog from './NameDialog.svelte';
  import {
    api,
    type AdminRoot,
    type AuditEntry,
    type ServerPath,
    type SettingsResponse,
    type User,
  } from '../api';

  let { csrf }: { csrf: string } = $props();

  let tab = $state<'users' | 'paths' | 'audit' | 'settings'>('users');
  let users = $state<User[]>([]);
  let roots = $state<AdminRoot[]>([]);
  let selectedUserId = $state<number | null>(null);
  let settings = $state<SettingsResponse | null>(null);
  let busy = $state(false);

  let newUsername = $state('');
  let newPassword = $state('');
  let newQuota = $state('');
  let rootPath = $state('');
  let rootAlias = $state('');
  let rootReadOnly = $state(false);
  let quotaDrafts = $state<Record<number, string>>({});
  let serverPaths = $state<ServerPath[]>([]);
  let audit = $state<AuditEntry[]>([]);
  let serverPathAlias = $state('');
  let serverPathReal = $state('');
  let confirm = $state<{ title: string; message: string; action: () => void } | null>(null);
  let passwordTarget = $state<User | null>(null);

  onMount(() => {
    void loadAll();
  });

  async function loadAll(): Promise<void> {
    try {
      const [userResult, settingsResult, pathResult, auditResult] = await Promise.all([
        api.listUsers(),
        api.getSettings(),
        api.listServerPaths(),
        api.listAudit(200),
      ]);
      users = userResult.users;
      settings = settingsResult;
      serverPaths = pathResult.paths;
      audit = auditResult.entries;
      for (const user of users) {
        quotaDrafts[user.id] = user.quota_bytes?.toString() ?? '';
      }
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function selectUser(user: User): Promise<void> {
    selectedUserId = user.id;
    try {
      roots = (await api.listUserRoots(user.id)).roots;
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function createUser(): Promise<void> {
    busy = true;
    try {
      const quota = newQuota.trim() === '' ? null : Number(newQuota);
      await api.createUser(
        {
          username: newUsername.trim(),
          password: newPassword,
          role: 'user',
          quota_bytes: quota,
        },
        csrf,
      );
      newUsername = '';
      newPassword = '';
      newQuota = '';
      await loadAll();
    } catch (err) {
      showNotice(messageOf(err));
    } finally {
      busy = false;
    }
  }

  async function saveQuota(user: User): Promise<void> {
    const raw = quotaDrafts[user.id] ?? '';
    const quota = Number(raw);
    if (raw.trim() !== '' && (!Number.isFinite(quota) || quota < 0)) {
      showNotice(t($language, '配额必须是非负数字'), 'warning', t($language, '输入错误'));
      return;
    }
    try {
      await api.updateUser(
        user.id,
        raw.trim() === '' ? { clear_quota: true } : { quota_bytes: quota },
        csrf,
      );
      await loadAll();
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function setRole(user: User, role: string): Promise<void> {
    try {
      await api.updateUser(user.id, { role }, csrf);
      await loadAll();
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function resetPassword(value: string): Promise<void> {
    const target = passwordTarget;
    passwordTarget = null;
    if (!target) return;
    try {
      await api.updateUser(target.id, { password: value }, csrf);
      showNotice(t($language, '临时密码已设置，用户下次登录必须修改'), 'info', t($language, '操作成功'));
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function toggleDisabled(user: User): Promise<void> {
    try {
      await api.updateUser(user.id, { disabled: !user.disabled }, csrf);
      await loadAll();
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function addRoot(): Promise<void> {
    if (selectedUserId === null) return;
    try {
      await api.addRoot(
        selectedUserId,
        { path: rootPath.trim(), alias: rootAlias.trim(), read_only: rootReadOnly },
        csrf,
      );
      rootPath = '';
      rootAlias = '';
      rootReadOnly = false;
      roots = (await api.listUserRoots(selectedUserId)).roots;
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  function askDeleteRoot(root: AdminRoot): void {
    if (selectedUserId === null) return;
    const userId = selectedUserId;
    confirm = {
      title: t($language, '移除存储目录'),
      message: t($language, '删除「{alias}」？用户将无法再访问该目录。', { alias: root.alias }),
      action: () => {
        void (async () => {
          try {
            await api.deleteRoot(userId, root.id, csrf);
            roots = (await api.listUserRoots(userId)).roots;
          } catch (err) {
            showNotice(messageOf(err));
          }
        })();
      },
    };
  }

  async function addServerPath(): Promise<void> {
    try {
      await api.addServerPath({ path: serverPathReal.trim(), alias: serverPathAlias.trim() }, csrf);
      serverPathReal = '';
      serverPathAlias = '';
      serverPaths = (await api.listServerPaths()).paths;
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  function askDeleteServerPath(path: ServerPath): void {
    confirm = {
      title: t($language, '删除存储目录'),
      message: t($language, '删除「{alias}」？必须先移除所有用户授权。', { alias: path.alias }),
      action: () => {
        void (async () => {
          try {
            await api.deleteServerPath(path.id, csrf);
            serverPaths = (await api.listServerPaths()).paths;
          } catch (err) {
            showNotice(messageOf(err));
          }
        })();
      },
    };
  }

  function settingValue(key: string): unknown {
    let current: unknown = settings?.settings;
    for (const part of key.split('.')) {
      if (!current || typeof current !== 'object') return undefined;
      current = (current as Record<string, unknown>)[part];
    }
    return current;
  }

  async function toggleSetting(key: string, value: boolean): Promise<void> {
    try {
      const result = await api.updateSetting(key, !value, csrf);
      settings = result;
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  async function resetSetting(key: string): Promise<void> {
    try {
      settings = await api.resetSetting(key, csrf);
    } catch (err) {
      showNotice(messageOf(err));
    }
  }

  function messageOf(err: unknown): string {
    return err instanceof Error ? err.message : t($language, '操作失败');
  }
</script>

<section class="admin-page" aria-labelledby="admin-title">
  <header class="admin-header">
    <div>
      <h1 id="admin-title">{t($language, '管理')}</h1>
      <span>{t($language, '用户、存储目录、审计与系统设置')}</span>
    </div>
    <Icon name="shield" size={24} />
  </header>

  <nav class="tabs" aria-label={t($language, '管理分类')}>
    <button type="button" class:active={tab === 'users'} onclick={() => (tab = 'users')}
      >{t($language, '用户')}</button
    >
    <button type="button" class:active={tab === 'paths'} onclick={() => (tab = 'paths')}
      >{t($language, '存储目录')}</button
    >
    <button type="button" class:active={tab === 'audit'} onclick={() => (tab = 'audit')}
      >{t($language, '审计日志')}</button
    >
    <button type="button" class:active={tab === 'settings'} onclick={() => (tab = 'settings')}
      >{t($language, '系统设置')}</button
    >
  </nav>

  <div class="admin-body">
    {#if tab === 'users'}
      <section class="admin-block">
        <h2>{t($language, '新建用户')}</h2>
        <div class="form-row">
          <input bind:value={newUsername} placeholder={t($language, '用户名')} />
          <input
            bind:value={newPassword}
            type="password"
            placeholder={t($language, '临时密码（至少 6 位）')}
          />
          <input bind:value={newQuota} placeholder={t($language, '配额字节，空=不限制')} />
          <button type="button" disabled={busy} onclick={() => void createUser()}
            >{t($language, '创建')}</button
          >
        </div>
      </section>

      <section class="admin-block">
        <h2>{t($language, '用户列表')}</h2>
        <div class="user-list">
          {#each users as user (user.id)}
            <div class="user-row">
              <button type="button" class="user-name" onclick={() => void selectUser(user)}>
                {user.username}
              </button>
              <select
                class="role-select"
                value={user.role}
                aria-label={t($language, '{username} 角色', { username: user.username })}
                onchange={(event) =>
                  void setRole(user, (event.currentTarget as HTMLSelectElement).value)}
              >
                <option value="user">{t($language, '用户')}</option>
                <option value="admin">{t($language, '管理员')}</option>
              </select>
              <span class:disabled={user.disabled} class="status"
                >{user.disabled ? t($language, '已禁用') : t($language, '正常')}</span
              >
              <button type="button" class="small" onclick={() => (passwordTarget = user)}
                >{t($language, '重置密码')}</button
              >
              <input
                class="quota-input"
                bind:value={quotaDrafts[user.id]}
                placeholder={t($language, '不限制')}
                aria-label={t($language, '{username} 配额', { username: user.username })}
              />
              <button type="button" class="small" onclick={() => void saveQuota(user)}
                >{t($language, '保存配额')}</button
              >
              <button type="button" class="small" onclick={() => void toggleDisabled(user)}>
                {user.disabled ? t($language, '启用') : t($language, '禁用')}
              </button>
            </div>
          {/each}
        </div>
      </section>

      {#if selectedUserId !== null}
        <section class="admin-block">
          <h2>{t($language, '分配存储目录')}</h2>
          <div class="form-row">
            <select bind:value={rootPath}>
              <option value="">{t($language, '选择存储目录')}</option>
              {#each serverPaths as path (path.id)}
                <option value={path.real_path}>{path.alias} — {path.real_path}</option>
              {/each}
            </select>
            <input bind:value={rootAlias} placeholder={t($language, '用户显示名')} />
            <select bind:value={rootReadOnly} aria-label={t($language, '权限')}>
              <option value={false}>{t($language, '读写')}</option>
              <option value={true}>{t($language, '只读')}</option>
            </select>
            <button type="button" onclick={() => void addRoot()}>{t($language, '添加')}</button>
          </div>
          <div class="user-list">
            {#each roots as root (root.id)}
              <div class="user-row">
                <span class="mono">{root.alias}</span>
                <span class="mono path">{root.real_path}</span>
                <span class="pill"
                  >{root.read_only ? t($language, '只读') : t($language, '读写')}</span
                >
                <button type="button" class="small danger" onclick={() => askDeleteRoot(root)}
                  >{t($language, '删除')}</button
                >
              </div>
            {/each}
          </div>
        </section>
      {/if}
    {:else if tab === 'paths'}
      <section class="admin-block">
        <h2>{t($language, '存储目录')}</h2>
        <p class="muted">
          {t(
            $language,
            '先登记服务器上已经存在且可读的存储目录（例如 /mnt/red → 红盘），再到用户页分配。目录不存在时请先在服务器上创建或挂载。',
          )}
        </p>
        <div class="form-row">
          <input
            bind:value={serverPathReal}
            placeholder={t($language, '目录路径，例如 /mnt/red')}
          />
          <input bind:value={serverPathAlias} placeholder={t($language, '别名，例如 红盘')} />
          <button type="button" onclick={() => void addServerPath()}>{t($language, '添加')}</button>
        </div>
        <div class="user-list">
          {#each serverPaths as path (path.id)}
            <div class="user-row path-row">
              <span class="mono">{path.alias}</span>
              <span class="mono path">{path.real_path}</span>
              <button type="button" class="small danger" onclick={() => askDeleteServerPath(path)}
                >{t($language, '删除')}</button
              >
            </div>
          {/each}
        </div>
      </section>
    {:else if tab === 'audit'}
      <section class="admin-block">
        <h2>{t($language, '最近操作')}</h2>
        <div class="audit-list">
          {#each audit as entry (entry.id)}
            <div class="audit-row">
              <span class="audit-time"
                >{new Date(entry.created_at * 1000).toLocaleString(localeOf($language))}</span
              >
              <span class="audit-user">{entry.username ?? '—'}</span>
              <span class="pill">{entry.action}</span>
              <span class="mono audit-path">{entry.path ?? ''}</span>
              <span class="audit-detail">{entry.detail ?? ''}</span>
            </div>
          {/each}
        </div>
        {#if audit.length === 0}<p class="muted">{t($language, '暂无审计记录。')}</p>{/if}
      </section>
    {:else if tab === 'settings'}
      <section class="admin-block">
        <h2>{t($language, '开关')}</h2>
        {#each ['thumbnails.enabled', 'editor.enabled'] as key (key)}
          <div class="setting-row">
            <span class="mono">{key}</span>
            <span class="pill">{settings?.sources[key] ?? 'default'}</span>
            <button
              type="button"
              class:on={Boolean(settingValue(key))}
              class="toggle"
              onclick={() => void toggleSetting(key, Boolean(settingValue(key)))}
            >
              {settingValue(key) ? t($language, '已开启') : t($language, '已关闭')}
            </button>
            <button type="button" class="small" onclick={() => void resetSetting(key)}
              >{t($language, '清除覆盖')}</button
            >
          </div>
        {/each}
      </section>
      <section class="admin-block">
        <h2>{t($language, '全部生效配置')}</h2>
        <pre>{JSON.stringify(settings?.settings ?? {}, null, 2)}</pre>
      </section>
    {/if}
  </div>
  {#if passwordTarget}
    <NameDialog
      open
      title={t($language, '重置密码')}
      label={t($language, '{username} 的新密码', { username: passwordTarget.username })}
      inputType="password"
      confirmLabel={t($language, '更新')}
      onConfirm={(value) => void resetPassword(value)}
      onClose={() => (passwordTarget = null)}
    />
  {/if}

  <ConfirmDialog
    open={confirm !== null}
    title={confirm?.title ?? ''}
    message={confirm?.message ?? ''}
    confirmLabel={t($language, '删除')}
    danger
    onConfirm={() => {
      confirm?.action();
      confirm = null;
    }}
    onCancel={() => (confirm = null)}
  />
</section>

<style>
  .admin-page {
    display: flex;
    width: min(100%, 1000px);
    margin: 0 auto;
    flex-direction: column;
    color: var(--text);
  }

  .admin-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 18px;
  }

  .admin-header h1 {
    margin: 0 0 4px;
    font-size: 24px;
    font-weight: 500;
  }

  .admin-header span {
    color: var(--text-2);
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

  .admin-body {
    overflow: visible;
  }

  .admin-block {
    margin-bottom: 24px;
  }

  h2 {
    margin: 0 0 10px;
    color: var(--text);
    font-size: 16px;
    font-weight: 500;
  }

  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  input,
  select {
    height: 34px;
    min-width: 140px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font-size: 13.5px;
  }

  button {
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font-size: 13.5px;
  }

  button:disabled {
    opacity: 0.6;
  }

  .audit-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .audit-row {
    display: grid;
    grid-template-columns: 150px 90px 110px minmax(120px, 1fr) minmax(100px, 1fr);
    gap: 10px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
  }

  .audit-time,
  .audit-detail {
    color: var(--text-2);
  }

  .audit-user {
    font-weight: 500;
  }

  .audit-path {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .user-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 10px;
  }

  .user-row {
    display: grid;
    grid-template-columns: minmax(120px, 1fr) 70px 70px minmax(120px, 180px) auto auto;
    align-items: center;
    gap: 8px;
    min-height: 40px;
    padding: 0 8px;
    border-top: 1px solid var(--border);
    font-size: 13.5px;
  }

  .user-name {
    height: auto;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font-weight: 500;
    text-align: left;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: fit-content;
    min-height: 20px;
    padding: 0 7px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 12px;
  }

  .status {
    color: var(--text-2);
  }

  .status.disabled {
    color: var(--danger);
  }

  .role-select {
    height: 32px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
  }

  .quota-input {
    min-width: 100px;
    height: 30px;
  }

  .small {
    height: 28px;
    padding: 0 8px;
    font-size: 12.5px;
  }

  .danger {
    color: var(--danger);
  }

  .mono {
    overflow: hidden;
    font-family: var(--font-mono);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .path {
    color: var(--text-2);
  }

  .setting-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 40px;
    border-top: 1px solid var(--border);
    font-size: 13.5px;
  }

  .toggle {
    margin-left: auto;
  }

  .toggle.on {
    border-color: var(--accent);
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  pre {
    overflow: auto;
    max-height: 260px;
    padding: 12px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12.5px;
  }

  @media (max-width: 720px) {
    .admin-page {
      width: 100%;
    }

    .user-row {
      grid-template-columns: minmax(100px, 1fr) 60px auto;
      padding: 8px;
    }

    .quota-input,
    .user-row .small {
      grid-column: span 1;
    }
  }
</style>
