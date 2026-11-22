import { writable } from 'svelte/store';

export interface RealtimeEvent {
  type: string;
  data: Record<string, unknown>;
}

interface WireEvent {
  type: string;
  data: Record<string, unknown>;
}

interface LeaderLease {
  id: string;
  ts: number;
}

export const realtimeEvent = writable<RealtimeEvent | null>(null);
export const realtimeActive = writable(false);

const CHANNEL_PREFIX = 'filehub.events';
const LEASE_PREFIX = 'filehub.events.leader';
const LEASE_MS = 6000;
const HEARTBEAT_MS = 2000;

const eventTypes = [
  'job.updated',
  'usage.updated',
  'dir_size.updated',
  'thumbnail.ready',
  'thumbnail.failed',
  'fs.changed',
  'share.created',
  'share.revoked',
  'share.visited',
  'share.downloaded',
  'share.uploaded',
  'notification',
  'admin.status',
  'audit.appended',
  'slow_request',
  'edit.presence',
  'user.features_changed',
  'roots.changed',
  'settings.changed',
  'updates.changed',
  'share.updated',
  'user.updated',
  'resync',
  'session.expired',
];

let source: EventSource | null = null;
let channel: BroadcastChannel | null = null;
let heartbeat: ReturnType<typeof setInterval> | null = null;
let tabId = '';
let leader = false;
let started = false;
let storage: Storage | null = null;
let leaseKey = LEASE_PREFIX;

function randomId(): string {
  if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
    return crypto.randomUUID();
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

function tabIdentity(): string {
  try {
    const existing = sessionStorage.getItem('filehub.events.tab');
    if (existing) return existing;
    const next = randomId();
    sessionStorage.setItem('filehub.events.tab', next);
    return next;
  } catch {
    return randomId();
  }
}

function openStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function readLease(): LeaderLease | null {
  if (!storage) return null;
  try {
    const raw = storage.getItem(leaseKey);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<LeaderLease>;
    if (typeof parsed.id !== 'string' || typeof parsed.ts !== 'number') return null;
    return { id: parsed.id, ts: parsed.ts };
  } catch {
    return null;
  }
}

function writeLease(): void {
  if (!storage) return;
  try {
    storage.setItem(leaseKey, JSON.stringify({ id: tabId, ts: Date.now() }));
  } catch {
    // Private-mode quota errors must not break realtime updates.
  }
}

function clearLease(): void {
  if (!storage) return;
  try {
    if (readLease()?.id === tabId) storage.removeItem(leaseKey);
  } catch {
    // Ignore storage failures; the lease will expire on its own.
  }
}

function leaseAlive(lease: LeaderLease | null, now = Date.now()): boolean {
  return lease !== null && now - lease.ts < LEASE_MS;
}

const BATCH_TYPES = new Set(['thumbnail.ready', 'thumbnail.failed']);
const BATCH_MS = 80;
let thumbQueue: WireEvent[] = [];
let thumbTimer: ReturnType<typeof setTimeout> | null = null;

function deliver(event: WireEvent): void {
  if (typeof event.type !== 'string' || event.data === null || typeof event.data !== 'object')
    return;
  realtimeActive.set(true);
  realtimeEvent.set({ type: event.type, data: event.data });
}

function broadcast(event: WireEvent): void {
  try {
    channel?.postMessage(event);
  } catch {
    // A closed channel simply means this tab no longer has followers.
  }
}

function flushThumbs(): void {
  thumbTimer = null;
  if (thumbQueue.length === 0) return;
  const events = thumbQueue;
  thumbQueue = [];
  const event =
    events.length === 1
      ? events[0]!
      : { type: 'thumbnail.batch', data: { events } as Record<string, unknown> };
  deliver(event);
  broadcast(event);
}

function enqueue(event: WireEvent): void {
  if (!BATCH_TYPES.has(event.type)) {
    deliver(event);
    broadcast(event);
    return;
  }
  thumbQueue.push(event);
  if (!thumbTimer) thumbTimer = setTimeout(flushThumbs, BATCH_MS);
}

function parseEvent(type: string, raw: string): void {
  let data: Record<string, unknown>;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) return;
    data = parsed as Record<string, unknown>;
  } catch {
    return;
  }
  enqueue({ type, data });
}

