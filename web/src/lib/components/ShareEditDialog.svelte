<script lang="ts">
  import { onMount } from 'svelte';
  import type { ShareInfo } from '../api';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';
  let {
    share,
    onClose,
    onSave,
  }: { share: ShareInfo; onClose: () => void; onSave: (readOnly: boolean) => Promise<void> } =
    $props();
  let readOnly = $state(false);
  onMount(() => {
    readOnly = share.read_only;
  });
  let busy = $state(false);
  let error = $state('');
  async function submit(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    busy = true;
    error = '';
    try {
      await onSave(readOnly);
      onClose();
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '保存失败');
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-overlay" role="presentation" onclick={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    class="edit-dialog"
    onsubmit={(event) => void submit(event)}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>{t($language, '编辑分享')}</h2>
      <button type="button" class="icon-button" aria-label={t($language, '关闭')} onclick={onClose}>
        <Icon name="close" size={18} />
      </button>
    </header>
    <label class="row"
      ><span>{t($language, '权限')}</span><select bind:value={readOnly}
        ><option value={false}>{t($language, '读写')}</option><option value={true}
          >{t($language, '只读')}</option
        ></select
      ></label
    >
    {#if error}<p class="error">{error}</p>{/if}
    <footer>
      <button type="button" class="ghost" onclick={onClose}>{t($language, '取消')}</button><button
        type="submit"
        disabled={busy}>{busy ? t($language, '保存中…') : t($language, '保存')}</button
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
  .edit-dialog {
    display: grid;
    width: min(380px, 100%);
    gap: 14px;
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
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-2);
    font-size: 13.5px;
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
