import { get } from 'svelte/store';
import { ApiError, api, type UploadStatus } from './api';
import { language, t } from './i18n';

export type UploadStatusKind = 'queued' | 'uploading' | 'paused' | 'error';

export interface UploadCandidate {
  file: File;
  relativePath?: string;
}

export interface UploadItem {
  id: string;
  fingerprint: string;
  name: string;
  relativePath: string;
  rootId: number;
  basePath: string;
  directory: string;
  size: number;
  lastModified: number;
  status: UploadStatusKind;
  receivedBytes: number;
  currentBytes: number;
  chunkSize: number;
  totalChunks: number;
  uploadId: string | null;
  error: string;
  priority: boolean;
  preemptedBy: string;
  createdAt: number;
}

interface PersistedUpload {
  uploadId: string;
  rootId: number;
  basePath: string;
  relativePath: string;
  size: number;
  lastModified: number;
  chunkSize: number;
  chunks: number;
  expiresAt: number;
}

interface UploadCallbacks {
  onUploaded?: (rootId: number, directory: string) => void;
  onNotice?: (message: string, kind?: 'info' | 'warning' | 'error') => void;
  onAllDone?: () => void;
}

const CONCURRENCY = 3;
const MAX_RETRIES = 3;

function joinPath(base: string, name: string): string {
  return base ? `${base}/${name}` : name;
}

function normalizedRelative(candidate: UploadCandidate): string {
  const raw = candidate.relativePath || candidate.file.webkitRelativePath || candidate.file.name;
  return raw.replaceAll('\\', '/').replace(/^\/+/, '');
}

function abortError(): DOMException {
  return new DOMException('aborted', 'AbortError');
}

function isAbort(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError';
}

function isRetryable(error: unknown): boolean {
  if (error instanceof ApiError) {
    return (
      error.status === 0 || error.status === 408 || error.status === 429 || error.status >= 500
    );
  }
  return !isAbort(error);
}

function errorText(error: unknown): string {
  if (error instanceof ApiError && error.code === 'PERMISSION_DENIED') {
    return t(get(language), '无写入权限');
  }
  if (error instanceof ApiError && error.code === 'FORBIDDEN') {
    return t(get(language), '当前目录只读');
  }
  if (error instanceof Error && error.message) return error.message;
  return t(get(language), '上传失败');
}

class UploadManager {
  items = $state<UploadItem[]>([]);
  pendingCount = $state(0);

  private userId = 0;
  private getCsrf: () => string = () => '';
  private callbacks: UploadCallbacks = {};
  private controllers = new Map<string, AbortController>();
  private mkdirCache = new Map<string, Promise<void>>();
  private persisted = new Map<string, PersistedUpload>();
  private sequence = 0;
  private suppressPump = false;

  get activeCount(): number {
    return this.items.filter((item) => item.status === 'uploading').length;
  }

  get hasWork(): boolean {
    return this.items.length > 0;
  }

  get pendingHasDirectories(): boolean {
    for (const entry of this.persisted.values()) {
      if (entry.relativePath.includes('/')) return true;
    }
    return false;
  }

  attach(userId: number, getCsrf: () => string, callbacks: UploadCallbacks): void {
    this.userId = userId;
    this.getCsrf = getCsrf;
    this.callbacks = callbacks;
    this.loadPersisted();
  }

  detach(): void {
    for (const controller of this.controllers.values()) controller.abort();
    this.controllers.clear();
    this.mkdirCache.clear();
    this.items = [];
    this.persisted.clear();
    this.pendingCount = 0;
    this.userId = 0;
  }