let openedOnce = false;
let reconnecting = false;

function openSource(): void {
  if (source || typeof EventSource === 'undefined') return;
  source = new EventSource('/api/v1/events', { withCredentials: true });
  // Treat the stream as active immediately so image requests can use async
  // thumbnails. If the connection fails, `onerror` switches back to sync.
  realtimeActive.set(true);
  for (const type of eventTypes) {
    source.addEventListener(type, (event) => parseEvent(type, (event as MessageEvent).data));
  }
  source.onopen = () => {
    realtimeActive.set(true);
    if (openedOnce && reconnecting) {
      reconnecting = false;
      const event = { type: 'resync', data: { reason: 'reconnect' } };
      deliver(event);
      broadcast(event);
    }
    openedOnce = true;
  };
  source.onerror = () => {
    // EventSource reconnects on its own; ask the app to resync after it opens.
    realtimeActive.set(false);
    reconnecting = true;
  };
}

function closeSource(): void {
  source?.close();
  source = null;
}

function becomeLeader(): void {
  leader = true;
  writeLease();
  openSource();
}

function stepDown(): void {
  leader = false;
  closeSource();
  openedOnce = false;
  reconnecting = false;
  if (channel) realtimeActive.set(true);
}

function elect(): void {
  if (!started) return;
  const lease = readLease();
  if (lease?.id === tabId) {
    becomeLeader();
    return;
  }
  if (!leaseAlive(lease)) {
    becomeLeader();
    return;
  }
  stepDown();
}

function heartbeatTick(): void {
  if (!started) return;
  const lease = readLease();
  if (leader) {
    if (lease && lease.id !== tabId && leaseAlive(lease)) {
      stepDown();
      return;
    }
    writeLease();
    openSource();
    return;
  }
  if (lease?.id === tabId || !leaseAlive(lease)) {
    becomeLeader();
  }
}

function onStorage(event: StorageEvent): void {
  if (event.key !== leaseKey) return;
  if (!started) return;
  if (leader) {
    const lease = readLease();
    if (lease && lease.id !== tabId && leaseAlive(lease)) stepDown();
    return;
  }
  const lease = readLease();
  if (!leaseAlive(lease) || lease?.id === tabId) becomeLeader();
}

function onVisibility(): void {
  if (document.visibilityState === 'visible') heartbeatTick();
}

function onPageHide(): void {
  if (leader) clearLease();
}

export function connectEvents(userId: number): void {
  if (typeof window === 'undefined' || started) return;
  started = true;
  tabId = tabIdentity();
  storage = openStorage();
  leaseKey = `${LEASE_PREFIX}.${userId}`;

  try {
    channel = new BroadcastChannel(`${CHANNEL_PREFIX}.${userId}`);
    channel.onmessage = (message: MessageEvent<WireEvent>) => deliver(message.data);
  } catch {
    channel = null;
    // Without BroadcastChannel a follower would never receive events, so
    // fall back to one EventSource per tab instead of electing a leader.
    storage = null;
  }

  if (!storage) {
    // Without a shared lease every tab keeps its own EventSource, which is
    // the safe fallback. Browsers still cap same-origin connections.
    openSource();
  } else {
    elect();
  }
  heartbeat = setInterval(heartbeatTick, HEARTBEAT_MS);
  window.addEventListener('storage', onStorage);
  document.addEventListener('visibilitychange', onVisibility);
  window.addEventListener('pagehide', onPageHide);
}

export function disconnectEvents(): void {
  started = false;
  if (heartbeat) clearInterval(heartbeat);
  heartbeat = null;
  window.removeEventListener('storage', onStorage);
  document.removeEventListener('visibilitychange', onVisibility);
  window.removeEventListener('pagehide', onPageHide);
  clearLease();
  leader = false;
  closeSource();
  if (thumbTimer) clearTimeout(thumbTimer);
  thumbTimer = null;
  thumbQueue = [];
  openedOnce = false;
  reconnecting = false;
  realtimeActive.set(false);
  try {
    channel?.close();
  } catch {
    // Ignore a channel that was already closed.
  }
  channel = null;
  storage = null;
}
