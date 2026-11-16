<script lang="ts">
  import { api, ApiError } from '../api';
  import Icon from './Icon.svelte';
  import { copyText } from '../clipboard';
  import { language, t } from '../i18n';
  import { overlayGuard } from '../overlayGuard';

  let {
    root,
    paths,
    name,
    isDir,
    csrf,
    onClose,
  }: {
    root: number;
    paths: string[];
    name: string;
    isDir: boolean;
    csrf: string;
    onClose: () => void;
  } = $props();

  let password = $state('');
  let expiresDays = $state('0');
  let readOnlyOverride = $state<boolean | null>(null);
  const readOnly = $derived(readOnlyOverride ?? (isDir || paths.length > 1));
  let error = $state('');
  let busy = $state(false);
  let created = $state<{ url: string; token: string } | null>(null);

  async function create(): Promise<void> {
    busy = true;
    error = '';
    try {
      const days = Number(expiresDays);
      const expires_at = days > 0 ? Math.floor(Date.now() / 1000) + days * 86400 : null;
      const result = await api.createShare(
        {
          root,
          path: paths[0],
          paths: paths.length > 1 ? paths : undefined,
          password: password.trim() || undefined,
          expires_at,
          read_only: readOnly,
        },
        csrf,
      );
      created = { url: result.url, token: result.token };
    } catch (err) {
      error = err instanceof ApiError ? err.message : t($language, '创建分享失败');
    } finally {
      busy = false;
    }
  }

  async function copyLink(): Promise<void> {
    if (!created) return;
    await copyText(`${window.location.origin}${created.url}`);
  }
</script>

<div class="share-overlay" role="presentation" use:overlayGuard={onClose}>
  <dialog
    open
    class="share-panel"
    aria-labelledby="share-title"
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <div>
        <h2 id="share-title">{t($language, '分享')}</h2>
        <span>{name}</span>
      </div>
      <button type="button" class="icon-button" aria-label={t($language, '关闭')} onclick={onClose}>
        <Icon name="close" size={20} />
      </button>
    </header>

    {#if created}
      <div class="created">
        <p>{t($language, '分享已创建，链接只显示一次：')}</p>
        <code>{window.location.origin}{created.url}</code>
        <div class="actions">
          <button type="button" onclick={() => void copyLink()}>{t($language, '复制链接')}</button>
          <button type="button" class="primary" onclick={onClose}>{t($language, '完成')}</button>
        </div>
      </div>
    {:else}
      <form
        onsubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <label>
          <span>{t($language, '密码（可选）')}</span>
          <input
            bind:value={password}
            type="password"
            placeholder={t($language, '留空表示无密码')}
          />
        </label>
        <label>
          <span>{t($language, '有效期')}</span>
          <select bind:value={expiresDays}>
            <option value="0">{t($language, '永久')}</option>
            <option value="1">{t($language, '1 天')}</option>
            <option value="7">{t($language, '7 天')}</option>
            <option value="30">{t($language, '30 天')}</option>
          </select>
        </label>
        {#if isDir && paths.length === 1}
          <label class="check">
            <input
              type="checkbox"
              checked={readOnly}
              onchange={(event) => (readOnlyOverride = event.currentTarget.checked)}
            />
            {t($language, '只读分享')}
          </label>
        {/if}
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <div class="actions">
          <button type="button" onclick={onClose}>{t($language, '取消')}</button>
          <button type="submit" class="primary" disabled={busy}
            >{busy ? t($language, '创建中…') : t($language, '创建分享')}</button
          >
        </div>
      </form>
    {/if}
  </dialog>
</div>

<style>
  .share-overlay {
    position: fixed;
    inset: 0;
    z-index: 65;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .share-panel {
    width: min(460px, 100%);
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-2);
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    margin-bottom: 18px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: 20px;
    font-weight: 500;
  }

  header span {
    color: var(--text-2);
    font-size: 13.5px;
  }

  form {
    display: flex;
    flex-direction: column;
    gap: 12px;
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

  .check {
    flex-direction: row;
    align-items: center;
  }

  .check input {
    width: auto;
    height: auto;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 6px;
  }

  .actions button {
    height: 34px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .actions .primary {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }

  .created p {
    margin: 0 0 8px;
    color: var(--text-2);
    font-size: 13.5px;
  }

  code {
    display: block;
    overflow-wrap: anywhere;
    padding: 10px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12.5px;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13.5px;
  }

  .icon-button {
    display: grid;
    width: 34px;
    height: 34px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }
</style>
