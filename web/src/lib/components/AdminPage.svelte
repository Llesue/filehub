<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { api, type AdminRoot, type ServerPath, type SettingsResponse, type User } from '../api';
  import { branding, brandMark, loadBranding } from '../branding';
  import { language, localeOf, t } from '../i18n';
  import { showNotice } from '../notices';
  import AssignPathDialog from './AssignPathDialog.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import ExternalPathDialog from './ExternalPathDialog.svelte';
  import Icon from './Icon.svelte';
  import NameDialog from './NameDialog.svelte';
  import NewUserDialog from './NewUserDialog.svelte';
  import QuotaDialog from './QuotaDialog.svelte';

  let {
    csrf,
    onAudit,
    onRootsChanged,
  }: { csrf: string; onAudit: () => void; onRootsChanged?: () => void } = $props();

  let users = $state<User[]>([]);
  let paths = $state<ServerPath[]>([]);
  let settings = $state<SettingsResponse | null>(null);
  let usage = $state<Record<number, number | null>>({});
  let usageComputing = $state(false);
  let usageTimer: ReturnType<typeof setTimeout> | null = null;
  let status = $state<Awaited<ReturnType<typeof api.adminStatus>> | null>(null);
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
  let editingPathOriginal = $state('');

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
      const [usageResult, rootEntries, statusResult] = await Promise.all([
        api.adminUsage(),
        Promise.all(
          users.map(async (user) => [user.id, (await api.listUserRoots(user.id)).roots] as const),
        ),
        api.adminStatus(),
      ]);
      applyUsage(usageResult);
      roots = Object.fromEntries(rootEntries);
      status = statusResult;
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '管理数据加载失败'));
    } finally {
      loading = false;
      scheduleUsagePoll();
    }
  }

  function applyUsage(result: Awaited<ReturnType<typeof api.adminUsage>>): void {
    usage = Object.fromEntries(
      users.map((user) => [user.id, result.users[user.id]?.used_bytes ?? null] as const),
    );
    usageComputing = users.some((user) => result.users[user.id]?.computing ?? false);
  }

  function scheduleUsagePoll(): void {
    if (usageTimer) {
      clearTimeout(usageTimer);
      usageTimer = null;
    }
    if (!usageComputing) return;
    usageTimer = setTimeout(() => void refreshUsage(), 2000);
  }

  async function refreshUsage(): Promise<void> {
    try {
      applyUsage(await api.adminUsage());
    } catch (error) {
      console.warn('admin usage refresh failed', error);
      usageComputing = false;
    }
    scheduleUsagePoll();
  }

  onMount(() => void load());
  onDestroy(() => {
    if (usageTimer) clearTimeout(usageTimer);
  });

  function formatBytes(value: number | null): string {
    if (value === null) return t($language, '不限制');
    if (value < 1024 ** 2) return `${(value / 1024).toFixed(0)} KB`;
    if (value < 1024 ** 3) return `${(value / 1024 ** 2).toFixed(1)} MB`;
    return `${(value / 1024 ** 3).toFixed(1)} GB`;
  }

  function usedLabel(userId: number): string {
    const value = usage[userId];
    return value === null || value === undefined ? t($language, '统计中…') : formatBytes(value);
  }

  function formatDate(value: number | null): string {
    return value
      ? new Date(value * 1000).toLocaleString(localeOf($language))
      : t($language, '从未登录');
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
    showNotice(t($language, '密码已重置'), 'info', t($language, '操作成功'));
  }

  async function toggleRole(user: User): Promise<void> {
    const role = user.role === 'admin' ? 'user' : 'admin';
    try {
      await api.updateUser(user.id, { role }, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '角色切换失败'));
    }
  }

  async function toggleDisabled(user: User): Promise<void> {
    try {
      await api.updateUser(user.id, { disabled: !user.disabled }, csrf);
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '状态切换失败'));
    }
  }

  async function confirmDelete(): Promise<void> {
    if (!deleteUser) return;
    try {
      await api.deleteUser(deleteUser.id, csrf);
      deleteUser = null;
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '删除用户失败'));
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
    onRootsChanged?.();
  }

  async function addPath(path: string, alias: string): Promise<void> {
    await api.addServerPath({ path, alias }, csrf);
    await load();
    onRootsChanged?.();
  }

  function beginPathAlias(path: ServerPath): void {
    editingPathId = path.id;
    editingPathAlias = path.alias;
    editingPathOriginal = path.alias;
  }

  async function finishPathAlias(): Promise<void> {
    if (editingPathId === null) return;
    const value = editingPathAlias.trim();
    if (!value || value === editingPathOriginal) {
      editingPathId = null;
      editingPathAlias = editingPathOriginal;
      return;
    }
    try {
      await api.updateServerPath(editingPathId, value, csrf);
      editingPathId = null;
      await load();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '修改别名失败'));
      editingPathId = null;
      editingPathAlias = editingPathOriginal;
    }
  }

  async function deletePath(path: ServerPath): Promise<void> {
    try {
      await api.deleteServerPath(path.id, csrf);
      await load();
      onRootsChanged?.();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '删除外部路径失败'));
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
      onRootsChanged?.();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '权限修改失败'));
    }
  }

  async function removeRoot(user: User, root: AdminRoot): Promise<void> {
    try {
      await api.deleteRoot(user.id, root.id, csrf);
      await load();
      onRootsChanged?.();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '移除授权失败'));
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
      showNotice(error instanceof Error ? error.message : t($language, '系统设置保存失败'));
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

  let brandNameInput = $state('FileHub');
  let brandReady = $state(false);
  let brandBusy = $state(false);
  $effect(() => {
    if (settings && !brandReady) {
      brandNameInput = String(setting('ui.brand_name') ?? 'FileHub');
      brandReady = true;
    }
  });

  async function saveBrandName(): Promise<void> {
    const value = brandNameInput.trim();
    if (!value) return;
    await setSetting('ui.brand_name', value);
    await loadBranding();
    showNotice(t($language, '已保存'), 'info', t($language, '品牌'));
  }

  async function uploadBrandIcon(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    if (file.size > 2 * 1024 * 1024) {
      showNotice(t($language, '图标不能超过 2MB'));
      return;
    }
    brandBusy = true;
    try {
      await api.uploadBrandingIcon(file, csrf);
      await loadBranding();
      showNotice(t($language, '图标已更新'), 'info', t($language, '品牌'));
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '上传失败'));
    } finally {
      brandBusy = false;
    }
  }

  async function resetBrandIcon(): Promise<void> {
    brandBusy = true;
    try {
      await api.deleteBrandingIcon(csrf);
      await loadBranding();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '操作失败'));
    } finally {
      brandBusy = false;
    }
  }

  let zipMaxGb = $state(2);
  let zipMaxFiles = $state(1000);
  let zipConcurrency = $state(1);
  let zipSettingsReady = $state(false);
  $effect(() => {
    if (settings && !zipSettingsReady) {
      zipMaxGb = Number(setting('downloads.zip_max_bytes') ?? 2 * 1024 ** 3) / 1024 ** 3;
      zipMaxFiles = Number(setting('downloads.zip_max_files') ?? 1000);
      zipConcurrency = Number(setting('downloads.zip_concurrency') ?? 1);
      zipSettingsReady = true;
    }
  });

  async function saveZipSettings(): Promise<void> {
    await setSetting('downloads.zip_max_bytes', Math.round(zipMaxGb * 1024 ** 3));
    await setSetting('downloads.zip_max_files', Math.round(zipMaxFiles));
    await setSetting('downloads.zip_concurrency', Math.max(1, Math.round(zipConcurrency)));
  }
