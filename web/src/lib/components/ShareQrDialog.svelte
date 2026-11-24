<script lang="ts">
  import { onMount } from 'svelte';
  import { branding } from '../branding';
  import { copyText } from '../clipboard';
  import { classifyFile, type FileKind } from '../fileKind';
  import { language, t } from '../i18n';
  import { overlayGuard } from '../overlayGuard';
  import Icon from './Icon.svelte';

  let {
    url,
    title,
    expiresAt = null,
    isDir = false,
    isCollection = false,
    onClose,
  }: {
    url: string;
    title: string;
    expiresAt?: number | null;
    isDir?: boolean;
    isCollection?: boolean;
    onClose: () => void;
  } = $props();

  let cardUrl = $state('');
  let failed = $state(false);

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

  function wrapLines(
    ctx: CanvasRenderingContext2D,
    text: string,
    maxWidth: number,
    maxLines: number,
  ): string[] {
    let lines: string[] = [];
    let line = '';
    for (const char of text) {
      const next = line + char;
      if (ctx.measureText(next).width > maxWidth && line) {
        lines.push(line);
        line = char;
      } else {
        line = next;
      }
    }
    if (line) lines.push(line);
    if (lines.length > maxLines) {
      lines = lines.slice(0, maxLines);
      const last = lines[maxLines - 1] ?? '';
      lines[maxLines - 1] = `${last.slice(0, -1)}…`;
    }
    return lines;
  }

  type TileStyle = { background: string; foreground: string; label: string };

  function tileStyle(kind: FileKind, name: string): TileStyle {
    const extension = name.includes('.') ? name.split('.').pop()!.slice(0, 4).toUpperCase() : 'FILE';
    switch (kind) {
      case 'folder':
        return { background: '#dbeafe', foreground: '#1d4ed8', label: 'DIR' };
      case 'image':
        return { background: '#dcfce7', foreground: '#15803d', label: extension };
      case 'video':
        return { background: '#ede9fe', foreground: '#6d28d9', label: extension };
      case 'audio':
        return { background: '#ffedd5', foreground: '#c2410c', label: extension };
      case 'pdf':
        return { background: '#fee2e2', foreground: '#b91c1c', label: 'PDF' };
      case 'text':
        return { background: '#f1f5f9', foreground: '#334155', label: extension };
      default:
        return { background: '#fef3c7', foreground: '#b45309', label: extension };
    }
  }

  async function renderCard(qr: string): Promise<string> {
    const canvas = document.createElement('canvas');
    canvas.width = 1080;
    canvas.height = 1600;
    const ctx = canvas.getContext('2d');
    if (!ctx) return '';
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
    glow.addColorStop(0, 'rgba(37, 99, 235, 0.18)');
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

    const kind: FileKind = isCollection
      ? 'folder'
      : isDir
        ? 'folder'
        : classifyFile(title);
    const tile = tileStyle(kind, title);
    roundRect(ctx, 96, 280, 108, 108, 26);
    ctx.fillStyle = tile.background;
    ctx.fill();
    ctx.strokeStyle = tile.foreground;
    ctx.fillStyle = tile.foreground;
    ctx.lineWidth = 5;
    if (kind === 'folder' || isCollection) {
      roundRect(ctx, 122, 306, 34, 16, 6);
      ctx.fill();
      roundRect(ctx, 118, 316, 64, 44, 10);
      ctx.fill();
    } else {
      ctx.beginPath();
      ctx.moveTo(128, 294);
      ctx.lineTo(158, 294);
      ctx.lineTo(178, 314);
      ctx.lineTo(178, 344);
      ctx.lineTo(128, 344);
      ctx.closePath();
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(158, 294);
      ctx.lineTo(158, 314);
      ctx.lineTo(178, 314);
      ctx.stroke();
    }
    if (kind !== 'folder' && !isCollection) {
      ctx.font = `700 ${tile.label.length > 3 ? 18 : 21}px ${font}`;
      ctx.textAlign = 'center';
      ctx.fillText(tile.label, 150, 374);
      ctx.textAlign = 'left';
    }

    ctx.fillStyle = '#111827';
    ctx.font = `600 44px ${font}`;
    const lines = wrapLines(ctx, title, 724, 2);
    lines.forEach((line, index) => ctx.fillText(line, 236, 330 + index * 58));

    const cardX = 90;
    const cardY = 440;
    roundRect(ctx, cardX, cardY, 900, 900, 40);
    ctx.fillStyle = '#ffffff';
    ctx.fill();
    ctx.strokeStyle = '#e5e7eb';
    ctx.lineWidth = 2;
    ctx.stroke();

    const image = new Image();
    image.src = qr;
    await image.decode();
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(image, cardX + 80, cardY + 80, 740, 740);

    ctx.textAlign = 'center';
    ctx.fillStyle = '#111827';
    ctx.font = `600 44px ${font}`;
    ctx.fillText(t($language, '扫码打开'), 540, 1458);
    ctx.fillStyle = '#9ca3af';
    ctx.font = `400 34px ${font}`;
    const expiry = expiresAt
      ? `${t($language, '有效期至')} ${new Date(expiresAt * 1000).toLocaleDateString()}`
      : t($language, '长期有效');
    ctx.fillText(expiry, 540, 1512);
    ctx.textAlign = 'left';

    return canvas.toDataURL('image/png');
  }

  onMount(() => {
    let cancelled = false;
    type QrEncoder = {
      toDataURL: (text: string, options: { width: number; margin: number }) => Promise<string>;
    };
    void import('qrcode')
      .then((module) => {
        const encoder = ((module as { default?: QrEncoder }).default ?? module) as QrEncoder;
        return encoder.toDataURL(url, { width: 740, margin: 1 });
      })
      .then(async (value) => {
        if (cancelled) return;
        cardUrl = await renderCard(value);
        if (!cardUrl) failed = true;
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

  async function copyLink(): Promise<void> {
    const text = `${t($language, '{brand} 给你分享了文件', { brand: $branding.name })}\n${url}`;
    await copyText(text);
  }

  function cardFileName(): string {
    const base = (title || 'filehub-share').replace(/[\\/:*?"<>|]+/g, '-').slice(0, 60);
    return `${base}.png`;
  }
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
      <strong>{t($language, '分享卡片')}</strong>
      <button
        type="button"
        class="icon-button"
        aria-label={t($language, '关闭')}
        onclick={onClose}
      >
        <Icon name="close" size={18} />
      </button>
    </header>
    {#if cardUrl}
      <img class="card-image" src={cardUrl} alt={t($language, '分享卡片')} />
      <div class="card-actions">
        <a class="primary" href={cardUrl} download={cardFileName()}>
          {t($language, '下载分享图片')}
        </a>
        <button type="button" class="secondary" onclick={() => void copyLink()}>
          {t($language, '复制链接')}
        </button>
      </div>
    {:else if failed}
      <p class="qr-message">{t($language, '二维码生成失败')}</p>
    {:else}
      <p class="qr-message">{t($language, '生成中…')}</p>
    {/if}
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
    width: min(420px, 100%);
    max-height: min(88dvh, 820px);
    flex-direction: column;
    gap: 14px;
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

  .card-image {
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: #fff;
  }

  .card-actions {
    display: flex;
    gap: 10px;
  }

  .card-actions a,
  .card-actions button {
    flex: 1 1 0;
    display: grid;
    place-items: center;
    min-height: 40px;
    padding: 10px 14px;
    border-radius: var(--radius-sm);
    font-size: 14px;
    font-weight: 600;
    text-decoration: none;
  }

  .card-actions .primary {
    background: var(--accent);
    color: #fff;
  }

  .card-actions .secondary {
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }

  .qr-message {
    margin: 0;
    color: var(--text-2);
    text-align: center;
  }
</style>
