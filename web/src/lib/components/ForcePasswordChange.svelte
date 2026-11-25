<script lang="ts">
  import { api, ApiError, type AuthResponse } from '../api';
  import { language, t } from '../i18n';
  import { branding, brandMark } from '../branding';

  let {
    csrf,
    onSuccess,
    onLogout,
  }: {
    csrf: string;
    onSuccess: (auth: AuthResponse) => void;
    onLogout: () => void;
  } = $props();

  let next = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(): Promise<void> {
    error = '';
    if ([...next].length < 8) {
      error = t($language, '密码至少需要 8 位');
      return;
    }
    if (next !== confirm) {
      error = t($language, '两次输入的新密码不一致');
      return;
    }
    busy = true;
    try {
      await api.forcePasswordChange(next, csrf);
      onSuccess(await api.me());
    } catch (err) {
      error = err instanceof ApiError ? err.message : t($language, '修改密码失败');
    } finally {
      busy = false;
    }
  }

  function logout(): void {
    onLogout();
  }
</script>

<main class="auth-shell">
  <section class="auth-card" aria-labelledby="force-password-title">
    <div class="brand">
      <span class="brand-mark" class:has-icon={$branding.iconUrl} aria-hidden="true">
        {#if $branding.iconUrl}<img src={$branding.iconUrl} alt="" />{:else}{brandMark(
            $branding.name,
          )}{/if}
      </span>
      <span>{$branding.name}</span>
    </div>

    <h1 id="force-password-title">{t($language, '必须修改密码')}</h1>
    <p class="lede">
      {t($language, '管理员重置了你的密码。请设置新密码后继续。')}
    </p>

    <form
      onsubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label>
        <span>{t($language, '新密码')}</span>
        <input
          bind:value={next}
          type="password"
          autocomplete="new-password"
          minlength="8"
          required
        />
      </label>
      <label>
        <span>{t($language, '确认新密码')}</span>
        <input
          bind:value={confirm}
          type="password"
          autocomplete="new-password"
          minlength="8"
          required
        />
      </label>

      <p class="hint">{t($language, '密码至少 8 位。')}</p>

      {#if error}
        <p class="error" role="alert">{error}</p>
      {/if}

      <button class="primary" type="submit" disabled={busy}>
        {busy ? t($language, '更新中…') : t($language, '更新密码')}
      </button>
    </form>

    <button type="button" class="text-button" onclick={logout}>
      {t($language, '退出登录')}
    </button>
  </section>
</main>

<style>
  .auth-shell {
    display: grid;
    min-height: 100vh;
    place-items: center;
    padding: 24px;
    background: var(--bg);
  }

  .auth-card {
    display: grid;
    width: min(420px, 100%);
    gap: 16px;
    padding: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .brand-mark {
    display: grid;
    width: 32px;
    height: 32px;
    place-items: center;
    border-radius: 9px;
    background: var(--accent-soft);
    color: var(--accent);
    font-weight: 700;
  }

  .brand-mark img {
    width: 100%;
    height: 100%;
    border-radius: 9px;
    object-fit: cover;
  }

  h1 {
    margin: 0;
    font-size: 1.35rem;
  }

  .lede {
    margin: 0;
    color: var(--text-muted);
    line-height: 1.6;
  }

  form {
    display: grid;
    gap: 12px;
  }

  label {
    display: grid;
    gap: 6px;
    color: var(--text-muted);
    font-size: 0.86rem;
  }

  input {
    width: 100%;
    box-sizing: border-box;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
  }

  input:focus {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 0.82rem;
  }

  .error {
    margin: 0;
    padding: 9px 11px;
    border-radius: 9px;
    background: var(--danger-soft);
    color: var(--danger);
    font-size: 0.86rem;
  }

  button {
    font: inherit;
    cursor: pointer;
  }

  .primary {
    padding: 10px 14px;
    border: 0;
    border-radius: 10px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }

  .primary:disabled {
    opacity: 0.6;
    cursor: default;
  }

  .text-button {
    justify-self: center;
    padding: 4px 8px;
    border: 0;
    background: transparent;
    color: var(--text-muted);
  }
</style>
