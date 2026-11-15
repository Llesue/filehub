import { writable } from 'svelte/store';

export type NoticeKind = 'error' | 'warning' | 'info';

export interface Notice {
  title: string;
  message: string;
  kind: NoticeKind;
  actionLabel?: string;
  action?: () => void;
}

export const notice = writable<Notice | null>(null);

export function showNotice(message: string, kind: NoticeKind = 'error', title = '操作失败'): void {
  notice.set({ title, message, kind });
}

export function showNoticeAction(
  message: string,
  title: string,
  actionLabel: string,
  action: () => void,
  kind: NoticeKind = 'info',
): void {
  notice.set({ title, message, kind, actionLabel, action });
}

export function clearNotice(): void {
  notice.set(null);
}
