<script lang="ts">
  import { onDestroy } from 'svelte';
  import { api, type Entry } from '../api';
  import { audioState, setAudioPlaying } from '../audio';
  import Icon from './Icon.svelte';
  import { language, t } from '../i18n';

  let {
    root,
    currentPath,
    items,
    index,
    onClose,
    onIndexChange,
    srcFor,
    thumbSrcFor,
    downloadHref,
  }: {
    root: number;
    currentPath: string;
    items: Entry[];
    index: number;
    onClose: () => void;
    onIndexChange: (index: number) => void;
    srcFor?: (entry: Entry) => string;
    thumbSrcFor?: (entry: Entry) => string;
    downloadHref?: (entry: Entry) => string;
  } = $props();

  let current = $derived(items[index]);
  let zoom = $state(1);
  let offsetX = $state(0);
  let offsetY = $state(0);
  let showOriginal = $state(false);
  let imageFallback = $state(false);
  let previewFailed = $state(false);
  let hiResLoaded = $state(false);
  let blurVisible = $state(true);
  let frameRatio = $state(1);
  let frameReady = $state(false);
  let hideBlurTimer: ReturnType<typeof setTimeout> | null = null;
  let dragging = false;
  let dragged = false;
  let dragStart = { x: 0, y: 0, offsetX: 0, offsetY: 0 };
  let videoWasOpen = false;
  let resumeAudio = false;
  let videoVolume = 0.8;
  let videoMuted = false;
  try {
    const rawVolume = localStorage.getItem('filehub.video.volume');
    const storedVolume = rawVolume === null ? Number.NaN : Number(rawVolume);
    if (Number.isFinite(storedVolume) && storedVolume >= 0 && storedVolume <= 1) {
      videoVolume = storedVolume;
    }
    videoMuted = localStorage.getItem('filehub.video.muted') === '1';
  } catch (error) {
    console.warn('could not read video volume', error);
  }

  function applyVideoVolume(event: Event): void {
    const video = event.currentTarget as HTMLVideoElement;
    video.volume = videoVolume;
    video.muted = videoMuted;
  }

  function rememberVideoVolume(event: Event): void {
    const video = event.currentTarget as HTMLVideoElement;
    videoVolume = video.volume;
    videoMuted = video.muted;
    try {
      localStorage.setItem('filehub.video.volume', String(videoVolume));
      localStorage.setItem('filehub.video.muted', videoMuted ? '1' : '0');
    } catch (error) {
      console.warn('could not persist video volume', error);
    }
  }

  $effect(() => {
    const videoOpen = current ? isVideo(current.name) : false;
    if (videoOpen && !videoWasOpen) {
      if ($audioState?.playing) {
        resumeAudio = true;
        setAudioPlaying(false);
      }
    } else if (!videoOpen && videoWasOpen && resumeAudio) {
      resumeAudio = false;
      if ($audioState) setAudioPlaying(true);
    }
    videoWasOpen = videoOpen;
  });

  onDestroy(() => {
    clearBlurTimer();
    if (resumeAudio) setAudioPlaying(true);
  });
  let entryPath = $derived(
    current ? (currentPath ? `${currentPath}/${current.name}` : current.name) : '',
  );
  let url = $derived(current ? (srcFor ? srcFor(current) : api.previewUrl(root, entryPath)) : '');
  let lowResUrl = $derived(
    current
      ? thumbSrcFor
        ? thumbSrcFor(current)
        : api.thumbUrl(root, entryPath, 320, undefined, true)
      : '',
  );
  let previewSrc = $derived(
    current
      ? showOriginal || imageFallback
        ? url
        : thumbSrcFor
          ? thumbSrcFor(current)
          : api.previewThumbUrl(root, entryPath, 2048)
      : '',
  );

  function clearBlurTimer(): void {
    if (hideBlurTimer !== null) {
      clearTimeout(hideBlurTimer);
      hideBlurTimer = null;
    }
  }

  function noteAspect(event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (image.naturalWidth > 0 && image.naturalHeight > 0) {
      frameRatio = image.naturalWidth / image.naturalHeight;
      frameReady = true;
    }
  }

  function handleHiResLoad(event: Event): void {
    noteAspect(event);
    hiResLoaded = true;
    clearBlurTimer();
    // Keep the blurred layer under the 80ms fade, then remove it completely.
    hideBlurTimer = setTimeout(() => {
      blurVisible = false;
      hideBlurTimer = null;
    }, 100);
  }

  function handleHiResError(): void {
    clearBlurTimer();
    hiResLoaded = false;
    blurVisible = true;
    if (showOriginal || imageFallback) previewFailed = true;
    else imageFallback = true;
  }

  function resetView(): void {
    clearBlurTimer();
    zoom = 1;
    offsetX = 0;
    offsetY = 0;
    showOriginal = false;
    imageFallback = false;
    previewFailed = false;
    hiResLoaded = false;
    blurVisible = true;
  }

  function previous(): void {
    if (index > 0) {
      resetView();
      onIndexChange(index - 1);
    }
  }

  function next(): void {
    if (index < items.length - 1) {
      resetView();
      onIndexChange(index + 1);
    }
  }

  function startDrag(event: PointerEvent): void {
    event.preventDefault();
    dragging = true;
    dragged = true;
    dragStart = { x: event.clientX, y: event.clientY, offsetX, offsetY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function drag(event: PointerEvent): void {
    if (!dragging) return;
    offsetX = dragStart.offsetX + event.clientX - dragStart.x;
    offsetY = dragStart.offsetY + event.clientY - dragStart.y;
  }

  function endDrag(): void {
    dragging = false;
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') onClose();
    if (event.key === 'ArrowLeft') previous();
    if (event.key === 'ArrowRight') next();
    if (event.key === '0') resetView();
  }

  function zoomBy(event: WheelEvent): void {
    if (!current || isVideo(current.name) || isAudio(current.name) || isPdf(current.name)) return;
    event.preventDefault();
    const factor = event.deltaY < 0 ? 1.15 : 0.87;
    zoom = Math.min(8, Math.max(0.2, zoom * factor));
  }

  function isVideo(name: string): boolean {
    return /\.(mp4|mov|mkv|webm|avi|m4v)$/i.test(name);
  }

  function isAudio(name: string): boolean {
    return /\.(mp3|wav|ogg|m4a|flac|aac)$/i.test(name);
  }

  function isPdf(name: string): boolean {
    return /\.pdf$/i.test(name);
  }
</script>

<svelte:window onkeydown={onKeydown} onpointermove={drag} onpointerup={endDrag} />

<div class="lightbox" role="presentation" onclick={onClose}>
  <header class="lightbox-bar" role="presentation" onclick={(event) => event.stopPropagation()}>
    <strong>{current?.name ?? ''}</strong>
    <div class="lightbox-actions">
      {#if current && !isVideo(current.name) && !isAudio(current.name) && !isPdf(current.name)}
        <button type="button" onclick={() => (zoom = Math.max(0.2, zoom - 0.2))}>−</button>
        <span>{Math.round(zoom * 100)}%</span>
        <button type="button" onclick={() => (zoom = Math.min(8, zoom + 0.2))}>+</button>
        <button type="button" onclick={() => resetView()}>{t($language, '复位')}</button>
        <button
          type="button"
          onclick={() => {
            showOriginal = !showOriginal;
            hiResLoaded = false;
            blurVisible = false;
            clearBlurTimer();
          }}
        >
          {showOriginal ? t($language, '显示预览图') : t($language, '显示原图')}
        </button>
      {/if}
      <button
        type="button"
        class="icon-button"
        aria-label={t($language, '关闭预览')}
        onclick={onClose}
      >
        <Icon name="close" size={22} />
      </button>
    </div>
  </header>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    class="lightbox-stage"
    role="presentation"
    onclick={() => {
      if (dragged) {
        dragged = false;
        return;
      }
      onClose();
    }}
    onwheel={zoomBy}
  >
    {#if current}
      {#if isVideo(current.name)}
        <video
          src={url}
          controls
          autoplay
          playsinline
          onclick={(event) => event.stopPropagation()}
          onloadedmetadata={applyVideoVolume}
          onvolumechange={rememberVideoVolume}><track kind="captions" /></video
        >
      {:else if isAudio(current.name)}
        <audio src={url} controls autoplay onclick={(event) => event.stopPropagation()}></audio>
      {:else if isPdf(current.name)}
        <iframe class="lightbox-document" src={url} title={current.name}></iframe>
      {:else if previewFailed}
        <div class="preview-failure" role="alert" onclick={(event) => event.stopPropagation()}>
          <p>{t($language, '图片过大，无法预览')}</p>
          <a href={downloadHref ? downloadHref(current) : api.downloadUrl(root, entryPath)} download
            >{t($language, '下载原图')}</a
          >
        </div>
      {:else}
        <div
          class="preview-frame"
          class:ready={frameReady}
          class:clipping={blurVisible}
          style={`--ratio:${frameRatio};transform:translate(${offsetX}px, ${offsetY}px) scale(${zoom})`}
        >
          {#if blurVisible}
            <img
              class="preview-blur"
              src={lowResUrl}
              alt=""
              aria-hidden="true"
              onload={noteAspect}
            />
          {/if}
          <img
            class="preview-hi"
            class:loaded={hiResLoaded}
            src={previewSrc}
            alt={current.name}
            onclick={(event) => {
              if (dragged) dragged = false;
              event.stopPropagation();
            }}
            onpointerdown={startDrag}
            onpointerup={endDrag}
            onpointercancel={endDrag}
            onload={handleHiResLoad}
            onerror={handleHiResError}
          />
        </div>
      {/if}
    {/if}
  </div>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <footer class="lightbox-footer" onclick={(event) => event.stopPropagation()}>
    <button type="button" onclick={previous} disabled={index <= 0}>{t($language, '上一项')}</button>
    <span>{index + 1} / {items.length}</span>
    <button type="button" onclick={next} disabled={index >= items.length - 1}
      >{t($language, '下一项')}</button
    >
  </footer>
</div>

<style>
  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: flex;
    flex-direction: column;
    background: var(--scrim-strong);
    color: var(--lightbox-fg);
  }

  .lightbox-bar,
  .lightbox-footer {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: space-between;
    padding: 10px 16px;
    background: rgb(0 0 0 / 38%);
    color: var(--lightbox-fg);
  }

  .lightbox-bar strong {
    overflow: hidden;
    max-width: 60vw;
    font-size: 15px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .lightbox-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
  }

  .lightbox-actions button,
  .lightbox-footer button {
    height: 30px;
    padding: 0 10px;
    border: 1px solid rgb(255 255 255 / 24%);
    border-radius: 4px;
    background: rgb(255 255 255 / 8%);
    color: var(--lightbox-fg);
    font-size: 13px;
  }

  .lightbox-actions button:hover,
  .lightbox-footer button:hover {
    background: rgb(255 255 255 / 18%);
  }

  .lightbox-actions button:disabled,
  .lightbox-footer button:disabled {
    opacity: 0.4;
  }

  .lightbox-actions .icon-button {
    display: grid;
    width: 36px;
    padding: 0;
    place-items: center;
  }

  .preview-failure {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
    padding: 24px;
    color: var(--lightbox-fg);
    text-align: center;
  }

  .preview-failure a {
    padding: 6px 14px;
    border: 1px solid rgb(255 255 255 / 24%);
    border-radius: var(--radius-sm);
    color: inherit;
  }

  .lightbox-stage {
    position: relative;
    display: grid;
    flex: 1;
    min-height: 0;
    overflow: auto;
    place-items: center;
    padding: 18px;
  }

  .preview-frame {
    position: relative;
    display: block;
    width: min(92vw, calc(78vh * var(--ratio, 1)));
    max-width: 92vw;
    max-height: 78vh;
    aspect-ratio: var(--ratio, 1);
    cursor: grab;
    transform-origin: center;
    visibility: hidden;
  }

  .preview-frame.ready {
    visibility: visible;
  }

  .preview-frame.clipping {
    overflow: hidden;
  }

  .preview-frame:active {
    cursor: grabbing;
  }

  .preview-frame .preview-blur {
    position: absolute;
    inset: -72px;
    width: calc(100% + 144px);
    height: calc(100% + 144px);
    max-width: none;
    max-height: none;
    object-fit: fill;
    pointer-events: none;
    filter: blur(18px);
  }

  .preview-frame .preview-hi {
    display: block;
    width: 100%;
    height: 100%;
    max-width: none;
    max-height: none;
    object-fit: contain;
    opacity: 0;
    transition: opacity 80ms linear;
  }

  .preview-frame .preview-hi.loaded {
    opacity: 1;
  }

  .lightbox-stage img {
    max-width: 92vw;
    max-height: 78vh;
    cursor: grab;
    object-fit: contain;
    touch-action: none;
    user-select: none;
  }

  .lightbox-stage img:active {
    cursor: grabbing;
  }

  .lightbox-stage video {
    max-width: 92vw;
    max-height: 78vh;
    object-fit: contain;
  }

  .lightbox-stage audio {
    width: min(640px, 90vw);
  }

  .lightbox-document {
    width: 92vw;
    height: 78vh;
    border: 0;
    background: var(--surface);
  }

  .lightbox-footer {
    justify-content: center;
    gap: 16px;
    font-size: 13px;
  }
</style>
