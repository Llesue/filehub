<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { api, type Entry } from '../api';
  import { audioState, setAudioPlaying } from '../audio';
  import Icon from './Icon.svelte';
  import FileName from './FileName.svelte';
  import { language, t } from '../i18n';
  import { classifyFile } from '../fileKind';
  import type { AudioInfo } from '../api';

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
    previewEdge = 2048,
    onEditSource,
    csrf,
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
    previewEdge?: number;
    onEditSource?: (entry: Entry) => void;
    csrf?: string;
  } = $props();

  let current = $derived(items[index]);
  let zoom = $state(1);
  let offsetX = $state(0);
  let offsetY = $state(0);
  let showOriginal = $state(false);
  let imageFallback = $state(false);
  let previewFailed = $state(false);
  let failureText = $state('');
  let hiResLoaded = $state(false);
  let blurPhase = $state<'visible' | 'fading' | 'hidden'>('visible');
  let frameRatio = $state(1);
  let frameReady = $state(false);
  let naturalWidth = $state(0);
  let blurCanvas = $state<HTMLCanvasElement | null>(null);
  let blurImageLoaded = $state(false);
  let sizeResolved = $state(false);
  let blurFallback = $state(false);
  let prepareToken = 0;
  let sizeTimer: ReturnType<typeof setTimeout> | null = null;
  let blurSourceImage: HTMLImageElement | null = null;
  let infoWidth = 0;
  let infoHeight = 0;
  let fadeBlurTimer: ReturnType<typeof setTimeout> | null = null;
  let hideBlurTimer: ReturnType<typeof setTimeout> | null = null;
  let svgUrl = $state('');
  let svgAbort: AbortController | null = null;
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
    prepareToken += 1;
    clearSizeTimer();
    clearBlurTimers();
    clearSvg();
    blurSourceImage = null;
    if (resumeAudio) setAudioPlaying(true);
  });
  let entryPath = $derived(
    current ? (currentPath ? `${currentPath}/${current.name}` : current.name) : '',
  );
  let isSvgPreview = $derived(current ? isSvgName(current.name) : false);
  let isGifPreview = $derived(current ? /\.gif$/i.test(current.name) : false);
  let url = $derived(current ? (srcFor ? srcFor(current) : api.previewUrl(root, entryPath)) : '');
  let audioInfo = $state<AudioInfo | null>(null);
  let audioInfoToken = 0;
  let transcodeBusy = $state(false);
  let transcodeError = $state('');
  let transcodedUrl = $state('');
  let playUrl = $derived(transcodedUrl || url);
  let warningTrack = $derived(
    audioInfo?.ffmpeg && audioInfo.tracks.length > 0
      ? (audioInfo.default_track !== null
          ? audioInfo.tracks[audioInfo.default_track]
          : audioInfo.tracks[0])
      : undefined,
  );

  $effect(() => {
    const entry = current;
    transcodedUrl = '';
    transcodeError = '';
    audioInfo = null;
    if (!entry || !isVideo(entry.name) || !csrf) return;
    const token = ++audioInfoToken;
    void api
      .audioInfo(root, entryPath)
      .then((info) => {
        if (token === audioInfoToken) audioInfo = info;
      })
      .catch(() => {
        if (token === audioInfoToken) audioInfo = null;
      });
  });

  async function startTranscode(): Promise<void> {
    if (!current || !csrf || transcodeBusy) return;
    transcodeBusy = true;
    transcodeError = '';
    const track = audioInfo?.default_track ?? warningTrack?.index ?? 0;
    try {
      const result = await api.transcodeAudio(root, entryPath, track, csrf);
      if (result.ready) {
        transcodedUrl = api.transcodedUrl(root, entryPath, track);
        return;
      }
      if (!result.job_id) throw new Error(t($language, '转码失败'));
      for (;;) {
        const job = await api.job(result.job_id);
        if (job.status === 'completed') {
          transcodedUrl = api.transcodedUrl(root, entryPath, track);
          return;
        }
        if (job.status === 'failed') throw new Error(job.error ?? t($language, '转码失败'));
        if (job.status === 'cancelled') throw new Error(t($language, '操作已取消'));
        await new Promise((resolve) => setTimeout(resolve, 500));
      }
    } catch (error) {
      transcodeError = error instanceof Error ? error.message : t($language, '转码失败');
    } finally {
      transcodeBusy = false;
    }
  }
  let lowResUrl = $derived(
    current
      ? thumbSrcFor
        ? thumbSrcFor(current)
        : api.thumbUrl(root, entryPath, 320, undefined, true)
      : '',
  );
  let previewSrc = $derived(
    current
      ? isSvgPreview
        ? svgUrl
        : showOriginal || imageFallback || isGifPreview
          ? url
          : thumbSrcFor
            ? thumbSrcFor(current)
            : api.previewThumbUrl(root, entryPath, previewEdge)
      : '',
  );

  $effect(() => {
    const entry = current;
    untrack(() => {
      if (!entry || !isSvgName(entry.name)) {
        clearSvg();
        return;
      }
      if (entry.size > 10 * 1024 * 1024) {
        clearSvg();
        previewFailed = true;
        failureText = t($language, '文件过大，无法预览');
        return;
      }
      loadSvg();
    });
  });

  $effect(() => {
    const entry = current;
    untrack(() => preparePreview(entry));
  });

  function clearSizeTimer(): void {
    if (sizeTimer !== null) {
      clearTimeout(sizeTimer);
      sizeTimer = null;
    }
  }

  function clearBlurTimers(): void {
    if (fadeBlurTimer !== null) {
      clearTimeout(fadeBlurTimer);
      fadeBlurTimer = null;
    }
    if (hideBlurTimer !== null) {
      clearTimeout(hideBlurTimer);
      hideBlurTimer = null;
    }
  }

  function preparePreview(entry: Entry | undefined): void {
    prepareToken += 1;
    const token = prepareToken;
    clearSizeTimer();
    clearBlurTimers();
    blurSourceImage = null;
    blurImageLoaded = false;
    sizeResolved = false;
    frameReady = false;
    blurFallback = false;
    naturalWidth = 0;
    frameRatio = 1;
    infoWidth = 0;
    infoHeight = 0;
    if (!entry) return;
    if (isSvgPreview || isVideo(entry.name) || isAudio(entry.name) || isPdf(entry.name)) {
      sizeResolved = true;
      return;
    }

    const image = new Image();
    image.decoding = 'async';
    image.onload = () => {
      if (token !== prepareToken) return;
      if (image.naturalWidth > 0 && image.naturalHeight > 0) {
        frameRatio = image.naturalWidth / image.naturalHeight;
      }
      blurSourceImage = image;
      blurImageLoaded = true;
      maybeRevealFrame();
    };
    image.onerror = () => {
      if (token !== prepareToken) return;
      blurImageLoaded = true;
      maybeRevealFrame();
    };
    image.src = lowResUrl;

    const finishSize = (): void => {
      if (token !== prepareToken) return;
      clearSizeTimer();
      sizeResolved = true;
      maybeRevealFrame();
    };
    if (root > 0) {
      api
        .imageInfo(root, entryPath)
        .then((info) => {
          if (token !== prepareToken) return;
          infoWidth = info.width ?? 0;
          infoHeight = info.height ?? 0;
          finishSize();
        })
        .catch(() => finishSize());
      sizeTimer = setTimeout(finishSize, 200);
    } else {
      finishSize();
    }
  }

  function maybeRevealFrame(): void {
    if (frameReady || !blurImageLoaded || !sizeResolved) return;
    if (infoWidth > 0 && infoHeight > 0) {
      const infoRatio = infoWidth / infoHeight;
      // EXIF rotation can make the header ratio differ from the rendered
      // thumbnail. Only apply the natural-size cap when both agree.
      naturalWidth = Math.abs(infoRatio / frameRatio - 1) <= 0.02 ? infoWidth : 0;
    }
    if (blurSourceImage) drawBlurredThumb(blurSourceImage);
    frameReady = true;
  }

  function drawBlurredThumb(image: HTMLImageElement): void {
    const canvas = blurCanvas;
    const sourceWidth = image.naturalWidth;
    const sourceHeight = image.naturalHeight;
    if (!canvas || sourceWidth <= 0 || sourceHeight <= 0) return;
    const ratio = sourceWidth / sourceHeight;
    const cap = naturalWidth > 0 ? naturalWidth : Number.POSITIVE_INFINITY;
    const displayWidth = Math.max(
      1,
      Math.min(window.innerWidth * 0.92, window.innerHeight * 0.78 * ratio, cap),
    );
    // Blur a bounded-size copy: the layer is only visible for ~300ms and is
    // stretched to the frame, so a 2048px source adds no visible quality.
    const baseWidth = Math.min(sourceWidth, 512);
    const baseHeight = Math.max(1, Math.round((sourceHeight * baseWidth) / sourceWidth));
    const radius = Math.min(48, Math.max(2, (18 * baseWidth) / displayWidth));
    const pad = Math.ceil(radius * 3);

    const padded = document.createElement('canvas');
    padded.width = baseWidth + pad * 2;
    padded.height = baseHeight + pad * 2;
    const paddedContext = padded.getContext('2d');
    if (!paddedContext) return;
    paddedContext.drawImage(image, pad, pad, baseWidth, baseHeight);
    paddedContext.drawImage(image, 0, 0, 1, sourceHeight, 0, pad, pad, baseHeight);
    paddedContext.drawImage(
      image,
      sourceWidth - 1,
      0,
      1,
      sourceHeight,
      pad + baseWidth,
      pad,
      pad,
      baseHeight,
    );
    paddedContext.drawImage(image, 0, 0, sourceWidth, 1, pad, 0, baseWidth, pad);
    paddedContext.drawImage(
      image,
      0,
      sourceHeight - 1,
      sourceWidth,
      1,
      pad,
      pad + baseHeight,
      baseWidth,
      pad,
    );
    paddedContext.drawImage(image, 0, 0, 1, 1, 0, 0, pad, pad);
    paddedContext.drawImage(image, sourceWidth - 1, 0, 1, 1, pad + baseWidth, 0, pad, pad);
    paddedContext.drawImage(
      image,
      0,
      sourceHeight - 1,
      1,
      1,
      0,
      pad + baseHeight,
      pad,
      pad,
    );
    paddedContext.drawImage(
      image,
      sourceWidth - 1,
      sourceHeight - 1,
      1,
      1,
      pad + baseWidth,
      pad + baseHeight,
      pad,
      pad,
    );

    canvas.width = baseWidth;
    canvas.height = baseHeight;
    const context = canvas.getContext('2d');
    if (!context) return;
    context.filter = `blur(${radius}px)`;
    if (!context.filter.includes('blur')) {
      blurFallback = true;
      return;
    }
    context.drawImage(padded, -pad, -pad);
    context.filter = 'none';
  }

  function handleHiResLoad(event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    if (isSvgPreview) {
      if (image.naturalWidth > 0 && image.naturalHeight > 0) {
        frameRatio = image.naturalWidth / image.naturalHeight;
      }
      frameReady = true;
    } else if (!blurImageLoaded && image.naturalWidth > 0 && image.naturalHeight > 0) {
      frameRatio = image.naturalWidth / image.naturalHeight;
    }
    hiResLoaded = true;
    clearBlurTimers();
    if (isSvgPreview) {
      blurPhase = 'hidden';
      return;
    }
    // Keep the blurred layer under the 80ms fade, then fade it out. The node
    // stays mounted so removing it can never re-present a stale blur frame.
    fadeBlurTimer = setTimeout(() => {
      blurPhase = 'fading';
      fadeBlurTimer = null;
    }, 100);
    hideBlurTimer = setTimeout(() => {
      blurPhase = 'hidden';
      hideBlurTimer = null;
    }, 220);
  }

  function handleHiResError(): void {
    clearBlurTimers();
    hiResLoaded = false;
    blurPhase = 'visible';
    if (showOriginal || imageFallback || isSvgPreview) {
      previewFailed = true;
      failureText = t($language, '无法显示该图片');
    } else {
      imageFallback = true;
    }
  }

  function resetView(): void {
    clearBlurTimers();
    zoom = 1;
    offsetX = 0;
    offsetY = 0;
    showOriginal = false;
    imageFallback = false;
    previewFailed = false;
    failureText = '';
    hiResLoaded = false;
    blurPhase = 'visible';
    naturalWidth = 0;
    frameReady = false;
  }

  function toggleOriginal(): void {
    showOriginal = !showOriginal;
    hiResLoaded = false;
    previewFailed = false;
    failureText = '';
    naturalWidth = 0;
    clearBlurTimers();
    blurPhase = showOriginal ? 'hidden' : 'visible';
  }

  function clearSvg(): void {
    if (svgAbort) {
      svgAbort.abort();
      svgAbort = null;
    }
    if (svgUrl) {
      URL.revokeObjectURL(svgUrl);
      svgUrl = '';
    }
  }

  function loadSvg(): void {
    clearSvg();
    const controller = new AbortController();
    svgAbort = controller;
    fetch(url, { signal: controller.signal, credentials: 'same-origin' })
      .then((response) => {
        if (!response.ok) throw new Error(`HTTP ${response.status}`);
        return response.blob();
      })
      .then((blob) => {
        if (controller.signal.aborted) return;
        svgUrl = URL.createObjectURL(blob);
        svgAbort = null;
      })
      .catch(() => {
        if (controller.signal.aborted) return;
        svgAbort = null;
        previewFailed = true;
        failureText = t($language, '无法显示该图片');
      });
  }

  let touchStartX = 0;
  let touchStartY = 0;
  let mobile = $state(false);
  let chromeHidden = $state(false);
  let tapTimer: number | null = null;
  let lastTap = 0;
  let frameEl = $state<HTMLElement | null>(null);
  let pinchCenterX = 0;
  let pinchCenterY = 0;
  let pinching = false;
  let pinchStartDistance = 0;
  let pinchStartZoom = 1;
  let pinchStartOffsetX = 0;
  let pinchStartOffsetY = 0;
  let pinchStartMidX = 0;
  let pinchStartMidY = 0;

  function pinchMetrics(touches: TouchList): { distance: number; midX: number; midY: number } {
    const a = touches[0];
    const b = touches[1];
    if (!a || !b) return { distance: 0, midX: 0, midY: 0 };
    return {
      distance: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY),
      midX: (a.clientX + b.clientX) / 2,
      midY: (a.clientY + b.clientY) / 2,
    };
  }

  onMount(() => {
    mobile = window.matchMedia('(pointer: coarse)').matches;
  });

  function onStageClick(): void {
    if (!mobile) {
      if (dragged) {
        dragged = false;
        return;
      }
      onClose();
      return;
    }
    if (dragged || pinching) {
      dragged = false;
      return;
    }
    const now = Date.now();
    if (now - lastTap <= 300) {
      if (tapTimer) {
        window.clearTimeout(tapTimer);
        tapTimer = null;
      }
      lastTap = 0;
      chromeHidden = true;
      zoom = zoom > 1.01 ? 1 : 1.5;
      offsetX = 0;
      offsetY = 0;
      return;
    }
    lastTap = now;
    if (tapTimer) window.clearTimeout(tapTimer);
    tapTimer = window.setTimeout(() => {
      tapTimer = null;
      lastTap = 0;
      chromeHidden = !chromeHidden;
    }, 280);
  }

  function onTouchStart(event: TouchEvent): void {
    if (event.touches.length === 2) {
      const metrics = pinchMetrics(event.touches);
      const rect = frameEl?.getBoundingClientRect();
      pinchCenterX = rect ? rect.left + rect.width / 2 : window.innerWidth / 2;
      pinchCenterY = rect ? rect.top + rect.height / 2 : window.innerHeight / 2;
      if (tapTimer) {
        window.clearTimeout(tapTimer);
        tapTimer = null;
      }
      pinching = true;
      dragging = false;
      pinchStartDistance = Math.max(1, metrics.distance);
      pinchStartZoom = zoom;
      pinchStartOffsetX = offsetX;
      pinchStartOffsetY = offsetY;
      pinchStartMidX = metrics.midX;
      pinchStartMidY = metrics.midY;
      return;
    }
    const touch = event.touches[0];
    if (!touch || pinching) return;
    touchStartX = touch.clientX;
    touchStartY = touch.clientY;
  }

  function onTouchMove(event: TouchEvent): void {
    if (!pinching || event.touches.length !== 2) return;
    event.preventDefault();
    const metrics = pinchMetrics(event.touches);
    const target = Math.min(
      8,
      Math.max(0.2, pinchStartZoom * (metrics.distance / pinchStartDistance)),
    );
    const ratio = target / pinchStartZoom;
    zoom = target;
    offsetX = metrics.midX - pinchCenterX - (pinchStartMidX - pinchCenterX - pinchStartOffsetX) * ratio;
    offsetY = metrics.midY - pinchCenterY - (pinchStartMidY - pinchCenterY - pinchStartOffsetY) * ratio;
  }

  function onTouchEnd(event: TouchEvent): void {
    if (pinching) {
      if (event.touches.length < 2) {
        pinching = false;
        dragged = true;
      }
      return;
    }
    const touch = event.changedTouches[0];
    if (!touch) return;
    const dx = touch.clientX - touchStartX;
    const dy = touch.clientY - touchStartY;
    if (Math.abs(dx) < 60 || Math.abs(dy) > 50) return;
    if (dx > 0) previous();
    else next();
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
    dragged = false;
    dragStart = { x: event.clientX, y: event.clientY, offsetX, offsetY };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function drag(event: PointerEvent): void {
    if (!dragging) return;
    const dx = event.clientX - dragStart.x;
    const dy = event.clientY - dragStart.y;
    if (Math.abs(dx) > 4 || Math.abs(dy) > 4) dragged = true;
    offsetX = dragStart.offsetX + dx;
    offsetY = dragStart.offsetY + dy;
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
    return classifyFile(name) === 'video';
  }

  function isAudio(name: string): boolean {
    return classifyFile(name) === 'audio';
  }

  function isPdf(name: string): boolean {
    return classifyFile(name) === 'pdf';
  }

  function isSvgName(name: string): boolean {
    return /\.svg$/i.test(name);
  }
</script>

{#snippet imageTools()}
  {#if current && !isVideo(current.name) && !isAudio(current.name) && !isPdf(current.name)}
    <button type="button" onclick={() => (zoom = Math.max(0.2, zoom - 0.2))}>−</button>
    <span class="zoom-value">{Math.round(zoom * 100)}%</span>
    <button type="button" onclick={() => (zoom = Math.min(8, zoom + 0.2))}>+</button>
    <button type="button" onclick={() => resetView()}>{t($language, '复位')}</button>
    {#if !isSvgPreview && !isGifPreview}
      <button type="button" onclick={toggleOriginal}>
        {showOriginal ? t($language, '显示预览图') : t($language, '显示原图')}
      </button>
    {/if}
  {/if}
{/snippet}

<svelte:window onkeydown={onKeydown} onpointermove={drag} onpointerup={endDrag} />

<div
  class="lightbox"
  class:mobile
  class:chrome-hidden={chromeHidden}
  role="presentation"
  onclick={onClose}
>
  {#if mobile && current}
    <div
      class="lightbox-blur"
      style={`background-image:url("${thumbSrcFor?.(current) ?? previewSrc}")`}
      aria-hidden="true"
    ></div>
  {/if}
  <header class="lightbox-bar" role="presentation" onclick={(event) => event.stopPropagation()}>
    <strong><FileName name={current?.name ?? ''} /></strong>
    <div class="lightbox-actions">
      <span class="desktop-image-tools">{@render imageTools()}</span>
      {#if current && isSvgPreview && onEditSource}
        <button type="button" onclick={() => onEditSource(current)}>
          {t($language, '编辑源码')}
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
    ontouchstart={onTouchStart}
    ontouchmove={onTouchMove}
    ontouchend={onTouchEnd}
    ontouchcancel={onTouchEnd}
    role="presentation"
    onclick={(event) => {
      event.stopPropagation();
      onStageClick();
    }}
    onwheel={zoomBy}
  >
    {#if current}
      {#if isVideo(current.name)}
        <video
          src={playUrl}
          controls
          autoplay
          playsinline
          onclick={(event) => event.stopPropagation()}
          onloadedmetadata={applyVideoVolume}
          onvolumechange={rememberVideoVolume}><track kind="captions" /></video
        >
        {#if csrf && warningTrack && !warningTrack.supported}
          <div class="audio-warning" role="presentation" onclick={(event) => event.stopPropagation()}>
            <span
              >{t($language, '默认音轨 {codec} 浏览器无法播放声音。', {
                codec: warningTrack.codec.toUpperCase(),
              })}</span
            >
            <button type="button" disabled={transcodeBusy} onclick={() => void startTranscode()}>
              {transcodeBusy ? t($language, '转码中…') : t($language, '转码播放')}
            </button>
            {#if downloadHref}
              <a href={downloadHref(current)}>{t($language, '下载文件')}</a>
            {/if}
            {#if transcodeError}<span class="audio-error">{transcodeError}</span>{/if}
          </div>
        {/if}
      {:else if isAudio(current.name)}
        <audio src={url} controls autoplay onclick={(event) => event.stopPropagation()}></audio>
      {:else if isPdf(current.name)}
        <iframe class="lightbox-document" src={url} title={current.name}></iframe>
      {:else if previewFailed}
        <div class="preview-failure" role="alert" onclick={(event) => event.stopPropagation()}>
          <p>{failureText || t($language, '图片过大，无法预览')}</p>
          <a href={downloadHref ? downloadHref(current) : api.downloadUrl(root, entryPath)} download
            >{t($language, '下载原图')}</a
          >
        </div>
      {:else}
        <div
          class="preview-frame"
          bind:this={frameEl}
          class:ready={frameReady}
          class:natural={naturalWidth > 0 && !isSvgPreview}
          style={`--ratio:${frameRatio};--natural:${naturalWidth}px;transform:translate(${offsetX}px, ${offsetY}px) scale(${zoom})`}
        >
          {#if !isSvgPreview}
            <canvas
              bind:this={blurCanvas}
              class="preview-blur"
              class:fallback-blur={blurFallback}
              class:fading={blurPhase === 'fading'}
              class:hidden-blur={blurPhase === 'hidden'}
              aria-hidden="true"
            ></canvas>
          {/if}
          {#if previewSrc}
            <img
              class="preview-hi"
              class:loaded={hiResLoaded}
              src={previewSrc}
              alt={current.name}
              onclick={(event) => {
                event.stopPropagation();
                if (mobile) onStageClick();
                else if (dragged) dragged = false;
              }}
              onpointerdown={startDrag}
              onpointerup={endDrag}
              onpointercancel={endDrag}
              onload={handleHiResLoad}
              onerror={handleHiResError}
            />
          {/if}
        </div>
      {/if}
    {/if}
  </div>

  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <footer class="lightbox-footer" onclick={(event) => event.stopPropagation()}>
    <div class="lightbox-mobile-tools">{@render imageTools()}</div>
    <div class="lightbox-nav">
      <button type="button" onclick={previous} disabled={index <= 0}
        >{t($language, '上一项')}</button
      >
      <span>{index + 1} / {items.length}</span>
      <button type="button" onclick={next} disabled={index >= items.length - 1}
        >{t($language, '下一项')}</button
      >
    </div>
  </footer>
</div>

<style>
  .lightbox-mobile-tools {
    display: none;
  }

  .lightbox-actions button,
  .lightbox-footer button {
    white-space: nowrap;
    flex: 0 0 auto;
  }

  .lightbox-nav {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }

  @media (max-width: 640px) {
    .lightbox-bar {
      padding: 6px 8px;
      padding-top: max(6px, env(safe-area-inset-top, 6px));
    }

    .lightbox-actions button {
      width: 44px;
      height: 44px;
    }

    .lightbox-bar strong {
      min-width: 0;
      flex: 1;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .lightbox-actions {
      flex: 0 0 auto;
    }

    .desktop-image-tools {
      display: none;
    }

    .lightbox-footer {
      flex-direction: column;
      padding-bottom: max(8px, env(safe-area-inset-bottom, 8px));
    }

    .lightbox-mobile-tools {
      display: flex;
      width: 100%;
      flex-wrap: wrap;
      align-items: center;
      justify-content: center;
      gap: 6px;
    }

    .lightbox-mobile-tools button {
      min-height: 40px;
      padding: 0 10px;
    }

    .lightbox-nav {
      width: 100%;
    }

    .lightbox-nav button {
      min-height: 40px;
    }
  }

  .lightbox {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: flex;
    flex-direction: column;
    background: var(--scrim-strong);
    color: var(--lightbox-fg);
  }

  .lightbox-blur {
    position: absolute;
    inset: -12%;
    z-index: 0;
    background-position: center;
    background-size: cover;
    filter: blur(32px) brightness(0.55);
    transform: scale(1.08);
    pointer-events: none;
  }

  .lightbox-bar,
  .lightbox-stage,
  .lightbox-footer {
    position: relative;
    z-index: 1;
  }

  .lightbox-bar,
  .lightbox-footer {
    transition: opacity 0.18s ease;
  }

  .lightbox.chrome-hidden .lightbox-bar,
  .lightbox.chrome-hidden .lightbox-footer {
    opacity: 0;
    pointer-events: none;
  }

  .lightbox.mobile .lightbox-stage {
    padding: 0;
  }

  .lightbox.mobile .preview-frame {
    width: 100vw;
    max-width: none;
    max-height: none;
    border-radius: 0;
  }

  .lightbox.mobile .preview-frame .preview-hi {
    width: 100%;
    max-width: none;
    max-height: none;
    height: auto;
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
    overflow: hidden;
    cursor: grab;
    transform-origin: center;
    visibility: hidden;
  }

  .preview-frame.natural {
    width: min(92vw, calc(78vh * var(--ratio, 1)), var(--natural, 100vw));
  }

  .preview-frame.ready {
    visibility: visible;
  }

  .preview-frame:active {
    cursor: grabbing;
  }

  /* The sharp image must paint above the persistent blur node. Without the
     explicit z-index the absolutely positioned blur canvas paints over the
     static img, so the compositor can flash a blur-only frame mid-fade. */
  .preview-frame .preview-blur {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
    z-index: 0;
    max-width: none;
    max-height: none;
    pointer-events: none;
    opacity: 1;
  }

  .preview-frame .preview-blur.fallback-blur {
    filter: blur(18px);
  }

  .preview-frame .preview-blur.fading {
    opacity: 0;
    transition: opacity 120ms linear;
  }

  .preview-frame .preview-blur.hidden-blur {
    opacity: 0;
    visibility: hidden;
  }

  .preview-frame .preview-hi {
    position: relative;
    z-index: 1;
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

  .audio-warning {
    position: absolute;
    bottom: 24px;
    left: 50%;
    display: flex;
    max-width: min(680px, calc(100% - 32px));
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    transform: translateX(-50%);
    box-shadow: var(--shadow);
  }

  .audio-warning button,
  .audio-warning a {
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    text-decoration: none;
  }

  .audio-warning button:disabled {
    opacity: 0.6;
  }

  .audio-error {
    color: var(--danger);
  }
</style>
