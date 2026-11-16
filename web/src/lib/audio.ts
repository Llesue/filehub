import { writable } from 'svelte/store';

export interface AudioTrack {
  name: string;
  url: string;
  key: string;
}

export interface AudioState {
  playlist: AudioTrack[];
  index: number;
  playing: boolean;
}

export const audioState = writable<AudioState | null>(null);

export function playAudio(playlist: AudioTrack[], index: number): void {
  if (playlist.length === 0) return;
  const safe = Math.max(0, Math.min(index, playlist.length - 1));
  audioState.set({ playlist, index: safe, playing: true });
}

export function toggleAudio(): void {
  audioState.update((state) =>
    state ? { ...state, playing: !state.playing } : state,
  );
}

export function stopAudio(): void {
  audioState.update((state) => (state ? { ...state, playing: false } : state));
}

export function closeAudio(): void {
  audioState.set(null);
}

export function nextAudio(): void {
  audioState.update((state) => {
    if (!state || state.playlist.length === 0) return state;
    const index = (state.index + 1) % state.playlist.length;
    return { ...state, index, playing: true };
  });
}

export function previousAudio(): void {
  audioState.update((state) => {
    if (!state || state.playlist.length === 0) return state;
    const index = (state.index - 1 + state.playlist.length) % state.playlist.length;
    return { ...state, index, playing: true };
  });
}

export function setAudioPlaying(playing: boolean): void {
  audioState.update((state) => (state ? { ...state, playing } : state));
}
