import { get, writable } from 'svelte/store';
import { language, t } from './i18n';

export type ProgressStatus = 'running' | 'done' | 'error';

export interface ProgressState {
  visible: boolean;
  title: string;
  detail: string;
  current: number;
  total: number;
  status: ProgressStatus;
  onCancel?: (() => void) | null;
}

const initial: ProgressState = {
  visible: false,
  title: '',
  detail: '',
  current: 0,
  total: 0,
  status: 'running',
  onCancel: null,
};

export const progress = writable<ProgressState>(initial);

export function startProgress(title: string, total = 0, detail = ''): void {
  progress.set({
    visible: true,
    title,
    detail,
    current: 0,
    total,
    status: 'running',
    onCancel: null,
  });
}

export function setProgressCancel(onCancel: (() => void) | null): void {
  progress.update((value) => ({ ...value, onCancel }));
}

export function updateProgress(current: number, detail = '', total?: number): void {
  progress.update((value) => ({
    ...value,
    current,
    detail,
    total: total ?? value.total,
  }));
}

export function finishProgress(detail = t(get(language), '完成')): void {
  progress.update((value) => ({ ...value, current: value.total, detail, status: 'done' }));
  setTimeout(() => progress.set(initial), 1800);
}

export function failProgress(detail: string): void {
  progress.update((value) => ({ ...value, detail, status: 'error' }));
  setTimeout(() => progress.set(initial), 4000);
}
