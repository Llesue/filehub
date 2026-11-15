<script lang="ts">
  import type { ServerPath } from '../api';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    paths,
    assigned,
    onClose,
    onAssign,
  }: {
    paths: ServerPath[];
    assigned: string[];
    onClose: () => void;
    onAssign: (path: ServerPath, readOnly: boolean) => Promise<void>;
  } = $props();

  let selected = $state('');
  let readOnly = $state(false);
  let busy = $state(false);
  let error = $state('');

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const path = paths.find((item) => String(item.id) === selected);
    if (!path) {
      error = '请选择外部路径';
      return;
    }
    busy = true;
    error = '';
    try {
      await onAssign(path, readOnly);
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : '分配失败';
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="assign-dialog"
    onsubmit={(event) => void submit(event)}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>分配外部路径</h2>
      <button type="button" class="icon-button" aria-label="关闭" onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    <div class="path-list">
      {#each paths as path (path.id)}
        {@const disabled = assigned.includes(path.real_path)}
        <button
          type="button"
          class="path-row"
          class:selected={selected === String(path.id)}
          {disabled}
          onclick={() => (selected = String(path.id))}
        >
          <span class="path-name">{path.alias}</span>
          <span class="path-real">{path.real_path}</span>
          {#if disabled}<span class="tag">已选择</span>{/if}
        </button>
      {/each}
      {#if paths.length === 0}<p class="muted">还没有外部路径，请先在外部路径区添加。</p>{/if}
    </div>
    <label class="permission"
      >权限
      <select bind:value={readOnly}
        ><option value={false}>读写</option><option value={true}>只读</option></select
      >
    </label>
    {#if error}<p class="error">{error}</p>{/if}
    <footer>
      <button type="button" class="ghost" onclick={onClose}>取消</button><button
        type="submit"
        disabled={busy || !selected}>{busy ? '分配中…' : '分配'}</button
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
  .assign-dialog {
    display: grid;
    width: min(560px, 100%);
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
  .path-list {
    max-height: 300px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }
  .path-row {
    display: grid;
    width: 100%;
    grid-template-columns: 100px minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    min-height: 42px;
    padding: 6px 10px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .path-row:hover:not(:disabled),
  .path-row.selected {
    background: var(--accent-soft);
  }
  .path-row:disabled {
    opacity: 0.55;
  }
  .path-name {
    font-weight: 600;
  }
  .path-real {
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    color: var(--accent-text);
    font-size: 11px;
  }
  .permission {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-2);
    font-size: 13px;
  }
  select {
    height: 34px;
    padding: 0 8px;
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
  .muted {
    margin: 12px;
    color: var(--text-2);
    font-size: 13px;
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
