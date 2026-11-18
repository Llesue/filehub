<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type User, type UserPreferences } from '../api';
  import { language, t, type Lang } from '../i18n';
  import { showNotice } from '../notices';
  import AvatarCropper from './AvatarCropper.svelte';
  import Icon from './Icon.svelte';
  import PasswordDialog from './PasswordDialog.svelte';

  let {
    user,
    csrf,
    preferences,
    system,
    onPreferences,
  }: {
    user: User;
    csrf: string;
    preferences: UserPreferences;
    system: { thumbnails: boolean; editor: boolean; terminal: boolean };
    onPreferences: (preferences: UserPreferences) => void;
  } = $props();

  let passwordOpen = $state(false);
  let avatarInput = $state<HTMLInputElement | null>(null);
  let avatarCropSrc = $state('');
  let saving = $state(false);
  let shareActivityNotify = $state(true);

  onMount(() => {
    try {
      shareActivityNotify = localStorage.getItem('filehub.notify.share_activity') !== '0';
    } catch {
      shareActivityNotify = true;
    }
  });

  function setShareActivityNotify(enabled: boolean): void {
    shareActivityNotify = enabled;
    try {
      localStorage.setItem('filehub.notify.share_activity', enabled ? '1' : '0');
    } catch {
      // localStorage may be unavailable; the in-memory toggle still applies.
    }
  }

  async function persist(patch: Parameters<typeof api.updatePreferences>[0]): Promise<void> {
    saving = true;
    try {
      const result = await api.updatePreferences(patch, csrf);
      onPreferences(result.preferences);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '偏好保存失败'));
    } finally {
      saving = false;
    }
  }

  function chooseAvatar(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    avatarCropSrc = URL.createObjectURL(file);
  }

  async function saveCroppedAvatar(blob: Blob): Promise<void> {
    try {
      const result = await api.uploadAvatar(blob, csrf);
      onPreferences(result.preferences);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '头像上传失败'));
    } finally {
      avatarCropSrc = '';
    }
  }

  async function changePassword(current: string, next: string): Promise<void> {
    await api.changePassword(current, next, csrf);
    showNotice(t($language, '密码已更新'), 'info', t($language, '修改成功'));
  }
</script>

