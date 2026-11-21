<script lang="ts">
  import { onMount } from 'svelte';
  import { language, t } from '../i18n';
  import { overlayGuard } from '../overlayGuard';
  import Icon from './Icon.svelte';

  let { url, title, onClose }: { url: string; title: string; onClose: () => void } = $props();
  let dataUrl = $state('');
  let failed = $state(false);

  onMount(() => {
    let cancelled = false;
    type QrEncoder = {
      toDataURL: (text: string, options: { width: number; margin: number }) => Promise<string>;
    };
    void import('qrcode')
      .then((module) => {
        const encoder = ((module as { default?: QrEncoder }).default ?? module) as QrEncoder;
        return encoder.toDataURL(url, { width: 320, margin: 1 });
      })
      .then((value) => {
        if (!cancelled) dataUrl = value;
      })
      .catch((error) => {
        if (!cancelled) {
          failed = true;
          console.warn('QR generation failed', error);
        }
      });
    return () => {
      cancelled = true;
    };
  });
</script>

<div class="qr-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <section
    class="qr-dialog"
    role="dialog"
    aria-modal="true"
    tabindex="-1"
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <strong>{title}</strong>
      <button
        type="button"
        class="icon-button"
        aria-label={t($language, '关闭')}
        onclick={onClose}
      >
        <Icon name="close" size={18} />
      </button>
    </header>
    {#if dataUrl}
      <img class="qr-image" src={dataUrl} alt={t($language, '分享二维码')} />
    {:else if failed}
      <p class="qr-message">{t($language, '二维码生成失败')}</p>
    {:else}
      <p class="qr-message">{t($language, '生成中…')}</p>
    {/if}
    <code class="qr-url">{url}</code>
  </section>
</div>

<style>
  .qr-overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 24px;
    padding-bottom: max(24px, env(safe-area-inset-bottom, 24px));
    background: var(--scrim);
    overscroll-behavior: contain;
  }

  .qr-dialog {
    display: flex;
    width: min(360px, 100%);
    max-height: min(85dvh, 720px);
    flex-direction: column;
    gap: 12px;
    padding: 18px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  header strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 17px;
  }

  .icon-button {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .qr-image {
    width: 100%;
    height: auto;
    background: #fff;
    border-radius: var(--radius-sm);
  }

  .qr-message {
    margin: 0;
    color: var(--text-2);
    font-size: 13px;
    text-align: center;
  }

  .qr-url {
    overflow-wrap: anywhere;
    color: var(--text-2);
    font-size: 12px;
  }
</style>
