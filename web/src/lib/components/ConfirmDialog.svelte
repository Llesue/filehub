<script lang="ts">
  import { language, t } from '../i18n';
  import { focusTrap } from '../focusTrap';
  import FileName from './FileName.svelte';

  let {
    open,
    title,
    message,
    items = [],
    confirmLabel,
    danger = false,
    requireText = null,
    busy = false,
    onConfirm,
    onCancel,
  }: {
    open: boolean;
    title: string;
    message: string;
    items?: { name: string; isDir: boolean }[];
    confirmLabel?: string;
    danger?: boolean;
    requireText?: string | null;
    busy?: boolean;
    onConfirm: (value: string) => void;
    onCancel: () => void;
  } = $props();

  let shownItems = $derived(items.slice(0, 12));
  let hiddenCount = $derived(Math.max(0, items.length - shownItems.length));

  let input = $state('');
  let overlayPressed = false;

  $effect(() => {
    if (!open) input = '';
  });
</script>

{#if open}
  <div
    class="confirm-overlay"
    role="presentation"
    onpointerdown={(event) => (overlayPressed = event.target === event.currentTarget)}
    onclick={(event) => {
      if (overlayPressed && event.target === event.currentTarget) onCancel();
      overlayPressed = false;
    }}
  >
    <dialog
      open
      class="confirm-panel"
      aria-labelledby="confirm-title"
      onclick={(event) => event.stopPropagation()}
      use:focusTrap
      onkeydown={(event) => {
        if (event.key === 'Escape') onCancel();
      }}
    >
      <form
        onsubmit={(event) => {
          event.preventDefault();
          if (!busy && (requireText === null || input === requireText)) onConfirm(input);
        }}
      >
        <h2 id="confirm-title">{title}</h2>
        <p class="message">
          {#each message.split('\n') as line, index (index)}
            <span class="line"
              >{#if line.startsWith('× ')}<span class="delete-x">×</span>{line.slice(
                  1,
                )}{:else}{line || ' '}{/if}</span
            >
          {/each}
        </p>
        {#if shownItems.length > 0}
          <ul class="confirm-items">
            {#each shownItems as item (item.name)}
              <li>
                <span class="delete-x">×</span>
                <FileName name={item.name} isDir={item.isDir} />
              </li>
            {/each}
          </ul>
          {#if hiddenCount > 0}
            <p class="more">{t($language, '…以及另外 {count} 项', { count: hiddenCount })}</p>
          {/if}
        {/if}
        {#if requireText}
          <input
            bind:value={input}
            placeholder={t($language, '输入 {code} 确认', { code: requireText })}
          />
        {/if}
        <div class="confirm-actions">
          <button type="button" onclick={onCancel} disabled={busy}>{t($language, '取消')}</button>
          <button
            type="submit"
            class:danger
            disabled={busy || (requireText !== null && input !== requireText)}
          >
            {busy ? t($language, '处理中…') : (confirmLabel ?? t($language, '确认'))}
          </button>
        </div>
      </form>
    </dialog>
  </div>
{/if}

<style>
  .confirm-overlay {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .confirm-panel {
    width: min(420px, 100%);
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-2);
  }

  h2 {
    margin: 0 0 8px;
    font-size: 20px;
    font-weight: 500;
  }

  .message {
    display: flex;
    max-height: 260px;
    flex-direction: column;
    gap: 2px;
    overflow: auto;
    margin: 0 0 16px;
    color: var(--text-2);
    font-size: 14.5px;
    line-height: 1.6;
  }

  .line {
    overflow-wrap: anywhere;
  }

  .confirm-items {
    display: flex;
    max-height: 260px;
    flex-direction: column;
    gap: 2px;
    overflow: auto;
    margin: 0 0 8px;
    padding: 0;
    color: var(--text-2);
    font-size: 14.5px;
    line-height: 1.6;
    list-style: none;
  }

  .confirm-items li {
    display: flex;
    align-items: baseline;
    min-width: 0;
    white-space: nowrap;
  }

  .more {
    margin: 0 0 16px;
    color: var(--text-2);
    font-size: 14px;
  }

  .delete-x {
    margin-right: 6px;
    color: var(--danger);
    font-weight: 700;
  }

  input {
    width: 100%;
    height: 38px;
    margin-bottom: 14px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .confirm-actions button {
    height: 34px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
  }

  .confirm-actions button.danger {
    border-color: var(--danger);
    background: var(--danger);
    color: var(--on-accent);
  }

  .confirm-actions button:disabled {
    opacity: 0.55;
  }
</style>
