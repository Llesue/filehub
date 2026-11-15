<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';
  let {
    value,
    onClose,
    onSave,
  }: {
    value: number | null;
    onClose: () => void;
    onSave: (bytes: number | null) => Promise<void>;
  } = $props();
  let amount = $state<string | number>('');
  let unit = $state('GB');
  onMount(() => {
    amount = value ? String(Math.round(value / 1024 ** 3)) : '15';
  });
  let busy = $state(false);
  let error = $state('');
  function bytes(): number | null {
    const raw = String(amount).trim();
    if (!raw) return null;
    const value = Number(raw);
    if (!Number.isFinite(value)) throw new Error('请输入有效的配额数字');
    return (
      Math.max(0, value) *
      (unit === 'KB' ? 1024 : unit === 'MB' ? 1024 ** 2 : unit === 'GB' ? 1024 ** 3 : 1024 ** 4)
    );
  }
  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      await onSave(bytes());
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : '保存失败';
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="quota-dialog"
    onsubmit={(event) => void submit(event)}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>配置配额</h2>
      <button type="button" class="icon-button" aria-label="关闭" onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    <p class="hint">留空表示不限制；只统计用户 home 目录。</p>
    <div class="quota-row">
      <input bind:value={amount} type="number" min="0" placeholder="不限制" /><select
        bind:value={unit}
        ><option>KB</option><option>MB</option><option>GB</option><option>TB</option></select
      >
    </div>
    {#if error}<p class="error">{error}</p>{/if}
    <footer>
      <button type="button" class="ghost" onclick={onClose}>取消</button><button
        type="submit"
        disabled={busy}>{busy ? '保存中…' : '保存'}</button
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
  .quota-dialog {
    display: grid;
    width: min(400px, 100%);
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
    font-size: 20px;
  }
  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 13px;
  }
  .quota-row {
    display: grid;
    grid-template-columns: 1fr 100px;
    gap: 8px;
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
  }
  footer button {
    height: 34px;
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
