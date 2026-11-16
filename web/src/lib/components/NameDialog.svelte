<script lang="ts">
  import { language, t } from '../i18n';
  import { focusTrap } from '../focusTrap';

  let {
    open,
    title,
    label,
    initialValue = '',
    inputType = 'text',
    confirmLabel,
    onConfirm,
    onClose,
  }: {
    open: boolean;
    title: string;
    label: string;
    initialValue?: string;
    inputType?: 'text' | 'password';
    confirmLabel?: string;
    onConfirm: (value: string) => void;
    onClose: () => void;
  } = $props();

  let value = $state('');
  let overlayPressed = false;

  $effect(() => {
    if (open) value = initialValue;
  });

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    const next = value.trim();
    if (next) onConfirm(next);
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
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form
      class="name-dialog"
      onsubmit={submit}
      onclick={(event) => event.stopPropagation()}
      use:focusTrap
      onkeydown={(event) => {
        if (event.key === 'Escape') onClose();
      }}
    >
      <h2>{title}</h2>
      <label>
        <span>{label}</span>
        <input bind:value type={inputType} />
      </label>
      <div class="dialog-actions">
        <button type="button" class="ghost" onclick={onClose}>{t($language, '取消')}</button>
        <button type="submit" disabled={!value.trim()}
          >{confirmLabel ?? t($language, '确定')}</button
        >
      </div>
    </form>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .name-dialog {
    width: min(420px, 100%);
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  h2 {
    margin: 0 0 16px;
    font-size: 22px;
    font-weight: 600;
  }

  label {
    display: grid;
    gap: 6px;
    color: var(--text-2);
    font-size: 14px;
  }

  input {
    width: 100%;
    height: 42px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 20px;
  }

  button {
    height: 38px;
    padding: 0 16px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }

  button.ghost {
    border-color: var(--border);
    background: transparent;
    color: var(--text);
  }

  button:disabled {
    opacity: 0.5;
  }
</style>
