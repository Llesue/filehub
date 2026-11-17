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
  import Icon from './Icon.svelte';

  let { hidden = false }: { hidden?: boolean } = $props();
  let audio = $state<HTMLAudioElement | null>(null);
  let currentKey = '';
  let currentTime = $state(0);
  let duration = $state(0);
  let volume = $state(0.8);
  let position = $state({ x: 0, y: 0 });
  let dragging = false;
  let dragStart = { x: 0, y: 0, left: 0, top: 0 };

  let current = $derived($audioState ? $audioState.playlist[$audioState.index] : null);
  const channel =
    typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel('filehub.audio') : null;

  onMount(() => {
    if (channel) {
      channel.onmessage = (message: MessageEvent<{ type?: string }>) => {
        if (message.data?.type === 'play' && audio && !audio.paused) {
          setAudioPlaying(false);
        }
      };
    }
    try {
      const stored = JSON.parse(localStorage.getItem('filehub.audio.position') ?? 'null') as {
        x: number;
        y: number;
      } | null;
      if (stored) position = stored;
      const rawVolume = localStorage.getItem('filehub.audio.volume');
      const storedVolume = rawVolume === null ? Number.NaN : Number(rawVolume);
      if (Number.isFinite(storedVolume) && storedVolume >= 0 && storedVolume <= 1) {
        volume = storedVolume;
      }
    } catch {
      // ignore malformed positions
    }
  });

  $effect(() => {
    if (audio) audio.volume = volume;
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

  function changeVolume(event: Event): void {
    const value = Number((event.currentTarget as HTMLInputElement).value);
    volume = Math.min(1, Math.max(0, value));
    if (audio) audio.volume = volume;
    try {
      localStorage.setItem('filehub.audio.volume', String(volume));
    } catch {
      // ignore storage failures
    }
  }

  function toggleMute(): void {
    volume = volume > 0 ? 0 : 0.8;
    if (audio) audio.volume = volume;
    try {
      localStorage.setItem('filehub.audio.volume', String(volume));
    } catch {
      // ignore storage failures
    }
  }

  function formatTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds <= 0) return '0:00';
    const total = Math.floor(seconds);
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, '0')}`;
  }

  function startDrag(event: PointerEvent): void {
    if ((event.target as HTMLElement | null)?.closest('button')) {
      return;
    }
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

  function stopPlayback(): void {
    stopAudio();
    if (audio) {
      audio.pause();
      audio.currentTime = 0;
    }
    currentTime = 0;
  }
</script>

{#if current && $audioState}
  <aside
    class="audio-player"
    class:lightbox-hidden={hidden}
    style={`transform:translate(${position.x}px, ${position.y}px)`}
    aria-label={t($language, '播放器')}
    aria-hidden={hidden}
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
      <button
        type="button"
        class="icon close"
        aria-label={t($language, '关闭播放器')}
        title={t($language, '关闭播放器')}
        onpointerdown={(event) => event.stopPropagation()}
        onclick={closeAudio}><Icon name="close" size={16} /></button
      >
    </header>
    <div class="audio-controls">
      <button
        type="button"
        onclick={previousAudio}
        aria-label={t($language, '上一首')}
        title={t($language, '上一首')}><Icon name="previous" size={16} /></button
      >
      <button
        type="button"
        class="primary"
        onclick={toggleAudio}
        aria-label={$audioState.playing ? t($language, '暂停') : t($language, '播放')}
        title={$audioState.playing ? t($language, '暂停') : t($language, '播放')}
        ><Icon name={$audioState.playing ? 'pause' : 'play'} size={16} /></button
      >
      <button
        type="button"
        onclick={stopPlayback}
        aria-label={t($language, '停止')}
        title={t($language, '停止')}><Icon name="stop" size={16} /></button
      >
      <button
        type="button"
        onclick={nextAudio}
        aria-label={t($language, '下一首')}
        title={t($language, '下一首')}><Icon name="next" size={16} /></button
      >
      <div class="audio-volume">
        <button
          type="button"
          class="icon"
          aria-label={volume > 0 ? t($language, '静音') : t($language, '取消静音')}
          title={volume > 0 ? t($language, '静音') : t($language, '取消静音')}
          onclick={toggleMute}
        >
          <Icon name={volume > 0 ? 'volume' : 'volume-muted'} size={16} />
        </button>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          value={volume}
          aria-label={t($language, '音量')}
          oninput={changeVolume}
        />
      </div>
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
      onplay={() => {
        setAudioPlaying(true);
        channel?.postMessage({ type: 'play' });
      }}
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
    grid-template-columns: minmax(0, 1fr);
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
    min-width: 0;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    cursor: grab;
    touch-action: none;
  }

  .audio-title {
    flex: 1 1 0;
    width: 0;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .icon {
    flex: 0 0 auto;
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
    display: grid;
    width: 32px;
    height: 30px;
    place-items: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
    font-size: 12.5px;
  }

  .audio-controls button.primary {
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

  .audio-volume {
    display: flex;
    flex: 0 1 86px;
    min-width: 64px;
    align-items: center;
    gap: 2px;
    margin-left: 2px;
  }

  .audio-volume input {
    flex: 1;
    width: 0;
    min-width: 0;
  }

  .audio-volume .icon {
    width: 22px;
    height: 22px;
  }

  .audio-player.lightbox-hidden {
    visibility: hidden;
    opacity: 0;
    pointer-events: none;
  }

  audio {
    display: none;
  }
</style>