</script>

<section class="admin-page">
  <header class="admin-header">
    <div>
      <h1>{t($language, '系统管理')}</h1>
      <p>{t($language, '用户、外部路径与系统设置')}</p>
    </div>
    <button type="button" class="audit-button" onclick={onAudit}
      ><Icon name="terminal" size={17} />{t($language, '审计日志')}</button
    >
  </header>

  {#if loading}<p class="muted">{t($language, '加载中…')}</p>{:else}
    {#if status}
      <section class="admin-section">
        <div class="section-heading">
          <h2>{t($language, '系统状态')}</h2>
        </div>
        <div class="status-grid">
          <span
            >{t($language, '后台任务')}：{status.jobs.length > 0
              ? status.jobs.map((job) => `${job.kind} ${job.queued}/${job.running}`).join(' · ')
              : '—'}</span
          >
          <span
            >{t($language, '目录大小缓存')}：{status.directory_sizes.length > 0
              ? status.directory_sizes.map((entry) => `${entry.status} ${entry.count}`).join(' · ')
              : '—'}</span
          >
          <span
            >{t($language, '数据库')}：{formatBytes(
              status.database.page_count * status.database.page_size,
            )} · WAL {formatBytes(status.database.wal_bytes)}</span
          >
          <span>
            {t($language, '索引维护')}：{status.index.scheduled
              ? t($language, '快速 {quick} 分钟 / 全量 {full} 小时', {
                  quick: status.index.quick_scan_minutes,
                  full: status.index.full_scan_hours,
                })
              : t($language, '已关闭')}
            {status.index.last_quick_scan
              ? ` · ${t($language, '上次快速扫描')} ${formatDate(status.index.last_quick_scan)}`
              : ''}
          </span>
        </div>
        {#if status.slow_requests.length > 0}
          <details class="slow-requests">
            <summary>{t($language, '最近慢请求')}（{status.slow_requests.length}）</summary>
            <p class="slow-hint">{t($language, '仅记录耗时 ≥1 秒的请求，最多保留 100 条。')}</p>
            {#each status.slow_requests.slice(0, 10) as request, index (index)}
              <div class="status-row">
                <span>{request.method} {request.path}</span>
                <span>{request.status} · {request.duration_ms} ms</span>
              </div>
            {/each}
          </details>
        {/if}
      </section>
    {/if}
    <section class="admin-section">
      <div class="section-heading">
        <h2>{t($language, '用户')}</h2>
        <button type="button" class="add-button" onclick={() => (createOpen = true)}
          >{t($language, '+ 新建用户')}</button
        >
      </div>
      {#each users as user (user.id)}
        <article class="user-card">
          <div class="user-line">
            <strong>{user.username}</strong>
            <button type="button" class="pill-button" onclick={() => void toggleRole(user)}
              >{user.role === 'admin' ? t($language, '管理员') : t($language, '成员')}</button
            >
            <button type="button" class="quota-button" onclick={() => (quotaUser = user)}
              >{t($language, '配额')} {formatBytes(user.quota_bytes)}</button
            >
            <span class="meta">{t($language, '已用')} {usedLabel(user.id)}</span>
            <button type="button" class="text-button" onclick={() => (passwordUser = user)}
              >{t($language, '重置密码')}</button
            >
            <button type="button" class="text-button" onclick={() => void toggleDisabled(user)}
              >{user.disabled ? t($language, '启用') : t($language, '禁用')}</button
            >
            <button type="button" class="text-button danger" onclick={() => (deleteUser = user)}
              >{t($language, '删除')}</button
            >
          </div>
          <div class="user-meta">
            <span>{t($language, '创建')} {formatDate(user.created_at)}</span>
            <span>{t($language, '最后登录')} {formatDate(user.last_login_at)}</span>
            {#if user.disabled}<span class="danger-text">{t($language, '已禁用')}</span>{/if}
          </div>
          <div class="roots">
            {#each roots[user.id] ?? [] as root (root.id)}
              <div class="root-row">
                <span class="root-alias">{root.alias}</span>
                <span class="root-path">{root.real_path}</span>
                {#if root.covered_by}
                  <span class="tag"
                    >{t($language, '已被父根 {name} 覆盖', { name: root.covered_by })}</span
                  >
                {/if}
                <button type="button" class="permission" onclick={() => void toggleRoot(user, root)}
                  >{root.read_only ? t($language, '只读') : t($language, '读写')}</button
                >
                <button
                  type="button"
                  class="text-button danger"
                  onclick={() => void removeRoot(user, root)}>{t($language, '删除')}</button
                >
              </div>
            {/each}
            <button type="button" class="assign-button" onclick={() => (assignUser = user)}
              >{t($language, '+ 分配外部路径')}</button
            >
          </div>
        </article>
      {/each}
    </section>

    <section class="admin-section">
      <div class="section-heading">
        <h2>{t($language, '外部路径')}</h2>
        <button type="button" class="add-button" onclick={() => (pathOpen = true)}
          >{t($language, '+ 添加外部路径')}</button
        >
      </div>
      {#each paths as path (path.id)}
        <div class="path-card">
          {#if editingPathId === path.id}
            <div class="path-alias-form">
              <input
                bind:value={editingPathAlias}
                aria-label={t($language, '外部路径别名')}
                onblur={() => void finishPathAlias()}
                onkeydown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    event.currentTarget.blur();
                  } else if (event.key === 'Escape') {
                    editingPathId = null;
                    editingPathAlias = editingPathOriginal;
                    event.currentTarget.blur();
                  }
                }}
              />
            </div>
          {:else}
            <button type="button" class="path-alias" onclick={() => beginPathAlias(path)}
              >{path.alias}</button
            >
          {/if}
          <span class="root-path">{path.real_path}</span>
          <button type="button" class="text-button danger" onclick={() => void deletePath(path)}
            >{t($language, '删除')}</button
          >
        </div>
      {:else}
        <p class="muted">{t($language, '还没有外部路径。')}</p>
      {/each}
    </section>

    <section class="admin-section">
      <h2>{t($language, '品牌')}</h2>
      <div class="setting-grid">
        <label class="setting-row"
          ><span>{t($language, '品牌名称')}</span>
          <div class="quota-setting">
            <input bind:value={brandNameInput} maxlength="64" />
            <button type="button" class="small" onclick={() => void saveBrandName()}
              >{t($language, '保存')}</button
            >
          </div></label
        >
        <div class="setting-row">
          <span>{t($language, '品牌图标')}</span>
          <div class="quota-setting">
            {#if $branding.iconUrl}
              <img class="brand-preview" src={$branding.iconUrl} alt="" />
            {:else}
              <span class="brand-preview fallback">{brandMark($branding.name)}</span>
            {/if}
            <label class="small upload-label"
              >{t($language, '上传图标')}<input
                type="file"
                accept="image/png,image/jpeg,image/webp"
                disabled={brandBusy}
                onchange={(event) => void uploadBrandIcon(event)}
              /></label
            >
            <button
              type="button"
              class="small"
              disabled={brandBusy}
              onclick={() => void resetBrandIcon()}>{t($language, '恢复默认图标')}</button
            >
          </div>
        </div>
      </div>
      <p class="brand-hint">{t($language, '仅支持 PNG/JPEG/WebP，最大 2MB，建议正方形。')}</p>
    </section>

    <section class="admin-section">
      <h2>{t($language, '系统设置')}</h2>
      <div class="setting-grid">
        <label class="setting-row"
          ><span>{t($language, '缩略图')}</span><input
            type="checkbox"
            checked={Boolean(setting('thumbnails.enabled'))}
            onchange={(event) => void setSetting('thumbnails.enabled', event.currentTarget.checked)}
          /></label
        >
        <div class="setting-row wide">
          <span>{t($language, 'ZIP 下载限制')}</span>
          <div class="zip-fields">
            <label class="zip-field"
              ><span>{t($language, '最大大小')}</span>
              <input bind:value={zipMaxGb} type="number" min="0" step="1" /><span>GB</span></label
            >
            <label class="zip-field"
              ><span>{t($language, '最大文件数')}</span>
              <input bind:value={zipMaxFiles} type="number" min="0" step="1" /></label
            >
            <label class="zip-field"
              ><span>{t($language, '并发数')}</span>
              <input bind:value={zipConcurrency} type="number" min="1" step="1" /></label
            >
            <button type="button" class="small" onclick={() => void saveZipSettings()}
              >{t($language, '保存')}</button
            >
          </div>
        </div>
        <label class="setting-row"
          ><span>{t($language, '文本编辑器')}</span><input
            type="checkbox"
            checked={Boolean(setting('editor.enabled'))}
            onchange={(event) => void setSetting('editor.enabled', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>{t($language, '终端')}</span><input
            type="checkbox"
            checked={Boolean(setting('terminal_enabled'))}
            onchange={(event) => void setSetting('terminal_enabled', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>{t($language, '默认显示隐藏文件')}</span><input
            type="checkbox"
            checked={Boolean(setting('show_hidden_default'))}
            onchange={(event) =>
              void setSetting('show_hidden_default', event.currentTarget.checked)}
          /></label
        >
        <label class="setting-row"
          ><span>{t($language, '默认主题')}</span><select
            value={String(setting('ui.default_theme') ?? 'system')}
            onchange={(event) => void setSetting('ui.default_theme', event.currentTarget.value)}
            ><option value="system">{t($language, '跟随系统')}</option><option value="light"
              >{t($language, '浅色')}</option
            ><option value="dark">{t($language, '深色')}</option></select
          ></label
        >
        <label class="setting-row"
          ><span>{t($language, '默认语言')}</span><select
            value={String(setting('ui.default_locale') ?? 'zh-CN')}
            onchange={(event) => void setSetting('ui.default_locale', event.currentTarget.value)}
            ><option value="zh-CN">{t($language, '中文')}</option><option value="en">English</option
            ></select
          ></label
        >
        <label class="setting-row"
          ><span>{t($language, '上传同名文件')}</span><select
            value={String(setting('upload_conflict_default') ?? 'rename')}
            onchange={(event) =>
              void setSetting('upload_conflict_default', event.currentTarget.value)}
            ><option value="rename">{t($language, '自动重命名')}</option><option value="overwrite"
              >{t($language, '覆盖')}</option
            ><option value="skip">{t($language, '跳过')}</option></select
          ></label
        >
        <label class="setting-row"
          ><span>{t($language, '默认配额')}</span>
          <div class="quota-setting">
            <input bind:value={defaultQuotaGb} type="number" min="0" step="1" /><span>GB</span
            ><button type="button" class="small" onclick={() => void saveDefaultQuota()}
              >{t($language, '保存')}</button
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
    title={t($language, '重置密码')}
    label={t($language, '{username} 的新密码', { username: passwordUser.username })}
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
    title={t($language, '删除用户')}
    message={t($language, '删除用户「{username}」？默认保留其 home 文件。', {
      username: deleteUser.username,
    })}
    confirmLabel={t($language, '删除')}
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
  .status-grid {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .status-row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 5px 0;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .status-row:last-child {
    border-bottom: 0;
  }

  .slow-requests {
    margin-top: 10px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .slow-requests summary {
    cursor: pointer;
  }

  .tag {
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 11px;
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

  .path-card .root-path {
    flex: 1;
  }
  .setting-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }
  .setting-row {
    display: flex;
    min-height: 52px;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
    padding: 10px 16px;
    border-top: 1px solid var(--border);
    color: var(--text);
    font-size: 13.5px;
  }
  .setting-row:first-child {
    border-top: 0;
  }
  .slow-hint {
    margin: 6px 0 2px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .zip-fields {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 14px;
  }

  .zip-field {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 13px;
    white-space: nowrap;
  }

  .zip-field input {
    width: 72px;
    height: 32px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
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
  .brand-hint {
    margin: 10px 0 0;
    color: var(--text-2);
    font-size: 13px;
    line-height: 1.5;
  }

  .brand-preview {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border-radius: 10px;
    object-fit: cover;
  }

  .brand-preview.fallback {
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 600;
  }

  .upload-label {
    display: inline-flex;
    align-items: center;
    cursor: pointer;
  }

  .upload-label input {
    display: none;
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
  @media (max-width: 520px) {
    .admin-header {
      align-items: flex-start;
      flex-wrap: wrap;
      gap: 10px;
    }

    h1 {
      font-size: 22px;
    }

    .audit-button {
      white-space: nowrap;
    }

    .user-meta {
      flex-direction: column;
      gap: 4px;
    }
  }
</style>