  enqueue(candidates: UploadCandidate[], rootId: number, basePath: string): void {
    if (!this.userId) return;
    for (const candidate of candidates) {
      const relativePath = normalizedRelative(candidate);
      const parts = relativePath.split('/').filter(Boolean);
      const name = parts.pop() ?? candidate.file.name;
      if (!name) continue;
      const directory = parts.join('/');
      const fingerprint = this.fingerprint(rootId, basePath, relativePath, candidate.file);
      if (this.items.some((item) => item.fingerprint === fingerprint)) continue;
      const saved = this.persisted.get(fingerprint);
      const usable = saved && saved.expiresAt > Math.floor(Date.now() / 1000) ? saved : null;
      this.sequence += 1;
      this.items = [
        ...this.items,
        {
          id: `local_${Date.now()}_${this.sequence}`,
          fingerprint,
          name,
          relativePath,
          rootId,
          basePath,
          directory,
          size: candidate.file.size,
          lastModified: candidate.file.lastModified,
          status: 'queued',
          receivedBytes: 0,
          currentBytes: 0,
          chunkSize: usable?.chunkSize ?? 0,
          totalChunks: usable?.chunks ?? 0,
          uploadId: usable?.uploadId ?? null,
          error: '',
          priority: false,
          preemptedBy: '',
          createdAt: Date.now(),
        },
      ];
      this.fileOf.set(fingerprint, candidate.file);
    }
    this.pump();
  }

  pause(id: string): void {
    const item = this.find(id);
    if (!item || item.status === 'error') return;
    item.status = 'paused';
    item.currentBytes = 0;
    item.preemptedBy = '';
    this.controllers.get(id)?.abort();
    this.controllers.delete(id);
    this.pump();
  }

  resume(id: string): void {
    const item = this.find(id);
    if (!item || item.status === 'uploading') return;
    const active = this.items
      .filter((candidate) => candidate.status === 'uploading' && candidate.id !== id)
      .sort((left, right) => left.receivedBytes - right.receivedBytes);
    if (active.length >= CONCURRENCY) {
      const victim = active.find((candidate) => !candidate.priority) ?? null;
      if (victim) {
        // Pause without letting the scheduler pull in an older queued file:
        // the resumed file must get the freed slot.
        this.suppressPump = true;
        this.pause(victim.id);
        this.suppressPump = false;
        victim.preemptedBy = item.name;
      }
    }
    item.status = 'queued';
    item.error = '';
    item.priority = true;
    this.pump();
  }

  retry(id: string): void {
    const item = this.find(id);
    if (!item) return;
    item.status = 'queued';
    item.error = '';
    item.currentBytes = 0;
    item.priority = true;
    this.pump();
  }

  cancel(id: string): void {
    const item = this.find(id);
    if (!item) return;
    this.controllers.get(id)?.abort();
    this.controllers.delete(id);
    if (item.uploadId) void api.cancelUpload(item.uploadId, this.getCsrf()).catch(() => undefined);
    this.persisted.delete(item.fingerprint);
    this.fileOf.delete(item.fingerprint);
    this.items = this.items.filter((candidate) => candidate.id !== id);
    this.savePersisted();
    this.pump();
  }

  pauseAll(): void {
    for (const item of this.items) {
      if (item.status === 'uploading') {
        this.controllers.get(item.id)?.abort();
        this.controllers.delete(item.id);
      }
      if (item.status === 'uploading' || item.status === 'queued') item.status = 'paused';
      item.preemptedBy = '';
      item.currentBytes = 0;
    }
    this.pump();
  }

  resumeAll(): void {
    for (const item of this.items) {
      if (item.status === 'paused') {
        item.status = 'queued';
        item.priority = false;
      }
    }
    this.pump();
  }

  cancelAll(): void {
    for (const item of this.items) {
      this.controllers.get(item.id)?.abort();
      if (item.uploadId)
        void api.cancelUpload(item.uploadId, this.getCsrf()).catch(() => undefined);
      this.persisted.delete(item.fingerprint);
      this.fileOf.delete(item.fingerprint);
    }
    this.controllers.clear();
    this.items = [];
    this.savePersisted();
  }

  private find(id: string): UploadItem | undefined {
    return this.items.find((item) => item.id === id);
  }

