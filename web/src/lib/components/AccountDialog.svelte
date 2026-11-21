<script lang="ts">
  import type { User } from '../api';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    user,
    avatarUrl,
    onSettings,
    onAdmin,
    onLogout,
    onClose,
  }: {
    user: User;
    avatarUrl: string;
    onSettings: () => void;
    onAdmin: () => void;
    onLogout: () => void;
    onClose: () => void;
  } = $props();
</script>

<div class="account-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <section
    class="account-dialog"
    aria-label={t($language, '账号菜单')}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <button
        type="button"
        class="icon-button close-button"
        aria-label={t($language, '关闭')}
        onclick={onClose}
      >
        <Icon name="close" size={18} />
      </button>
      {#if avatarUrl}
        <div class="avatar avatar-image"><img src={avatarUrl} alt="" /></div>
      {:else}
        <div class="avatar">{user.username.slice(0, 1).toUpperCase()}</div>
      {/if}
      <strong>{user.username}</strong>
    </header>

    <div class="menu-section">
      <button type="button" onclick={onSettings}>
        <Icon name="settings" size={18} />{t($language, '个人设置')}
      </button>
      {#if user.role === 'admin'}
        <button type="button" onclick={onAdmin}>
          <Icon name="shield" size={18} />{t($language, '系统管理')}
        </button>
      {/if}
      <button type="button" class="danger" onclick={onLogout}>
        <Icon name="logout" size={18} />{t($language, '退出登录')}
      </button>
    </div>
  </section>
</div>

<style>
  .account-overlay {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: flex;
    justify-content: flex-end;
    padding: 64px 18px 18px;
    background: var(--scrim);
  }

  .account-dialog {
    display: flex;
    width: min(300px, 100%);
    height: fit-content;
    flex-direction: column;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  header {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 8px 16px;
  }

  header strong {
    font-size: 16px;
    font-weight: 600;
  }

  .avatar-image {
    overflow: hidden;
  }

  .avatar-image img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .avatar {
    display: grid;
    width: 56px;
    height: 56px;
    flex: 0 0 auto;
    place-items: center;
    box-sizing: border-box;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 22px;
    font-weight: 600;
  }

  .close-button {
    position: absolute;
    top: 0;
    right: 0;
  }

  .icon-button {
    display: grid;
    width: 30px;
    height: 30px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .menu-section {
    display: flex;
    flex-direction: column;
    padding: 6px 0;
    border-top: 1px solid var(--border);
  }

  .menu-section button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    height: 38px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: center;
  }

  .menu-section button:hover {
    background: var(--hover);
  }

  .menu-section button.danger {
    color: var(--danger);
  }

  @media (max-width: 640px) {
    .account-overlay {
      align-items: flex-start;
      justify-content: flex-end;
      padding: 60px 12px 12px;
    }

    .account-dialog {
      width: min(300px, 100%);
      max-height: calc(100dvh - 72px);
      overflow-y: auto;
    }

    .menu-section button {
      min-height: 44px;
    }
  }
</style>
