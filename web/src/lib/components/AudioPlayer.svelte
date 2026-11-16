<script lang="ts">
  import { onMount } from 'svelte';
  import {
    audioState,
    closeAudio,
    nextAudio,
    previousAudio,
    setAudioPlaying,
    stopAudio,
    toggleAudio,
  } from '../audio';
  import { language, t } from '../i18n';

  let audio = $state<HTMLAudioElement | null>(null);
  let currentKey = '';
  let currentTime = $state(0);
  let duration = $state(0);
  let position = $state({ x: 0, y: 0 });
  let dragging = false;
  let dragStart = { x: 0, y: 0, left: 0, top: 0 };

  let current = $derived($audioState ? $audioState.playlist[$audioState.index] : null);

  onMount(() => {
    try {
      const stored = JSON.parse(localStorage.getItem('filehub.audio.position') ?? 'null') as {
        x: number;
        y: number;
      } | null;
      if (stored) position = stored;
    } catch {
      // ignore malformed positions
    }
  });

  $effect(() => {
    const state = $audioState;
    if (!audio) return;
    if (!state || !current) {
      audio.pause();
      currentKey = '';
      return;
    }
    if (currentKey !== current.key) {
      currentKey = current.key;
      audio.src = current.url;
      audio.currentTime = 0;
      currentTime = 0;
      duration = 0;
    }
    if (state.playing && audio.paused) {
      void audio.play().catch(() => setAudioPlaying(false));
    }
    if (!state.playing && !audio.paused) {
      audio.pause();
    }
  });

  function onTimeUpdate(): void {
    if (audio) currentTime = audio.currentTime;
  }

  function onLoaded(): void {
    if (audio) duration = Number.isFinite(audio.duration) ? audio.duration : 0;
  }

  function seek(event: Event): void {
    if (!audio) return;
    const value = Number((event.currentTarget as HTMLInputElement).value);
    audio.currentTime = value;
    currentTime = value;
  }

  function formatTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds <= 0) return '0:00';
    const total = Math.floor(seconds);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
  }

  function startDrag(event: PointerEvent): void {
    dragging = true;
    dragStart = {
      x: event.clientX,
      y: event.clientY,
      left: position.x,
      top: position.y,
    };
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function drag(event: PointerEvent): void {
    if (!dragging) return;
    position = {
      x: dragStart.left + event.clientX - dragStart.x,
      y: dragStart.top + event.clientY - dragStart.y,
    };
  }

  function endDrag(): void {
    if (!dragging) return;
    dragging = false;
    try {
      localStorage.setItem('filehub.audio.position', JSON.stringify(position));
    } catch {
      // ignore storage failures
    }
  }
</script>

{#if current && $audioState}
  <aside
    class="audio-player"
    style={`transform:translate(${position.x}px, ${position.y}px)`}
    aria-label={t($language, '播放器')}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header
      onpointerdown={startDrag}
      onpointermove={drag}
      onpointerup={endDrag}
      onpointercancel={endDrag}
      title={t($language, '拖动播放器')}
    >
      <span class="audio-title" title={current.name}>{current.name}</span>
      <button type="button" class="icon" aria-label={t($language, '关闭播放器')} onclick={closeAudio}
        >×</button
      >
    </header>
    <div class="audio-controls">
      <button type="button" onclick={previousAudio} aria-label={t($language, '上一首')}
        >|‹</button
      >
      <button
        type="button"
        onclick={toggleAudio}
        aria-label={$audioState.playing ? t($language, '暂停') : t($language, '播放')}
        >{$audioState.playing ? t($language, '暂停') : t($language, '播放')}</button
      >
      <button type="button" onclick={stopAudio} aria-label={t($language, '停止')}
        >{t($language, '停止')}</button
      >
      <button type="button" onclick={nextAudio} aria-label={t($language, '下一首')}
        >›|</button
      >
    </div>
    <div class="audio-progress">
      <span>{formatTime(currentTime)}</span>
      <input
        type="range"
        min="0"
        max={Math.max(1, duration)}
        step="0.1"
        value={currentTime}
        aria-label={t($language, '播放进度')}
        oninput={seek}
      />
      <span>{formatTime(duration)}</span>
    </div>
    <audio
      bind:this={audio}
      bind:duration
      ontimeupdate={onTimeUpdate}
      onloadedmetadata={onLoaded}
      onended={nextAudio}
      onplay={() => setAudioPlaying(true)}
      onpause={() => setAudioPlaying(false)}
    ></audio>
  </aside>
{/if}

<style>
  .audio-player {
    position: fixed;
    right: 20px;
    bottom: 20px;
    z-index: 90;
    display: grid;
    width: min(320px, calc(100vw - 32px));
    gap: 8px;
    padding: 10px 12px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    cursor: grab;
    touch-action: none;
  }

  .audio-title {
    overflow: hidden;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .icon {
    width: 24px;
    height: 24px;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font-size: 18px;
  }

  .audio-controls {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .audio-controls button {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
  }

  .audio-controls button:nth-child(2) {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }

  .audio-progress {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 11.5px;
  }

  .audio-progress input {
    flex: 1;
    min-width: 0;
  }

  audio {
    display: none;
  }
</style>