  private pump(): void {
    if (this.suppressPump) return;
    for (const item of this.items) {
      if (item.status === 'paused' && item.preemptedBy) {
        item.status = 'queued';
        item.preemptedBy = '';
        item.priority = false;
      }
    }
    let active = this.activeCount;
    const queued = this.items
      .filter((item) => item.status === 'queued')
      .sort(
        (left, right) =>
          Number(right.priority) - Number(left.priority) || left.createdAt - right.createdAt,
      );
    for (const next of queued) {
      if (active >= CONCURRENCY) break;
      void this.run(next);
      active += 1;
    }
  }

  private async run(item: UploadItem): Promise<void> {
    const controller = new AbortController();
    this.controllers.set(item.id, controller);
    item.status = 'uploading';
    item.error = '';
    item.preemptedBy = '';
    item.currentBytes = 0;
    try {
      for (let attempt = 0; attempt < MAX_RETRIES; attempt += 1) {
        try {
          await this.transfer(item, controller.signal);
          return;
        } catch (error) {
          if (isAbort(error) || item.status !== 'uploading') return;
          if (!isRetryable(error) || attempt === MAX_RETRIES - 1) throw error;
          item.error = errorText(error);
          item.currentBytes = 0;
          await this.sleep(700 * 2 ** attempt, controller.signal);
        }
      }
    } catch (error) {
      if (isAbort(error) || item.status !== 'uploading') return;
      item.status = 'error';
      item.error = errorText(error);
      item.currentBytes = 0;
      this.callbacks.onNotice?.(
        t(get(language), '「{name}」上传失败：{message}', {
          name: item.name,
          message: item.error,
        }),
        'error',
      );
    } finally {
      this.controllers.delete(item.id);
      this.pump();
    }
  }

  private async transfer(item: UploadItem, signal: AbortSignal): Promise<void> {
    const file = this.fileOf.get(item.fingerprint);
    if (!file) throw new Error(t(get(language), '文件已不可用，请重新选择'));

    const targetPath = await this.ensureDirectories(item, signal);
    if (signal.aborted || item.status !== 'uploading') throw abortError();

    let session: UploadStatus;
    if (item.uploadId) {
      try {
        session = await api.uploadStatus(item.uploadId, signal);
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) {
          item.uploadId = null;
          this.persisted.delete(item.fingerprint);
          this.savePersisted();
          throw new ApiError(
            410,
            'UPLOAD_EXPIRED',
            t(get(language), '上传会话已过期，重新上传「{name}」', { name: item.name }),
          );
        }
        throw error;
      }
    } else {
      const created = await api.createUpload(
        item.rootId,
        targetPath,
        item.name,
        file.size,
        this.getCsrf(),
      );
      item.uploadId = created.upload_id;
      item.chunkSize = created.chunk_size;
      item.totalChunks = created.chunks;
      session = {
        upload_id: created.upload_id,
        total_size: file.size,
        chunk_size: created.chunk_size,
        received: 0,
        received_chunks: [],
        expires_at: created.expires_at,
      };
      this.persistItem(item, created.expires_at, created.chunk_size, created.chunks);
      // The user may have cancelled while the session was being created. The
      // early cancel ran before the id existed, so clean it up here.
      if (!this.items.some((candidate) => candidate.id === item.id)) {
        this.persisted.delete(item.fingerprint);
        this.savePersisted();
        await api.cancelUpload(created.upload_id, this.getCsrf()).catch(() => undefined);
        throw abortError();
      }
    }

    const received = new Set(session.received_chunks);
    item.chunkSize = session.chunk_size;
    item.totalChunks = file.size === 0 ? 0 : Math.ceil(file.size / session.chunk_size);
    item.receivedBytes = this.receivedBytes(item, received);

    if (file.size > 0) {
      for (let index = 0; index < item.totalChunks; index += 1) {
        if (signal.aborted || item.status !== 'uploading') throw abortError();
        const start = index * item.chunkSize;
        const end = Math.min(start + item.chunkSize, file.size);
        if (!received.has(index)) {
          await api.uploadChunk(
            item.uploadId ?? '',
            index,
            file.slice(start, end),
            this.getCsrf(),
            signal,
            (loaded) => {
              if (item.status === 'uploading') item.currentBytes = loaded;
            },
          );
          item.receivedBytes += end - start;
          item.currentBytes = 0;
        }
      }
    }

