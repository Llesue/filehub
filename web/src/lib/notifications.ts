import { derived, writable } from 'svelte/store';

export interface NotificationItem {
  id: string;
  kind: string;
  message: string;
  at: number;
  read: boolean;
}

const STORAGE_KEY = 'filehub.notifications';
const MAX_ITEMS = 50;

function loadStored(): NotificationItem[] {
  if (typeof localStorage === 'undefined') return [];
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((item): item is NotificationItem => {
        if (item === null || typeof item !== 'object') return false;
        const record = item as Record<string, unknown>;
        return (
          typeof record.id === 'string' &&
          typeof record.kind === 'string' &&
          typeof record.message === 'string' &&
          typeof record.at === 'number' &&
          typeof record.read === 'boolean'
        );
      })
      .slice(0, MAX_ITEMS);
  } catch {
    return [];
  }
}

function persist(items: NotificationItem[]): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(items));
  } catch {
    // Storage can be full or disabled; in-memory notifications still work.
  }
}

export const notifications = writable<NotificationItem[]>(loadStored());

export const unreadCount = derived(
  notifications,
  (items) => items.filter((item) => !item.read).length,
);

export function addNotification(kind: string, message: string): void {
  const text = message.trim();
  if (!text) return;
  const item: NotificationItem = {
    id: `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`,
    kind,
    message: text,
    at: Date.now(),
    read: false,
  };
  notifications.update((items) => {
    const next = [item, ...items].slice(0, MAX_ITEMS);
    persist(next);
    return next;
  });
}

export function markAllRead(): void {
  notifications.update((items) => {
    if (items.every((item) => item.read)) return items;
    const next = items.map((item) => (item.read ? item : { ...item, read: true }));
    persist(next);
    return next;
  });
}

export function clearNotifications(): void {
  notifications.set([]);
  persist([]);
}

export function ensureNotificationOwner(userId: number): void {
  const ownerKey = 'filehub.notifications.owner';
  try {
    if (localStorage.getItem(ownerKey) === String(userId)) return;
    localStorage.setItem(ownerKey, String(userId));
  } catch {
    // Storage is optional; in-memory notifications are still scoped by reload.
  }
  clearNotifications();
}