<section class="preferences-page">
  <header class="page-header">
    <div>
      <h1>{t($language, '偏好设置')}</h1>
      <p>{t($language, '账号、外观和文件浏览行为，修改后即时保存。')}</p>
    </div>
    <Icon name="settings" size={24} />
  </header>

  <section class="pref-card">
    <h2>{t($language, '账号')}</h2>
    <div class="profile-row">
      <button type="button" class="avatar-button" onclick={() => avatarInput?.click()}>
        {#if preferences.avatar_version > 0}
          <img src={api.avatarUrl(preferences.avatar_version)} alt={t($language, '头像')} />
        {:else}
          {user.username.slice(0, 1).toUpperCase()}
        {/if}
        <span class="avatar-edit"><Icon name="pencil" size={14} /></span>
      </button>
      <div class="profile-info">
        <strong>{user.username}</strong>
        <button type="button" class="link" onclick={() => (passwordOpen = true)}
          >{t($language, '修改密码')}</button
        >
      </div>
      <input
        bind:this={avatarInput}
        class="hidden-input"
        type="file"
        accept="image/jpeg,image/png,image/webp"
        onchange={chooseAvatar}
      />
    </div>
  </section>

  <section class="pref-card">
    <h2>{t($language, '外观')}</h2>
    <label class="pref-row">
      <span>{t($language, '主题')}</span>
      <select
        value={preferences.theme}
        onchange={(event) =>
          void persist({
            theme: (event.currentTarget as HTMLSelectElement).value as 'light' | 'dark' | 'system',
          })}
      >
        <option value="system">{t($language, '跟随系统')}</option>
        <option value="light">{t($language, '浅色')}</option>
        <option value="dark">{t($language, '深色')}</option>
      </select>
    </label>
    <label class="pref-row">
      <span>{t($language, '语言')}</span>
      <select
        value={preferences.language}
        onchange={(event) =>
          void persist({ language: (event.currentTarget as HTMLSelectElement).value as Lang })}
      >
        <option value="zh-CN">{t($language, '中文')}</option>
        <option value="en">English</option>
      </select>
    </label>
    {#if system.editor}
      <div class="pref-row">
        <span>{t($language, '启用文本文件编辑')}</span>
        <input
          type="checkbox"
          checked={preferences.editor_enabled}
          onchange={(event) => void persist({ editor_enabled: event.currentTarget.checked })}
        />
      </div>
    {/if}
    {#if system.terminal}
      <div class="pref-row">
        <span>{t($language, '启用终端')}</span>
        <input
          type="checkbox"
          checked={preferences.terminal_enabled}
          onchange={(event) => void persist({ terminal_enabled: event.currentTarget.checked })}
        />
      </div>
    {/if}
  </section>

  <section class="pref-card">
    <h2>{t($language, '文件浏览')}</h2>
    <label class="pref-row">
      <span>{t($language, '显示隐藏文件')}</span>
      <input
        type="checkbox"
        checked={preferences.show_hidden}
        onchange={(event) => void persist({ show_hidden: event.currentTarget.checked })}
      />
    </label>
    {#if system.thumbnails}
      <div class="pref-row">
        <span>{t($language, '显示缩略图')}</span>
        <input
          type="checkbox"
          checked={preferences.show_thumbnails}
          onchange={(event) => void persist({ show_thumbnails: event.currentTarget.checked })}
        />
      </div>
    {/if}
    <div class="pref-row">
      <span>{t($language, '分享访问通知')}</span>
      <input
        type="checkbox"
        checked={shareActivityNotify}
        onchange={(event) => setShareActivityNotify(event.currentTarget.checked)}
      />
    </div>
    <label class="pref-row">
      <span>{t($language, '上传同名文件')}</span>
      <select
        value={preferences.upload_conflict}
        onchange={(event) =>
          void persist({
            upload_conflict: (event.currentTarget as HTMLSelectElement).value as
              'rename' | 'overwrite' | 'skip',
          })}
      >
        <option value="rename">{t($language, '自动重命名')}</option>
        <option value="overwrite">{t($language, '覆盖')}</option>
        <option value="skip">{t($language, '跳过')}</option>
      </select>
    </label>
  </section>

  {#if saving}<p class="saving">{t($language, '保存中…')}</p>{/if}
</section>

<PasswordDialog
  open={passwordOpen}
  onClose={() => (passwordOpen = false)}
  onSave={changePassword}
/>

{#if avatarCropSrc}
  <AvatarCropper
    src={avatarCropSrc}
    onConfirm={(blob) => void saveCroppedAvatar(blob)}
    onClose={() => (avatarCropSrc = '')}
  />
{/if}

<style>
  .preferences-page {
    display: flex;
    width: min(100%, 1000px);
    margin: 0 auto;
    flex-direction: column;
    gap: 18px;
  }
  .page-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    margin: 0 0 4px;
    font-size: 24px;
    font-weight: 500;
  }
  .page-header p {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
  }
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
  .profile-row {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .avatar-button {
    position: relative;
    display: grid;
    width: 64px;
    height: 64px;
    flex: 0 0 auto;
    place-items: center;
    overflow: visible;
    padding: 0;
    box-sizing: border-box;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 24px;
    font-weight: 600;
  }
  .avatar-button img {
    display: block;
    width: 100%;
    height: 100%;
    border-radius: 50%;
    object-fit: cover;
  }
  .avatar-edit {
    position: absolute;
    right: -10px;
    bottom: -8px;
    display: grid;
    width: 23px;
    height: 23px;
    place-items: center;
    border: 2px solid var(--surface);
    border-radius: 50%;
    background: var(--accent);
    color: var(--on-accent);
  }
  .profile-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .profile-info strong {
    font-size: 16px;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: 13px;
  }
  .hidden-input {
    display: none;
  }
  .pref-row {
    display: flex;
    min-height: 36px;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    color: var(--text);
    font-size: 14px;
  }
  .pref-row select {
    width: min(220px, 50%);
    height: 34px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .pref-row input[type='checkbox'] {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  .saving {
    color: var(--text-2);
    font-size: 12px;
    text-align: right;
  }
</style>