    if (signal.aborted || item.status !== 'uploading') throw abortError();
    await api.completeUpload(item.uploadId ?? '', this.getCsrf());
    this.complete(item);
  }

  private async ensureDirectories(item: UploadItem, signal: AbortSignal): Promise<string> {
    let parent = item.basePath;
    if (!item.directory) return parent;
    for (const segment of item.directory.split('/').filter(Boolean)) {
      if (signal.aborted) throw abortError();
      const key = `${this.userId}:${item.rootId}:${parent}:${segment}`;
      let pending = this.mkdirCache.get(key);
      if (!pending) {
        pending = api
          .mkdir(item.rootId, parent, segment, this.getCsrf(), true)
          .then(() => undefined)
          .catch((error: unknown) => {
            this.mkdirCache.delete(key);
            throw error;
          });
        this.mkdirCache.set(key, pending);
      }
      await pending;
      parent = joinPath(parent, segment);
    }
    return parent;
  }

  private receivedBytes(item: UploadItem, received: Set<number>): number {
    let total = 0;
    for (let index = 0; index < item.totalChunks; index += 1) {
      if (!received.has(index)) continue;
      const start = index * item.chunkSize;
      total += Math.min(item.chunkSize, item.size - start);
    }
    return total;
  }

  private complete(item: UploadItem): void {
    this.controllers.delete(item.id);
    this.persisted.delete(item.fingerprint);
    this.fileOf.delete(item.fingerprint);
    this.items = this.items.filter((candidate) => candidate.id !== item.id);
    this.savePersisted();
    this.callbacks.onUploaded?.(item.rootId, joinPath(item.basePath, item.directory));
    if (this.items.length === 0) this.callbacks.onAllDone?.();
  }

  private sleep(ms: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(resolve, ms);
      signal.addEventListener(
        'abort',
        () => {
          clearTimeout(timer);
          reject(abortError());
        },
        { once: true },
      );
    });
  }

  private fingerprint(rootId: number, basePath: string, relativePath: string, file: File): string {
    return [this.userId, rootId, basePath, relativePath, file.size, file.lastModified].join('|');
  }

  private storageKey(): string {
    return `filehub.uploads.pending.${this.userId}`;
  }

  private loadPersisted(): void {
    this.persisted.clear();
    try {
      const raw = localStorage.getItem(this.storageKey());
      if (!raw) {
        this.pendingCount = 0;
        return;
      }
      const parsed = JSON.parse(raw) as Record<string, PersistedUpload>;
      const now = Math.floor(Date.now() / 1000);
      for (const [key, value] of Object.entries(parsed)) {
        if (value.expiresAt > now) this.persisted.set(key, value);
      }
      this.pendingCount = this.persisted.size;
    } catch (error) {
      console.warn('failed to load pending uploads', error);
      this.pendingCount = 0;
    }
  }

  private persistItem(
    item: UploadItem,
    expiresAt: number,
    chunkSize: number,
    chunks: number,
  ): void {
    if (!item.uploadId) return;
    this.persisted.set(item.fingerprint, {
      uploadId: item.uploadId,
      rootId: item.rootId,
      basePath: item.basePath,
      relativePath: item.relativePath,
      size: item.size,
      lastModified: item.lastModified,
      chunkSize,
      chunks,
      expiresAt,
    });
    this.pendingCount = this.persisted.size;
    this.savePersisted();
  }

  private savePersisted(): void {
    try {
      const value: Record<string, PersistedUpload> = {};
      for (const [key, entry] of this.persisted) value[key] = entry;
      localStorage.setItem(this.storageKey(), JSON.stringify(value));
    } catch (error) {
      console.warn('failed to save pending uploads', error);
    }
    this.pendingCount = this.persisted.size;
  }

  private fileOf = new Map<string, File>();
}

export const uploadManager = new UploadManager();
