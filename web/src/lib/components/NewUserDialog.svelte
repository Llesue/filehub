<script lang="ts">
  import { onMount } from 'svelte';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    defaultQuotaBytes = 15 * 1024 ** 3,
    onClose,
    onSave,
  }: {
    defaultQuotaBytes?: number;
    onClose: () => void;
    onSave: (username: string, password: string, quotaBytes: number) => Promise<void>;
  } = $props();

  let username = $state('');
  let password = $state(randomPassword());
  let quota = $state('');
  let unit = $state('GB');
  onMount(() => {
    quota = String(Math.round(defaultQuotaBytes / 1024 ** 3));
  });
  let busy = $state(false);
  let error = $state('');

  function randomPassword(): string {
    const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%';
    const bytes = crypto.getRandomValues(new Uint8Array(24));
    return Array.from(bytes, (value) => alphabet[value % alphabet.length]).join('');
  }

  function bytes(): number {
    const value = Number(quota);
    const multiplier = unit === 'KB' ? 1024 : unit === 'MB' ? 1024 ** 2 : 1024 ** 3;
    return Math.max(0, value) * multiplier;
  }

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    error = '';
    if (!username.trim() || !password) {
      error = t($language, '用户名和密码不能为空');
      return;
    }
    busy = true;
    try {
      await onSave(username.trim(), password, bytes());
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '创建用户失败');
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="user-dialog"
    onsubmit={(event) => void submit(event)}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>{t($language, '新建用户')}</h2>
      <button type="button" class="icon-button" aria-label={t($language, '关闭')} onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    <label
      ><span>{t($language, '用户名')}</span><input
        bind:value={username}
        autocomplete="off"
      /></label
    >
    <label
      ><span>{t($language, '初始密码（可修改）')}</span><input
        bind:value={password}
        autocomplete="new-password"
      /></label
    >
    <label
      ><span>{t($language, '配额')}</span>
      <div class="quota-row">
        <input bind:value={quota} type="number" min="0" /><select bind:value={unit}
          ><option>KB</option><option>MB</option><option>GB</option><option>TB</option></select
        >
      </div></label
    >
    {#if error}<p class="error">{error}</p>{/if}
    <footer>
      <button type="button" class="ghost" onclick={onClose}>{t($language, '取消')}</button><button
        type="submit"
        disabled={busy}>{busy ? t($language, '创建中…') : t($language, '创建用户')}</button
      >
    </footer>
  </form>
</div>

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }
  .user-dialog {
    display: grid;
    width: min(460px, 100%);
    gap: 12px;
    padding: 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 21px;
  }
  label {
    display: grid;
    gap: 5px;
    color: var(--text-2);
    font-size: 13.5px;
  }
  input,
  select {
    height: 38px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .quota-row {
    display: grid;
    grid-template-columns: 1fr 100px;
    gap: 8px;
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
  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  footer button {
    height: 36px;
    padding: 0 14px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }
  footer button.ghost {
    border-color: var(--border);
    background: transparent;
    color: var(--text);
  }
</style>
