<script lang="ts">
  import { onMount } from 'svelte';
  import { language, t } from '../i18n';
  import { overlayGuard } from '../overlayGuard';
  import Icon from './Icon.svelte';

  let { url, title, onClose }: { url: string; title: string; onClose: () => void } = $props();
  let dataUrl = $state('');
  let failed = $state(false);
  let cardBusy = $state(false);

  function roundRect(
    ctx: CanvasRenderingContext2D,
    x: number,
    y: number,
    width: number,
    height: number,
    radius: number,
  ): void {
    ctx.beginPath();
    ctx.moveTo(x + radius, y);
    ctx.arcTo(x + width, y, x + width, y + height, radius);
    ctx.arcTo(x + width, y + height, x, y + height, radius);
    ctx.arcTo(x, y + height, x, y, radius);
    ctx.arcTo(x, y, x + width, y, radius);
    ctx.closePath();
  }

  function wrapText(
    ctx: CanvasRenderingContext2D,
    text: string,
    x: number,
    y: number,
    maxWidth: number,
    lineHeight: number,
    maxLines: number,
  ): number {
    let line = '';
    let lines = 0;
    for (const char of text) {
      const next = line + char;
      if (ctx.measureText(next).width > maxWidth && line) {
        ctx.fillText(line, x, y + lines * lineHeight);
        lines += 1;
        if (lines >= maxLines) return lines;
        line = char;
      } else {
        line = next;
      }
    }
    if (line && lines < maxLines) {
      ctx.fillText(line, x, y + lines * lineHeight);
      lines += 1;
    }
    return lines;
  }

  async function saveCard(): Promise<void> {
    if (!dataUrl || cardBusy) return;
    cardBusy = true;
    try {
      const canvas = document.createElement('canvas');
      canvas.width = 1080;
      canvas.height = 1440;
      const ctx = canvas.getContext('2d');
      if (!ctx) return;
      const accent =
        getComputedStyle(document.documentElement).getPropertyValue('--accent').trim() || '#2563eb';
      const font = '"Inter", "Noto Sans SC", system-ui, -apple-system, "Segoe UI", sans-serif';

      const bg = ctx.createLinearGradient(0, 0, 0, canvas.height);
      bg.addColorStop(0, '#eef4ff');
      bg.addColorStop(0.55, '#f8fbff');
      bg.addColorStop(1, '#ffffff');
      ctx.fillStyle = bg;
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      const glow = ctx.createRadialGradient(930, 90, 30, 930, 90, 480);
      glow.addColorStop(0, 'rgba(37, 99, 235, 0.20)');
      glow.addColorStop(1, 'rgba(37, 99, 235, 0)');
      ctx.fillStyle = glow;
      ctx.fillRect(0, 0, canvas.width, canvas.height);

      roundRect(ctx, 96, 96, 84, 84, 22);
      ctx.fillStyle = accent;
      ctx.fill();
      ctx.fillStyle = '#ffffff';
      ctx.font = `700 46px ${font}`;
      ctx.textBaseline = 'alphabetic';
      ctx.fillText('F', 123, 156);

      ctx.fillStyle = '#111827';
      ctx.font = `600 38px ${font}`;
      ctx.fillText('FileHub', 206, 146);
      ctx.fillStyle = '#6b7280';
      ctx.font = `400 22px ${font}`;
      ctx.fillText(t($language, '自托管文件管理器'), 206, 180);

      ctx.fillStyle = '#111827';
      ctx.font = `600 46px ${font}`;
      const titleLines = wrapText(ctx, title, 96, 300, 888, 60, 2);

      const qrTop = titleLines > 1 ? 440 : 410;
      roundRect(ctx, 110, qrTop, 860, 740, 32);
      ctx.fillStyle = '#ffffff';
      ctx.fill();
      ctx.strokeStyle = '#e5e7eb';
      ctx.lineWidth = 2;
      ctx.stroke();

      const image = new Image();
      image.src = dataUrl;
      await image.decode();
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(image, 220, qrTop + 50, 640, 640);

      ctx.fillStyle = '#374151';
      ctx.font = `400 24px ${font}`;
      wrapText(ctx, url, 96, qrTop + 810, 888, 34, 2);

      ctx.fillStyle = '#9ca3af';
      ctx.font = `400 22px ${font}`;
      ctx.fillText(t($language, '扫码打开'), 96, 1360);
      ctx.textAlign = 'right';
      ctx.fillText('filehub', 984, 1360);
      ctx.textAlign = 'left';

      const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
      if (!blob) return;
      const link = document.createElement('a');
      const name = (title || 'filehub-share').replace(/[\\/:*?"<>|]+/g, '-').slice(0, 60);
      link.href = URL.createObjectURL(blob);
      link.download = `${name}.png`;
      link.click();
      window.setTimeout(() => URL.revokeObjectURL(link.href), 5_000);
    } catch (error) {
      console.warn('share card generation failed', error);
    } finally {
      cardBusy = false;
    }
  }

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
      <button type="button" class="card-button" disabled={cardBusy} onclick={() => void saveCard()}>
        {cardBusy ? t($language, '生成中…') : t($language, '保存分享卡片')}
      </button>
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

  .card-button {
    width: 100%;
    padding: 12px 16px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent);
    color: #fff;
    font-weight: 600;
  }

  .card-button:disabled {
    opacity: 0.6;
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
