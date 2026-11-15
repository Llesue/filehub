<script lang="ts">
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';
  let {
    onClose,
    onSave,
  }: { onClose: () => void; onSave: (path: string, alias: string) => Promise<void> } = $props();
  let path = $state('');
  let alias = $state('');
  let busy = $state(false);
  let error = $state('');
  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      await onSave(path.trim(), alias.trim());
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : '添加失败';
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="path-dialog"
    onsubmit={(event) => void submit(event)}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>添加外部路径</h2>
      <button type="button" class="icon-button" aria-label="关闭" onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    <label><span>服务器目录</span><input bind:value={path} placeholder="/mnt/red" /></label>
    <label><span>别名</span><input bind:value={alias} placeholder="红盘" /></label>
    <p class="hint">目录必须已存在且可读；添加后到用户卡片的“分配外部路径”中授权。</p>
    {#if error}<p class="error">{error}</p>{/if}
    <footer>
      <button type="button" class="ghost" onclick={onClose}>取消</button><button
        type="submit"
        disabled={busy || !path.trim() || !alias.trim()}>{busy ? '添加中…' : '添加'}</button
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
  .path-dialog {
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
    font-size: 20px;
  }
  label {
    display: grid;
    gap: 5px;
    color: var(--text-2);
    font-size: 13.5px;
  }
  input {
    height: 38px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12.5px;
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
