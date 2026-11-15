<script lang="ts">
  import { language, t } from '../i18n';
  import { clearNotice, notice } from '../notices';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  function runAction(): void {
    const action = $notice?.action;
    clearNotice();
    action?.();
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if ($notice && event.key === 'Enter') {
      event.preventDefault();
      if ($notice.action) runAction();
      else clearNotice();
    }
  }}
/>

{#if $notice}
  <div class="notice-overlay" role="presentation" use:overlayGuard={clearNotice}>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <section
      class={`notice-dialog ${$notice.kind}`}
      role="alertdialog"
      aria-modal="true"
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
    >
      <header>
        <strong>{$notice.title}</strong>
        <button
          type="button"
          class="icon-button"
          aria-label={t($language, '关闭')}
          onclick={clearNotice}
        >
          <Icon name="close" size={18} />
        </button>
      </header>
      <p>{$notice.message}</p>
      <footer>
        {#if $notice.action}
          <button type="button" class="secondary" onclick={clearNotice}
            >{t($language, '取消')}</button
          >
          <button type="button" onclick={runAction}
            >{$notice.actionLabel ?? t($language, '确定')}</button
          >
        {:else}
          <button type="button" onclick={clearNotice}>{t($language, '知道了')}</button>
        {/if}
      </footer>
    </section>
  </div>
{/if}

<style>
  .notice-overlay {
    position: fixed;
    inset: 0;
    z-index: 90;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .notice-dialog {
    width: min(460px, 100%);
    padding: 20px;
    border: 1px solid var(--border);
    border-top: 4px solid var(--accent);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .notice-dialog.error {
    border-top-color: var(--danger);
  }

  .notice-dialog.warning {
    border-top-color: var(--file-slide);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  header strong {
    font-size: 17px;
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

  p {
    margin: 14px 0 20px;
    color: var(--text-2);
    font-size: 14.5px;
    line-height: 1.6;
    overflow-wrap: anywhere;
  }

  footer {
    display: flex;
    justify-content: flex-end;
  }

  footer {
    gap: 8px;
  }

  footer button.secondary {
    border-color: var(--border);
    background: transparent;
    color: var(--text);
  }

  footer button {
    height: 36px;
    padding: 0 16px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }
</style>
