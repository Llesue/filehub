<script lang="ts">
  let {
    open,
    onClose,
    onSave,
  }: {
    open: boolean;
    onClose: () => void;
    onSave: (current: string, next: string) => Promise<void>;
  } = $props();

  let current = $state('');
  let next = $state('');
  let confirm = $state('');
  let error = $state('');
  let busy = $state(false);
  let overlayPressed = false;

  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    error = '';
    if (next !== confirm) {
      error = '两次输入的新密码不一致';
      return;
    }
    busy = true;
    try {
      await onSave(current, next);
      current = '';
      next = '';
      confirm = '';
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : '修改密码失败';
    } finally {
      busy = false;
    }
  }
</script>

{#if open}
  <div
    class="dialog-overlay"
    role="presentation"
    onpointerdown={(event) => (overlayPressed = event.target === event.currentTarget)}
    onclick={(event) => {
      if (overlayPressed && event.target === event.currentTarget) onClose();
      overlayPressed = false;
    }}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="password-dialog"
      onsubmit={(event) => void submit(event)}
      onclick={(event) => event.stopPropagation()}
    >
      <h2>修改密码</h2>
      <label
        ><span>当前密码</span><input
          bind:value={current}
          type="password"
          autocomplete="current-password"
        /></label
      >
      <label
        ><span>新密码</span><input
          bind:value={next}
          type="password"
          autocomplete="new-password"
        /></label
      >
      <label
        ><span>确认新密码</span><input
          bind:value={confirm}
          type="password"
          autocomplete="new-password"
        /></label
      >
      {#if error}<p class="error">{error}</p>{/if}
      <div class="actions">
        <button type="button" class="ghost" onclick={onClose}>取消</button>
        <button type="submit" disabled={busy || !current || !next}
          >{busy ? '更新中…' : '更新密码'}</button
        >
      </div>
    </form>
  </div>
{/if}

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

  .password-dialog {
    display: grid;
    width: min(420px, 100%);
    gap: 12px;
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  h2 {
    margin: 0 0 4px;
    font-size: 22px;
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

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }

  .actions button {
    height: 36px;
    padding: 0 14px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }

  .actions button.ghost {
    border-color: var(--border);
    background: transparent;
    color: var(--text);
  }
</style>
