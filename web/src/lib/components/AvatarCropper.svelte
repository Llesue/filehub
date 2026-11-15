<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    src,
    onConfirm,
    onClose,
  }: {
    src: string;
    onConfirm: (blob: Blob) => void;
    onClose: () => void;
  } = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);
  let image = $state<HTMLImageElement | null>(null);
  let scale = $state(1);
  let offsetX = $state(0);
  let offsetY = $state(0);
  let dragging = false;
  let dragStart = { x: 0, y: 0, offsetX: 0, offsetY: 0 };
  let ready = $state(false);

  const size = 512;

  function draw(): void {
    if (!canvas || !image) return;
    const context = canvas.getContext('2d');
    if (!context) return;
    context.clearRect(0, 0, size, size);
    const cover = Math.max(size / image.naturalWidth, size / image.naturalHeight);
    const width = image.naturalWidth * cover * scale;
    const height = image.naturalHeight * cover * scale;
    context.drawImage(
      image,
      (size - width) / 2 + offsetX,
      (size - height) / 2 + offsetY,
      width,
      height,
    );
  }

  onMount(() => {
    const element = new Image();
    element.onload = () => {
      image = element;
      ready = true;
      draw();
    };
    element.src = src;
  });

  onDestroy(() => URL.revokeObjectURL(src));

  function startDrag(event: PointerEvent): void {
    dragging = true;
    dragStart = { x: event.clientX, y: event.clientY, offsetX, offsetY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function drag(event: PointerEvent): void {
    if (!dragging) return;
    offsetX = dragStart.offsetX + event.clientX - dragStart.x;
    offsetY = dragStart.offsetY + event.clientY - dragStart.y;
    draw();
  }

  function zoom(event: WheelEvent): void {
    event.preventDefault();
    scale = Math.min(3, Math.max(0.5, scale * (event.deltaY < 0 ? 1.1 : 0.9)));
    draw();
  }

  function confirm(): void {
    if (!canvas || !ready) return;
    canvas.toBlob(
      (blob) => {
        if (blob) onConfirm(blob);
      },
      'image/jpeg',
      0.9,
    );
  }
</script>

<div class="crop-overlay" role="presentation" use:overlayGuard={onClose}>
  <section class="crop-dialog" role="presentation" onclick={(event) => event.stopPropagation()}>
    <header>
      <h2>调整头像</h2>
      <button type="button" class="close-button" aria-label="关闭" onclick={onClose}>
        <Icon name="close" size={18} />
      </button>
    </header>
    <div class="crop-stage">
      <canvas
        bind:this={canvas}
        width={size}
        height={size}
        onpointerdown={startDrag}
        onpointermove={drag}
        onpointerup={() => (dragging = false)}
        onpointercancel={() => (dragging = false)}
        onwheel={zoom}
      ></canvas>
    </div>
    <div class="zoom-row">
      <span>缩放</span>
      <input
        type="range"
        min="0.5"
        max="3"
        step="0.01"
        value={scale}
        oninput={(event) => {
          scale = Number(event.currentTarget.value);
          draw();
        }}
      />
    </div>
    <footer>
      <button type="button" class="ghost" onclick={onClose}>取消</button>
      <button type="button" disabled={!ready} onclick={confirm}>使用这张头像</button>
    </footer>
  </section>
</div>

<style>
  .crop-overlay {
    position: fixed;
    inset: 0;
    z-index: 85;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .crop-dialog {
    display: grid;
    width: min(440px, 100%);
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

  .close-button {
    display: grid;
    width: 32px;
    height: 32px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .crop-stage {
    display: grid;
    width: 100%;
    aspect-ratio: 1;
    place-items: center;
    overflow: hidden;
    border-radius: 50%;
    background: var(--surface-2);
  }

  canvas {
    width: 100%;
    height: 100%;
    cursor: grab;
    touch-action: none;
  }

  canvas:active {
    cursor: grabbing;
  }

  .zoom-row {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--text-2);
    font-size: 13px;
  }
  .zoom-row input {
    flex: 1;
    accent-color: var(--accent);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  footer button {
    height: 36px;
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
