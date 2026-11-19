<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte';
  import { SvelteMap } from 'svelte/reactivity';
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
  let loadedOnce = false;
  let createOpen = $state(false);
  let pathOpen = $state(false);
  let quotaUser = $state<User | null>(null);
  let passwordUser = $state<User | null>(null);
  let assignUser = $state<User | null>(null);
  let deleteUser = $state<User | null>(null);
  let removeRootTarget = $state<{ user: User; root: AdminRoot } | null>(null);
  let deletePathTarget = $state<ServerPath | null>(null);
  let editingPathId = $state<number | null>(null);
  let editingPathAlias = $state('');
  let editingPathOriginal = $state('');
  let savingPathId = $state<number | null>(null);
  let pathAliasInput = $state<HTMLInputElement | null>(null);

  function password(): string {
    const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%';
    return Array.from(
      crypto.getRandomValues(new Uint8Array(20)),
      (v) => alphabet[v % alphabet.length],
    ).join('');
  }

  async function load(): Promise<void> {
    if (!loadedOnce) loading = true;
    try {
      const [userResult, pathResult, settingsResult] = await Promise.all([
        api.listUsers(),
        api.listServerPaths(),
        api.getSettings(),
      ]);
      users = userResult.users;
      paths = pathResult.paths;
      settings = settingsResult;
      loading = false;
      loadedOnce = true;
      void loadDeferred();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '管理数据加载失败'));
    } finally {
      loading = false;
      loadedOnce = true;
    }
  }

  async function loadDeferred(): Promise<void> {
    try {
      const [usageResult, rootsResult, statusResult] = await Promise.all([
        api.adminUsage(),
        api.listAllRoots(),
        api.adminStatus(),
      ]);
      applyUsage(usageResult);
      roots = Object.fromEntries(
        Object.entries(rootsResult.roots).map(([id, entries]) => [Number(id), entries]),
      );
      status = statusResult;
      scheduleUsagePoll();
    } catch (error) {
      console.warn('could not load deferred admin data', error);
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
    if (settingFlushTimer) clearTimeout(settingFlushTimer);
    for (const timer of savedTimers.values()) clearTimeout(timer);
    savedTimers.clear();
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

  async function toggleFeature(
    user: User,
    key: 'terminal' | 'zip' | 'unzip',
    value: boolean,
  ): Promise<void> {
    const original = { ...user.features };
    user.features = { ...user.features, [key]: value };
    try {
      const result = await api.updateUser(
        user.id,
        {
          [`feature_${key}`]: value,
        },
        csrf,
      );
      const index = users.findIndex((entry) => entry.id === user.id);
      if (index >= 0) users[index] = result.user;
    } catch (error) {
      user.features = original;
      showNotice(error instanceof Error ? error.message : t($language, '功能开关保存失败'));
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

  async function beginPathAlias(path: ServerPath): Promise<void> {
    editingPathId = path.id;
    editingPathAlias = path.alias;
    editingPathOriginal = path.alias;
    await tick();
    pathAliasInput?.focus();
    pathAliasInput?.select();
  }

  async function finishPathAlias(): Promise<void> {
    if (editingPathId === null) return;
    const id = editingPathId;
    const original = editingPathOriginal;
    const value = editingPathAlias.trim();
    editingPathId = null;
    editingPathAlias = original;
    if (!value || value === original) return;
    const target = paths.find((path) => path.id === id);
    const previous = target?.alias ?? original;
    if (target) target.alias = value;
    savingPathId = id;
    try {
      const result = await api.updateServerPath(id, value, csrf);
      if (target) target.alias = result.alias;
    } catch (error) {
      if (target) target.alias = previous;
      editingPathId = id;
      editingPathAlias = value;
      editingPathOriginal = original;
      showNotice(error instanceof Error ? error.message : t($language, '修改别名失败'));
    } finally {
      savingPathId = null;
    }
  }

  async function confirmDeletePath(): Promise<void> {
    const path = deletePathTarget;
    deletePathTarget = null;
    if (!path) return;
    try {
      await api.deleteServerPath(path.id, csrf);
      await load();
      onRootsChanged?.();
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '删除外部路径失败'));
    }
  }

  async function toggleRoot(user: User, root: AdminRoot): Promise<void> {
    const previous = root.read_only;
    root.read_only = !previous;
    try {
      await api.updateRoot(
        user.id,
        root.id,
        { alias: root.alias, read_only: root.read_only, sort_order: root.sort_order },
        csrf,
      );
      onRootsChanged?.();
    } catch (error) {
      root.read_only = previous;
      showNotice(error instanceof Error ? error.message : t($language, '权限修改失败'));
    }
  }

  async function confirmRemoveRoot(): Promise<void> {
    const target = removeRootTarget;
    removeRootTarget = null;
    if (!target) return;
    try {
      await api.deleteRoot(target.user.id, target.root.id, csrf);
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

  type SettingState = 'saving' | 'saved' | 'error';
  let settingStates = $state<Record<string, SettingState>>({});
  let settingErrors = $state<Record<string, string>>({});
  const pendingSettings = new SvelteMap<string, unknown>();
  const savedTimers = new SvelteMap<string, ReturnType<typeof setTimeout>>();
  let settingFlushTimer: ReturnType<typeof setTimeout> | null = null;
  let settingsChain: Promise<void> = Promise.resolve();

  function markSettingState(key: string, state: SettingState, message = ''): void {
    settingStates = { ...settingStates, [key]: state };
    settingErrors = { ...settingErrors, [key]: message };
    const existing = savedTimers.get(key);
    if (existing) {
      clearTimeout(existing);
      savedTimers.delete(key);
    }
    if (state === 'saved') {
      const timer = setTimeout(() => {
        savedTimers.delete(key);
        if (settingStates[key] !== 'saved') return;
        const next = { ...settingStates };
        delete next[key];
        settingStates = next;
      }, 1500);
      savedTimers.set(key, timer);
    }
  }

  function setLocalSetting(key: string, value: unknown): void {
    if (!settings) return;
    const parts = key.split('.');
    let target = settings.settings as Record<string, unknown>;
    for (let index = 0; index < parts.length - 1; index += 1) {
      const part = parts[index];
      if (!part) continue;
      const next = target[part];
      if (!next || typeof next !== 'object') target[part] = {};
      target = target[part] as Record<string, unknown>;
    }
    const last = parts[parts.length - 1];
    if (last) target[last] = value;
  }

  function applySettingsSnapshot(result: SettingsResponse): void {
    settings = result;
    for (const [key, value] of pendingSettings) setLocalSetting(key, value);
  }

  function queueSetting(key: string, value: unknown): void {
    setLocalSetting(key, value);
    pendingSettings.set(key, value);
    markSettingState(key, 'saving');
    if (settingFlushTimer) clearTimeout(settingFlushTimer);
    settingFlushTimer = setTimeout(() => {
      settingFlushTimer = null;
      void flushSettings();
    }, 250);
  }

  async function flushSettings(): Promise<void> {
    if (!settings || pendingSettings.size === 0) return;
    const entries = Array.from(pendingSettings);
    pendingSettings.clear();
    const values = Object.fromEntries(entries);
    settingsChain = settingsChain.then(async () => {
      try {
        const result = await api.updateSettings(values, csrf);
        applySettingsSnapshot(result);
        for (const key of Object.keys(values)) {
          if (!pendingSettings.has(key)) markSettingState(key, 'saved');
        }
        if ('ui.brand_name' in values) void loadBranding();
      } catch (error) {
        const message =
          error instanceof Error ? error.message : t($language, '系统设置保存失败');
        showNotice(message);
        for (const key of Object.keys(values)) {
          if (!pendingSettings.has(key)) markSettingState(key, 'error', message);
        }
        try {
          applySettingsSnapshot(await api.getSettings());
        } catch (refreshError) {
          console.warn('could not reload settings after failed save', refreshError);
        }
      }
    });
    await settingsChain;
  }

  function saveNumber(
    key: string,
    raw: string | number,
    minimum: number,
    maximum: number,
    scale = 1,
  ): void {
    const value = Number(raw);
    if (!Number.isFinite(value) || value < minimum || value > maximum) {
      markSettingState(key, 'error', t($language, '数值超出允许范围'));
      return;
    }
    const next = Math.round(value * scale);
    const current = Number(setting(key));
    if (Number.isFinite(current) && current === next) return;
    queueSetting(key, next);
  }

  let defaultQuotaGb = $state(15);
  let settingReady = $state(false);
  $effect(() => {
    if (settings && !settingReady) {
      defaultQuotaGb = Number(setting('default_quota_bytes') ?? 15 * 1024 ** 3) / 1024 ** 3;
      settingReady = true;
    }
  });

  function saveDefaultQuota(): void {
    saveNumber('default_quota_bytes', defaultQuotaGb, 0, 1024 * 1024, 1024 ** 3);
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

  let customCssInput = $state('');
  let customCssBusy = $state(false);
  let loadedCssVersion = -1;

  $effect(() => {
    const version = $branding.customCssVersion;
    if (version === loadedCssVersion) return;
    loadedCssVersion = version;
    if (version <= 0) {
      customCssInput = '';
      return;
    }
    void fetch(api.customCssUrl(version), { credentials: 'include' })
      .then((response) => (response.ok ? response.text() : ''))
      .then((text) => (customCssInput = text))
      .catch((error) => console.warn('could not load custom CSS', error));
  });

  async function saveCustomCss(): Promise<void> {
    customCssBusy = true;
    try {
      await api.saveCustomCss(customCssInput, csrf);
      await loadBranding();
      showNotice(t($language, '已保存'), 'info', t($language, '自定义 CSS'));
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '保存失败'));
    } finally {
      customCssBusy = false;
    }
  }

  async function clearCustomCss(): Promise<void> {
    customCssBusy = true;
    try {
      await api.clearCustomCss(csrf);
      customCssInput = '';
      await loadBranding();
      showNotice(t($language, '已清除'), 'info', t($language, '自定义 CSS'));
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '操作失败'));
    } finally {
      customCssBusy = false;
    }
  }

  function saveBrandName(): void {
    const value = brandNameInput.trim();
    if (!value) {
      markSettingState('ui.brand_name', 'error', t($language, '品牌名称不能为空'));
      return;
    }
    if (value === String(setting('ui.brand_name') ?? '')) return;
    queueSetting('ui.brand_name', value);
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

  let previewEdge = $state(2048);
  let jpegMaxMp = $state(1000);
  let maxPixelsMp = $state(50);
  let maxSourceMb = $state(512);
  let thumbSettingsReady = $state(false);
  $effect(() => {
    if (settings && !thumbSettingsReady) {
      previewEdge = Number(setting('thumbnails.preview_edge') ?? 2048);
      jpegMaxMp = Number(setting('thumbnails.jpeg_max_pixels') ?? 1_000_000_000) / 1_000_000;
      maxPixelsMp = Number(setting('thumbnails.max_pixels') ?? 50_000_000) / 1_000_000;
      maxSourceMb = Number(setting('thumbnails.max_source_bytes') ?? 512 * 1024 ** 2) / 1024 ** 2;
      thumbSettingsReady = true;
    }
  });

  function blurOnEnter(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
      event.preventDefault();
      (event.currentTarget as HTMLElement | null)?.blur();
    }
  }

  function settingSource(key: string): string {
    const source = settings?.sources[key] ?? 'default';
    return source === 'db'
      ? t($language, '数据库')
      : source === 'config'
        ? t($language, '配置文件')
        : t($language, '默认值');
  }

  async function clearSetting(key: string): Promise<void> {
    pendingSettings.delete(key);
    markSettingState(key, 'saving');
    try {
      applySettingsSnapshot(await api.resetSetting(key, csrf));
      markSettingState(key, 'saved');
    } catch (error) {
      const message = error instanceof Error ? error.message : t($language, '系统设置保存失败');
      showNotice(message);
      markSettingState(key, 'error', message);
    }
  }

  function saveThumbSettings(): void {
    saveNumber('thumbnails.preview_edge', previewEdge, 512, 4096);
    saveNumber('thumbnails.jpeg_max_pixels', jpegMaxMp, 1, 100_000, 1_000_000);
    saveNumber('thumbnails.max_pixels', maxPixelsMp, 1, 100_000, 1_000_000);
    saveNumber('thumbnails.max_source_bytes', maxSourceMb, 1, 1_048_576, 1024 ** 2);
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

  function saveZipSettings(): void {
    saveNumber('downloads.zip_max_bytes', zipMaxGb, 0, 1024 * 1024, 1024 ** 3);
    saveNumber('downloads.zip_max_files', zipMaxFiles, 0, 10_000_000);
    saveNumber('downloads.zip_concurrency', zipConcurrency, 1, 64);
  }
</script>

{#snippet saveState(key: string)}
  {#if settingStates[key]}
    <small
      class="setting-save"
      class:saved={settingStates[key] === 'saved'}
      class:error={settingStates[key] === 'error'}
      title={settingErrors[key] || ''}
      >{settingStates[key] === 'saving'
        ? t($language, '保存中…')
        : settingStates[key] === 'saved'
          ? t($language, '已保存')
          : t($language, '保存失败')}</small
    >
  {/if}
{/snippet}

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
          <div class="feature-row">
            <label>
              <input
                type="checkbox"
                checked={user.features.terminal}
                onchange={(event) =>
                  void toggleFeature(user, 'terminal', event.currentTarget.checked)}
              />
              {t($language, '允许终端')}
            </label>
            <label>
              <input
                type="checkbox"
                checked={user.features.zip}
                onchange={(event) => void toggleFeature(user, 'zip', event.currentTarget.checked)}
              />
              {t($language, '允许ZIP')}
            </label>
            <label>
              <input
                type="checkbox"
                checked={user.features.unzip}
                onchange={(event) =>
                  void toggleFeature(user, 'unzip', event.currentTarget.checked)}
              />
              {t($language, '允许解压')}
            </label>
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
                  onclick={() => (removeRootTarget = { user, root })}>{t($language, '删除')}</button
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
                bind:this={pathAliasInput}
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
          {#if savingPathId === path.id}
            <small class="setting-save">{t($language, '保存中…')}</small>
          {/if}
          <span class="root-path">{path.real_path}</span>
          <button type="button" class="text-button danger" onclick={() => (deletePathTarget = path)}
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
          ><span>{t($language, '品牌名称')}{@render saveState('ui.brand_name')}</span>
          <div class="quota-setting">
            <input
              bind:value={brandNameInput}
              maxlength="64"
              onblur={saveBrandName}
              onkeydown={blurOnEnter}
            />
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
      <div class="settings-group">
        <h3>{t($language, '自定义 CSS')}</h3>
        <p class="settings-hint">
          {t($language, '全局生效，所有页面和分享页都会加载。最多 256 KiB。')}
        </p>
        <textarea
          bind:value={customCssInput}
          rows="8"
          spellcheck="false"
          placeholder={t($language, '在此粘贴 CSS')}></textarea>
        <div class="css-actions">
          <button
            type="button"
            class="small"
            disabled={customCssBusy}
            onclick={() => void saveCustomCss()}>{t($language, '保存')}</button
          >
          <button
            type="button"
            class="small ghost"
            disabled={customCssBusy}
            onclick={() => void clearCustomCss()}>{t($language, '清除')}</button
          >
        </div>
      </div>
    </section>

    <section class="admin-section">
      <h2>{t($language, '系统设置')}</h2>

      <div class="settings-group">
        <h3>{t($language, '功能开关')}</h3>
        <p class="settings-hint">
          {t($language, '关闭后，用户偏好设置中的对应选项会隐藏。后端仍会强制关闭该功能。')}
        </p>
        <div class="setting-grid">
          <label class="setting-row"
            ><span>{t($language, '缩略图')}{@render saveState('thumbnails.enabled')}</span><input
              type="checkbox"
              checked={Boolean(setting('thumbnails.enabled'))}
              onchange={(event) =>
                queueSetting('thumbnails.enabled', event.currentTarget.checked)}
            /></label
          >
          <label class="setting-row"
            ><span>{t($language, '文本编辑器')}{@render saveState('editor.enabled')}</span><input
              type="checkbox"
              checked={Boolean(setting('editor.enabled'))}
              onchange={(event) => queueSetting('editor.enabled', event.currentTarget.checked)}
            /></label
          >
          <label class="setting-row"
            ><span>{t($language, '终端')}{@render saveState('terminal_enabled')}</span><input
              type="checkbox"
              checked={Boolean(setting('terminal_enabled'))}
              onchange={(event) => queueSetting('terminal_enabled', event.currentTarget.checked)}
            /></label
          >
        </div>
      </div>

      <div class="settings-group">
        <h3>{t($language, '下载与配额')}</h3>
        <div class="setting-grid">
          <div class="setting-row wide">
            <span>{t($language, 'ZIP 下载限制')}</span>
            <div class="zip-fields">
              <label class="zip-field"
                ><span>{t($language, '最大大小')}{@render saveState('downloads.zip_max_bytes')}</span>
                <input
                  bind:value={zipMaxGb}
                  type="number"
                  min="0"
                  step="1"
                  onblur={saveZipSettings}
                  onkeydown={blurOnEnter}
                /><span>GB</span></label
              >
              <label class="zip-field"
                ><span>{t($language, '最大文件数')}{@render saveState('downloads.zip_max_files')}</span>
                <input
                  bind:value={zipMaxFiles}
                  type="number"
                  min="0"
                  step="1"
                  onblur={saveZipSettings}
                  onkeydown={blurOnEnter}
                /></label
              >
              <label class="zip-field"
                ><span>{t($language, '并发数')}{@render saveState('downloads.zip_concurrency')}</span>
                <input
                  bind:value={zipConcurrency}
                  type="number"
                  min="1"
                  step="1"
                  onblur={saveZipSettings}
                  onkeydown={blurOnEnter}
                /></label
              >
            </div>
          </div>
          <label class="setting-row"
            ><span>{t($language, '默认配额')}{@render saveState('default_quota_bytes')}</span>
            <div class="quota-setting">
              <input
                bind:value={defaultQuotaGb}
                type="number"
                min="0"
                step="1"
                onblur={saveDefaultQuota}
                onkeydown={blurOnEnter}
              /><span>GB</span>
            </div></label
          >
        </div>
      </div>

      <div class="settings-group">
        <h3>{t($language, '缩略图与预览')}</h3>
        <p class="settings-hint">
          {t($language, '预览长边范围 512–4096；像素上限按百万像素（MP）计。')}
        </p>
        <div class="setting-grid">
          <label class="setting-row wide"
            ><span
              >{t($language, '预览长边')}
              <small class="setting-source">{settingSource('thumbnails.preview_edge')}</small>
              {@render saveState('thumbnails.preview_edge')}</span
            >
            <div class="quota-setting">
              <input
                bind:value={previewEdge}
                type="number"
                min="512"
                max="4096"
                step="128"
                onblur={saveThumbSettings}
                onkeydown={blurOnEnter}
              />
              <span>px</span>
              {#if settings?.sources['thumbnails.preview_edge'] === 'db'}
                <button
                  type="button"
                  class="small"
                  onclick={() => void clearSetting('thumbnails.preview_edge')}
                  >{t($language, '清除覆盖')}</button
                >
              {/if}
            </div></label
          >
          <label class="setting-row wide"
            ><span
              >{t($language, 'JPEG 像素上限')}
              <small class="setting-source"
                >{settingSource('thumbnails.jpeg_max_pixels')}</small
              >
              {@render saveState('thumbnails.jpeg_max_pixels')}</span
            >
            <div class="quota-setting">
              <input
                bind:value={jpegMaxMp}
                type="number"
                min="1"
                step="1"
                onblur={saveThumbSettings}
                onkeydown={blurOnEnter}
              /><span>MP</span>
            </div></label
          >
          <label class="setting-row wide"
            ><span
              >{t($language, '其他图片像素上限')}
              <small class="setting-source">{settingSource('thumbnails.max_pixels')}</small>
              {@render saveState('thumbnails.max_pixels')}</span
            >
            <div class="quota-setting">
              <input
                bind:value={maxPixelsMp}
                type="number"
                min="1"
                step="1"
                onblur={saveThumbSettings}
                onkeydown={blurOnEnter}
              /><span>MP</span>
            </div></label
          >
          <label class="setting-row wide"
            ><span
              >{t($language, '源文件大小上限')}
              <small class="setting-source">{settingSource('thumbnails.max_source_bytes')}</small>
              {@render saveState('thumbnails.max_source_bytes')}</span
            >
            <div class="quota-setting">
              <input
                bind:value={maxSourceMb}
                type="number"
                min="1"
                step="1"
                onblur={saveThumbSettings}
                onkeydown={blurOnEnter}
              /><span>MB</span>
            </div></label
          >
        </div>
      </div>

      <div class="settings-group">
        <h3>{t($language, '新用户默认值')}</h3>
        <p class="settings-hint">
          {t($language, '只影响新建用户；已有用户继续使用自己的偏好。')}
        </p>
        <div class="setting-grid">
          <label class="setting-row"
            ><span
              >{t($language, '默认显示隐藏文件')}{@render saveState('show_hidden_default')}</span
            ><input
              type="checkbox"
              checked={Boolean(setting('show_hidden_default'))}
              onchange={(event) =>
                queueSetting('show_hidden_default', event.currentTarget.checked)}
            /></label
          >
          <label class="setting-row"
            ><span>{t($language, '默认主题')}{@render saveState('ui.default_theme')}</span><select
              value={String(setting('ui.default_theme') ?? 'system')}
              onchange={(event) => queueSetting('ui.default_theme', event.currentTarget.value)}
              ><option value="system">{t($language, '跟随系统')}</option><option value="light"
                >{t($language, '浅色')}</option
              ><option value="dark">{t($language, '深色')}</option></select
            ></label
          >
          <label class="setting-row"
            ><span>{t($language, '默认语言')}{@render saveState('ui.default_locale')}</span><select
              value={String(setting('ui.default_locale') ?? 'zh-CN')}
              onchange={(event) => queueSetting('ui.default_locale', event.currentTarget.value)}
              ><option value="zh-CN">{t($language, '中文')}</option><option value="en"
                >English</option
              ></select
            ></label
          >
          <label class="setting-row"
            ><span
              >{t($language, '上传同名文件')}{@render saveState('upload_conflict_default')}</span
            ><select
              value={String(setting('upload_conflict_default') ?? 'rename')}
              onchange={(event) =>
                queueSetting('upload_conflict_default', event.currentTarget.value)}
              ><option value="rename">{t($language, '自动重命名')}</option><option value="overwrite"
                >{t($language, '覆盖')}</option
              ><option value="skip">{t($language, '跳过')}</option></select
            ></label
          >
        </div>
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
{#if removeRootTarget}<ConfirmDialog
    open
    title={t($language, '移除外部路径')}
    message={t($language, '从用户「{username}」移除「{alias}」？', {
      username: removeRootTarget.user.username,
      alias: removeRootTarget.root.alias,
    })}
    confirmLabel={t($language, '移除')}
    danger
    onConfirm={() => void confirmRemoveRoot()}
    onCancel={() => (removeRootTarget = null)}
  />{/if}
{#if deletePathTarget}<ConfirmDialog
    open
    title={t($language, '删除外部路径')}
    message={t($language, '删除外部路径「{alias}」？', { alias: deletePathTarget.alias })}
    confirmLabel={t($language, '删除')}
    danger
    onConfirm={() => void confirmDeletePath()}
    onCancel={() => (deletePathTarget = null)}
  />{/if}
{#if deleteUser}<ConfirmDialog
    open
    title={t($language, '删除用户')}
    message={t($language, '删除用户「{username}」？默认保留其 home 文件。', {
      username: deleteUser.username,
    })}
    confirmLabel={t($language, '删除')}
    requireText={deleteUser.username}
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
  .settings-group {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .settings-group h3 {
    margin: 8px 0 0;
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
  }
  .setting-save {
    margin-left: 6px;
    color: var(--text-2);
    font-size: 11.5px;
    font-weight: 400;
  }

  .setting-save.saved {
    color: var(--accent-text);
  }

  .setting-save.error {
    color: var(--danger);
  }

  .setting-source {
    margin-left: 6px;
    color: var(--text-2);
    font-size: 11.5px;
  }

  .settings-hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12.5px;
  }
  .settings-group textarea {
    width: 100%;
    min-height: 140px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12.5px;
    resize: vertical;
  }
  .css-actions {
    display: flex;
    gap: 8px;
  }
  .small.ghost {
    background: transparent;
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

  .feature-row {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem 1.25rem;
    margin-top: 0.5rem;
    font-size: 0.9rem;
  }

  .feature-row label {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    cursor: pointer;
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
