<script lang="ts">
  import { api, ApiError, type AuthResponse } from '../api';
  import { language, t } from '../i18n';
  import { branding, brandMark } from '../branding';

  let {
    mode,
    onSuccess,
  }: {
    mode: 'setup' | 'login';
    onSuccess: (auth: AuthResponse) => void;
  } = $props();

  let token = $state('');
  let username = $state('');
  let password = $state('');
  let confirmPassword = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(): Promise<void> {
    error = '';

    if (mode === 'setup' && password !== confirmPassword) {
      error = t($language, '两次输入的密码不一致');
      return;
    }

    busy = true;
    try {
      const auth =
        mode === 'setup'
          ? await api.setup({ token, username, password, language: $language })
          : await api.login({ username, password });
      onSuccess(auth);
    } catch (err) {
      error = err instanceof ApiError ? err.message : t($language, '请求失败，请稍后重试');
    } finally {
      busy = false;
    }
  }
</script>

<main class="auth-shell">
  <section class="auth-card" aria-labelledby="auth-title">
    <div class="brand">
      <span class="brand-mark" class:has-icon={$branding.iconUrl} aria-hidden="true">
        {#if $branding.iconUrl}<img src={$branding.iconUrl} alt="" />{:else}{brandMark($branding.name)}{/if}
      </span>
      <span>{$branding.name}</span>
    </div>

    <h1 id="auth-title">{mode === 'setup' ? t($language, '首次设置') : t($language, '登录')}</h1>
    <p class="lede">
      {mode === 'setup'
        ? t($language, '创建第一个管理员账号，完成后即可开始使用。')
        : t($language, '使用你的账号访问 {brand}。', { brand: $branding.name })}
    </p>

    <form
      onsubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      {#if mode === 'setup'}
        <label>
          <span>{t($language, '设置令牌')}</span>
          <input
            bind:value={token}
            autocomplete="one-time-code"
            placeholder={t($language, '启动日志中的 setup token')}
            required
          />
        </label>
      {/if}

      <label>
        <span>{t($language, '用户名')}</span>
        <input
          bind:value={username}
          autocomplete="username"
          minlength="3"
          maxlength="32"
          required
        />
      </label>

      <label>
        <span>{t($language, '密码')}</span>
        <input
          bind:value={password}
          type="password"
          autocomplete={mode === 'setup' ? 'new-password' : 'current-password'}
          minlength="10"
          required
        />
      </label>

      {#if mode === 'setup'}
        <label>
          <span>{t($language, '确认密码')}</span>
          <input
            bind:value={confirmPassword}
            type="password"
            autocomplete="new-password"
            minlength="10"
            required
          />
        </label>
      {/if}

      {#if error}
        <p class="error" role="alert">{error}</p>
      {/if}

      <button class="primary" type="submit" disabled={busy}>
        {busy
          ? t($language, '处理中…')
          : mode === 'setup'
            ? t($language, '创建管理员')
            : t($language, '登录')}
      </button>
    </form>

    {#if mode === 'setup'}
      <p class="hint">{t($language, '设置令牌见服务启动日志，或由 FILEHUB_SETUP_TOKEN 指定。')}</p>
    {/if}
  </section>
</main>

<style>
  .auth-shell {
    display: grid;
    min-height: 100dvh;
    place-items: center;
    padding: 24px;
    background: var(--bg);
  }

  .auth-card {
    width: min(100%, 420px);
    padding: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 28px;
    font-size: 17px;
    font-weight: 600;
  }

  .brand-mark img {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    object-fit: cover;
  }

  .brand-mark {
    display: grid;
    width: 24px;
    height: 24px;
    place-items: center;
    border-radius: 7px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 14.5px;
    font-weight: 600;
  }

  .brand-mark.has-icon {
    background: transparent;
  }

  h1 {
    margin: 0 0 6px;
    color: var(--text);
    font-size: 24px;
    font-weight: 500;
    line-height: 1.4;
  }

  .lede {
    margin: 0 0 24px;
    color: var(--text-2);
    font-size: 14.5px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
    color: var(--text-2);
    font-size: 13.5px;
    font-weight: 500;
  }

  input {
    width: 100%;
    height: 40px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    outline: 0;
    background: var(--surface);
    color: var(--text);
    font-size: 16px;
  }

  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .primary {
    height: 40px;
    margin-top: 4px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
  }

  .primary:hover:not(:disabled) {
    filter: brightness(0.96);
  }

  .primary:disabled {
    opacity: 0.6;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13.5px;
  }

  .hint {
    margin: 18px 0 0;
    color: var(--text-2);
    font-size: 12.5px;
    line-height: 1.5;
  }

  @media (max-width: 480px) {
    .auth-shell {
      padding: 16px;
    }

    .auth-card {
      padding: 22px;
    }
  }
</style>
