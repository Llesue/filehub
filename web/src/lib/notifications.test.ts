import { get } from 'svelte/store';
import { beforeEach, describe, expect, it } from 'vitest';
import {
  addNotification,
  clearNotifications,
  markAllRead,
  notifications,
  unreadCount,
} from './notifications';

describe('notification center store', () => {
  beforeEach(() => {
    clearNotifications();
  });

  it('adds notifications and counts unread items', () => {
    addNotification('job_failed', 'Thumbnail generation failed');
    addNotification('quota', 'Quota is nearly full');
    expect(get(notifications)).toHaveLength(2);
    expect(get(unreadCount)).toBe(2);
  });

  it('ignores empty messages', () => {
    addNotification('notice', '   ');
    expect(get(notifications)).toHaveLength(0);
  });

  it('marks every item as read', () => {
    addNotification('notice', 'one');
    addNotification('notice', 'two');
    markAllRead();
    expect(get(unreadCount)).toBe(0);
    expect(get(notifications).every((item) => item.read)).toBe(true);
  });

  it('orders newest first and caps the list', () => {
    for (let index = 0; index < 55; index += 1) {
      addNotification('notice', `message ${index}`);
    }
    const items = get(notifications);
    expect(items).toHaveLength(50);
    expect(items[0]?.message).toBe('message 54');
    expect(items[49]?.message).toBe('message 5');
  });
});
