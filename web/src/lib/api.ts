export type Role = 'admin' | 'user';

export interface UserPreferences {
  theme: 'light' | 'dark';
  language: 'zh-CN' | 'en';
  show_hidden: boolean;
  show_thumbnails: boolean;
  upload_conflict: 'rename' | 'overwrite' | 'skip';
  terminal_enabled: boolean;
  editor_enabled: boolean;
  custom_css_root_id: number | null;
  custom_css_path: string | null;
  custom_css_version: number;
  avatar_version: number;
}

export interface SystemFeatures {
  thumbnails: boolean;
  editor: boolean;
  terminal: boolean;
}

export interface PreferencesResponse {
  preferences: UserPreferences;
  system: SystemFeatures;
}

export interface User {
  id: number;
  username: string;
  role: Role;
  quota_bytes: number | null;
  disabled: boolean;
  created_at: number;
  updated_at: number;
  last_login_at: number | null;
}

export interface AuthResponse {
  user: User;
  csrf_token: string;
  auth: 'session' | 'token';
}

export interface SetupStatus {
  initialized: boolean;
}

export interface SettingsResponse {
  settings: Record<string, unknown>;
  sources: Record<string, string>;
  restart_required: string[];
}

export interface Root {
  id: number;
  alias: string;
  read_only: boolean;
}

export interface Entry {
  item_id?: number;
  name: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: number;
  created_at: number;
  mime_type: string;
  has_preview: boolean;
  symlink_state?: 'ok' | 'broken' | 'denied';
  symlink_target?: string | null;
}

export interface ListResponse {
  root: Root;
  path: string;
  read_only: boolean;
  entries: Entry[];
  next_cursor: string | null;
}

export interface UploadCreated {
  upload_id: string;
  chunk_size: number;
  chunks: number;
  expires_at: number;
}

export interface ServerPath {
  id: number;
  alias: string;
  real_path: string;
  created_at: number;
}

export interface ShareInfo {
  id: number;
  kind?: 'single' | 'collection';
  item_count?: number;
  real_path: string;
  is_dir: boolean;
  has_password: boolean;
  expires_at: number | null;
  read_only: boolean;
  revoked: boolean;
  created_at: number;
  visit_count: number;
}

export interface AdminRoot {
  id: number;
  alias: string;
  real_path: string;
  read_only: boolean;
  sort_order: number;
  covered_by: string | null;
  covered_by_id: number | null;
}

export interface AuditEntry {
  id: number;
  username: string | null;
  action: string;
  path: string | null;
  detail: string | null;
  ip: string | null;
  created_at: number;
}

export interface ApiTokenInfo {
  id: number;
  name: string;
  prefix: string;
  scope: 'read' | 'write';
  last_used_at: number | null;
  expires_at: number | null;
  created_at: number;
}

export interface TrustedLink {
  id: number;
  link_path: string;
  target_path: string;
  access: 'inherit' | 'read_only' | 'read_write';
  created_at: number;
}

export interface Usage {
  quota_bytes: number | null;
  used_bytes: number | null;
  disks: Array<{ name: string; total: number; used: number; available: number }>;
  computed_at?: number | null;
  stale?: boolean;
  computing?: boolean;
}

export type JobStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';

export interface JobInfo {
  id: string;
  user_id: number;
  kind: string;
  status: JobStatus;
  progress: number;
  total: number;
  bytes_done: number;
  bytes_total: number;
  result: unknown;
  error: string | null;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
}

export interface TransferJobResult {
  items: Array<{
    source: string;
    target?: string;
    status: 'completed' | 'failed' | 'skipped';
    size?: number;
    error?: string;
  }>;
  failed: number;
  partial: boolean;
}

export interface DirSizeInfo {
  status: 'ready' | 'queued' | 'running' | 'error';
  size: number | null;
  indexed: boolean;
  partial?: boolean;
  stale?: boolean;
  refreshing?: boolean;
  error?: string | null;
  job_id?: string | null;
}

export interface FileContent {
  content: string;
  etag: string;
  size: number;
  modified: number;
}

export interface ShareEntry {
  item_id?: number;
  name: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: number;
  created_at?: number;
  mime_type?: string;
  has_preview?: boolean;
}

export interface ShareMeta {
  collection?: boolean;
  name: string;
  path: string;
  is_dir: boolean;
  read_only: boolean;
  size?: number;
  modified?: number;
  mime_type?: string;
  has_preview?: boolean;
  entries?: ShareEntry[];
  next_cursor?: string | null;
}

export interface FileProperties {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  symlink_state?: 'ok' | 'broken' | 'denied';
  symlink_target?: string | null;
  mime_type: string;
  size: number;
  created?: number | null;
  modified?: number | null;
  accessed?: number | null;
  mode?: string;
  image?: {
    width: number;
    height: number;
    format: string;
    exif?: Record<string, string | number>;
  } | null;
}

export interface SearchEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
  modified: number;
  created?: number;
  mime_type?: string;
  has_preview?: boolean;
  rootId?: number;
}

export interface DeletePreview {
  token: string;
  expires_at: number;
  items: Array<{ relative: string; is_dir: boolean }>;
  confirm_code: string;
}

interface ApiErrorBody {
  error?: {
    code?: string;
    message?: string;
  };
}

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
    this.code = code;
  }
}

const BASE = '/api/v1';

async function request<T>(path: string, init: RequestInit = {}, csrf?: string): Promise<T> {
  const headers = new Headers(init.headers);
  headers.set('Accept', 'application/json');

  if (init.body) {
    headers.set('Content-Type', 'application/json');
  }
  if (csrf) {
    headers.set('X-CSRF-Token', csrf);
  }

  const response = await fetch(`${BASE}${path}`, {
    ...init,
    headers,
    credentials: 'include',
  });

  if (!response.ok) {
    let body: unknown = null;
    try {
      body = await response.json();
    } catch (error) {
      console.warn('failed to parse API error response', error);
    }

    const parsed = parseErrorBody(body);
    throw new ApiError(
      response.status,
      parsed?.code ?? 'INTERNAL_ERROR',
      parsed?.message ?? response.statusText ?? 'Request failed',
    );
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

function parseErrorBody(body: unknown): ApiErrorBody['error'] | null {
  if (!body || typeof body !== 'object') {
    return null;
  }

  const error = (body as ApiErrorBody).error;
  if (!error || typeof error !== 'object') {
    return null;
  }

  return error;
}

async function apiErrorFromResponse(response: Response): Promise<ApiError> {
  let body: unknown = null;
  try {
    body = await response.json();
  } catch (error) {
    console.warn('failed to parse API error response', error);
  }

  const parsed = parseErrorBody(body);
  return new ApiError(
    response.status,
    parsed?.code ?? 'INTERNAL_ERROR',
    parsed?.message ?? response.statusText ?? 'Request failed',
  );
}

export const api = {
  setupStatus(): Promise<SetupStatus> {
    return request<SetupStatus>('/setup/status');
  },

  setup(payload: { token: string; username: string; password: string }): Promise<AuthResponse> {
    return request<AuthResponse>('/setup', {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  login(payload: { username: string; password: string }): Promise<AuthResponse> {
    return request<AuthResponse>('/login', {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  logout(csrf: string): Promise<void> {
    return request<void>('/logout', { method: 'POST' }, csrf);
  },

  me(): Promise<AuthResponse> {
    return request<AuthResponse>('/me');
  },

  roots(): Promise<{ roots: Root[] }> {
    return request<{ roots: Root[] }>('/roots');
  },

  list(
    root: number,
    path: string,
    sort = 'name',
    order = 'asc',
    sizes = false,
    cursor?: string,
    limit = 200,
    signal?: AbortSignal,
  ): Promise<ListResponse> {
    const query = new URLSearchParams({
      root: String(root),
      path,
      sort,
      order,
      sizes: String(sizes),
      limit: String(limit),
    });
    if (cursor) query.set('cursor', cursor);
    return request<ListResponse>(`/list?${query.toString()}`, { signal });
  },

  properties(root: number, path: string): Promise<FileProperties> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<FileProperties>(`/properties?${query.toString()}`);
  },

  dirSize(root: number, path: string, signal?: AbortSignal): Promise<DirSizeInfo> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<DirSizeInfo>(`/fs/dir-size?${query.toString()}`, { signal });
  },

  downloadUrl(root: number, path: string): string {
    const query = new URLSearchParams({ root: String(root), path });
    return `${BASE}/download?${query.toString()}`;
  },

  thumbUrl(root: number, path: string, width = 256, version?: number): string {
    const query = new URLSearchParams({
      root: String(root),
      path,
      w: String(width),
    });
    if (version !== undefined) query.set('v', String(version));
    return `${BASE}/thumb?${query.toString()}`;
  },

  thumbStatus(
    root: number,
    path: string,
    width = 320,
  ): Promise<{
    state: 'ready' | 'failed' | 'missing';
    reason?: string | null;
    message?: string | null;
  }> {
    const query = new URLSearchParams({ root: String(root), path, w: String(width) });
    return request(`/thumb/status?${query.toString()}`);
  },

  retryThumb(
    root: number,
    path: string,
    width: number,
    csrf: string,
  ): Promise<{ cleared: number }> {
    return request(
      '/thumb/retry',
      { method: 'POST', body: JSON.stringify({ root, path, w: width }) },
      csrf,
    );
  },

  async publicShare(
    token: string,
    path: string,
    ticket?: string,
    cursor?: string,
    limit = 200,
  ): Promise<ShareMeta> {
    const query = new URLSearchParams({ path, limit: String(limit) });
    if (cursor) query.set('cursor', cursor);
    const headers = new Headers({ Accept: 'application/json' });
    if (ticket) headers.set('X-Share-Ticket', ticket);
    const response = await fetch(`${BASE}/public/shares/${token}?${query.toString()}`, {
      headers,
      credentials: 'include',
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
    return (await response.json()) as ShareMeta;
  },

  async unlockShare(
    token: string,
    password: string,
  ): Promise<{ ticket: string; expires_at: number }> {
    return request<{ ticket: string; expires_at: number }>(`/public/shares/${token}/unlock`, {
      method: 'POST',
      body: JSON.stringify({ password }),
    });
  },

  async uploadShared(
    token: string,
    path: string,
    filename: string,
    file: Blob,
    ticket?: string,
  ): Promise<{ name: string }> {
    const query = new URLSearchParams({ path, filename });
    const headers = new Headers({ 'Content-Type': 'application/octet-stream' });
    if (ticket) headers.set('X-Share-Ticket', ticket);
    const response = await fetch(`${BASE}/public/shares/${token}/upload?${query.toString()}`, {
      method: 'POST',
      headers,
      body: file,
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
    return response.json() as Promise<{ name: string }>;
  },

  publicZipFormUrl(token: string): string {
    return `${BASE}/public/shares/${token}/zip/form`;
  },

  publicThumbUrl(token: string, path: string, width = 320, ticket?: string, item?: number): string {
    const query = new URLSearchParams({ path, w: String(width) });
    if (ticket) query.set('ticket', ticket);
    if (item !== undefined) query.set('item', String(item));
    return `${BASE}/public/shares/${token}/thumb?${query.toString()}`;
  },

  publicDownloadUrl(
    token: string,
    path: string,
    ticket?: string,
    inline = false,
    item?: number,
  ): string {
    const query = new URLSearchParams({ path });
    if (ticket) query.set('ticket', ticket);
    if (inline) query.set('inline', 'true');
    if (item !== undefined) query.set('item', String(item));
    return `${BASE}/public/shares/${token}/download?${query.toString()}`;
  },

  publicZipPreflight(
    token: string,
    payload: { path?: string; paths?: string[]; item_ids?: number[]; ticket?: string },
  ): Promise<{
    allowed: boolean;
    files?: number;
    bytes?: number;
    message?: string;
    max_bytes: number;
    max_files: number;
  }> {
    return request(`/public/shares/${token}/zip/preflight`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  createShareUpload(
    token: string,
    payload: {
      path: string;
      filename: string;
      total_size: number;
      chunk_size?: number;
      ticket?: string;
    },
  ): Promise<{ upload_id: string; chunk_size: number; chunks: number; expires_at: number }> {
    return request(`/public/shares/${token}/uploads`, {
      method: 'POST',
      body: JSON.stringify(payload),
    });
  },

  shareUploadStatus(
    token: string,
    id: string,
    ticket?: string,
  ): Promise<{
    upload_id: string;
    received: number;
    total_size: number;
    chunks: number;
    received_chunks: number[];
    expires_at: number;
  }> {
    const query = new URLSearchParams();
    if (ticket) query.set('ticket', ticket);
    return request(`/public/shares/${token}/uploads/${id}?${query.toString()}`);
  },

  async uploadShareChunk(
    token: string,
    id: string,
    index: number,
    chunk: Blob,
    ticket?: string,
  ): Promise<{ received: number; chunk: number }> {
    const query = new URLSearchParams();
    if (ticket) query.set('ticket', ticket);
    const response = await fetch(
      `${BASE}/public/shares/${token}/uploads/${id}/chunks/${index}?${query.toString()}`,
      {
        method: 'PUT',
        headers: { 'Content-Type': 'application/octet-stream' },
        body: chunk,
      },
    );
    if (!response.ok) throw await apiErrorFromResponse(response);
    return response.json() as Promise<{ received: number; chunk: number }>;
  },

  completeShareUpload(token: string, id: string, ticket?: string): Promise<{ name: string }> {
    const query = new URLSearchParams();
    if (ticket) query.set('ticket', ticket);
    return request(`/public/shares/${token}/uploads/${id}/complete?${query.toString()}`, {
      method: 'POST',
      body: JSON.stringify({}),
    });
  },

  async cancelShareUpload(token: string, id: string, ticket?: string): Promise<void> {
    const query = new URLSearchParams();
    if (ticket) query.set('ticket', ticket);
    const response = await fetch(
      `${BASE}/public/shares/${token}/uploads/${id}?${query.toString()}`,
      { method: 'DELETE' },
    );
    if (!response.ok) throw await apiErrorFromResponse(response);
  },

  async downloadShared(token: string, path: string, ticket?: string): Promise<Blob> {
    const query = new URLSearchParams({ path });
    const headers = new Headers();
    if (ticket) headers.set('X-Share-Ticket', ticket);
    const response = await fetch(`${BASE}/public/shares/${token}/download?${query.toString()}`, {
      headers,
      credentials: 'include',
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
    return response.blob();
  },

  probeFile(
    root: number,
    path: string,
  ): Promise<{ is_text: boolean; reason: string; size: number }> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<{ is_text: boolean; reason: string; size: number }>(
      `/file/probe?${query.toString()}`,
    );
  },

  getFile(root: number, path: string): Promise<FileContent> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<FileContent>(`/file?${query.toString()}`);
  },

  saveFile(
    root: number,
    path: string,
    content: string,
    etag: string,
    csrf: string,
  ): Promise<{ etag: string; size: number }> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<{ etag: string; size: number }>(
      `/file?${query.toString()}`,
      {
        method: 'PUT',
        headers: { 'If-Match': etag },
        body: JSON.stringify({ content }),
      },
      csrf,
    );
  },

  listUsers(): Promise<{ users: User[] }> {
    return request<{ users: User[] }>('/admin/users');
  },

  createUser(
    payload: { username: string; password: string; role?: string; quota_bytes?: number | null },
    csrf: string,
  ): Promise<{ user: User }> {
    return request<{ user: User }>(
      '/admin/users',
      { method: 'POST', body: JSON.stringify(payload) },
      csrf,
    );
  },

  updateUser(
    id: number,
    payload: {
      password?: string;
      role?: string;
      quota_bytes?: number;
      clear_quota?: boolean;
      disabled?: boolean;
    },
    csrf: string,
  ): Promise<{ user: User }> {
    return request<{ user: User }>(
      `/admin/users/${id}`,
      { method: 'PUT', body: JSON.stringify(payload) },
      csrf,
    );
  },

  deleteUser(id: number, csrf: string): Promise<void> {
    return request<void>(`/admin/users/${id}`, { method: 'DELETE' }, csrf);
  },

  listUserRoots(userId: number): Promise<{ roots: AdminRoot[] }> {
    return request<{ roots: AdminRoot[] }>(`/admin/users/${userId}/roots`);
  },

  addRoot(
    userId: number,
    payload: { path: string; alias: string; read_only?: boolean; sort_order?: number },
    csrf: string,
  ): Promise<{ root_id: number; real_path: string }> {
    return request<{ root_id: number; real_path: string }>(
      `/admin/users/${userId}/roots`,
      { method: 'POST', body: JSON.stringify(payload) },
      csrf,
    );
  },

  updateRoot(
    userId: number,
    rootId: number,
    payload: { alias: string; read_only: boolean; sort_order?: number },
    csrf: string,
  ): Promise<{ root: AdminRoot }> {
    return request<{ root: AdminRoot }>(
      `/admin/users/${userId}/roots/${rootId}`,
      { method: 'PUT', body: JSON.stringify(payload) },
      csrf,
    );
  },

  userUsage(userId: number): Promise<{ used_bytes: number }> {
    return request<{ used_bytes: number }>(`/admin/users/${userId}/usage`);
  },

  adminUsage(): Promise<{
    users: Record<
      number,
      {
        used_bytes: number | null;
        quota_bytes: number | null;
        computed_at: number | null;
        computing: boolean;
      }
    >;
  }> {
    return request('/admin/users/usage');
  },

  adminStatus(): Promise<{
    jobs: Array<{ kind: string; queued: number; running: number }>;
    job_counts: Array<{ kind: string; status: string; count: number }>;
    directory_sizes: Array<{ status: string; count: number }>;
    database: { page_count: number; page_size: number; freelist_count: number; wal_bytes: number };
    slow_requests: Array<{
      method: string;
      path: string;
      status: number;
      duration_ms: number;
      request_id: string | null;
      at: number;
    }>;
    index: {
      scheduled: boolean;
      quick_scan_minutes: number;
      full_scan_hours: number;
      last_quick_scan: number | null;
      last_full_scan: number | null;
    };
  }> {
    return request('/admin/status');
  },

  deleteRoot(userId: number, rootId: number, csrf: string): Promise<void> {
    return request<void>(`/admin/users/${userId}/roots/${rootId}`, { method: 'DELETE' }, csrf);
  },

  listTrustedLinks(): Promise<{ links: TrustedLink[] }> {
    return request<{ links: TrustedLink[] }>('/admin/trusted-links');
  },

  createTrustedLink(
    payload: { link_path: string; target_path: string; access?: string },
    csrf: string,
  ): Promise<{ id: number }> {
    return request<{ id: number }>(
      '/admin/trusted-links',
      { method: 'POST', body: JSON.stringify(payload) },
      csrf,
    );
  },

  deleteTrustedLink(id: number, csrf: string): Promise<void> {
    return request<void>(`/admin/trusted-links/${id}`, { method: 'DELETE' }, csrf);
  },

  cmd(
    root: number,
    cwd: string,
    line: string,
    confirm: boolean,
    csrf: string,
  ): Promise<{ output: string }> {
    return request<{ output: string }>(
      '/cmd',
      {
        method: 'POST',
        body: JSON.stringify({ root, cwd, line, confirm }),
      },
      csrf,
    );
  },

  mkdir(
    root: number,
    path: string,
    name: string,
    csrf: string,
    recursive = false,
  ): Promise<{ path: string }> {
    return request<{ path: string }>(
      '/fs/mkdir',
      { method: 'POST', body: JSON.stringify({ root, path, name, recursive }) },
      csrf,
    );
  },

  touch(root: number, path: string, name: string, csrf: string): Promise<{ path: string }> {
    return request<{ path: string }>(
      '/fs/touch',
      { method: 'POST', body: JSON.stringify({ root, path, name }) },
      csrf,
    );
  },

  rename(root: number, path: string, newName: string, csrf: string): Promise<{ path: string }> {
    return request<{ path: string }>(
      '/fs/rename',
      { method: 'POST', body: JSON.stringify({ root, path, new_name: newName }) },
      csrf,
    );
  },

  transfer(
    mode: 'copy' | 'move',
    sourceRoot: number,
    paths: string[],
    destRoot: number,
    destPath: string,
    csrf: string,
  ): Promise<{ items: Array<{ source: string; target: string }> }> {
    return request<{ items: Array<{ source: string; target: string }> }>(
      mode === 'copy' ? '/fs/copy' : '/fs/move',
      {
        method: 'POST',
        body: JSON.stringify({
          source_root: sourceRoot,
          paths,
          dest_root: destRoot,
          dest_path: destPath,
        }),
      },
      csrf,
    );
  },

  transferJob(
    mode: 'copy' | 'move',
    sourceRoot: number,
    paths: string[],
    destRoot: number,
    destPath: string,
    csrf: string,
  ): Promise<{ job_id: string; status: string }> {
    return request<{ job_id: string; status: string }>(
      '/fs/transfer-jobs',
      {
        method: 'POST',
        body: JSON.stringify({
          source_root: sourceRoot,
          paths,
          dest_root: destRoot,
          dest_path: destPath,
          move_items: mode === 'move',
        }),
      },
      csrf,
    );
  },

  job(id: string, signal?: AbortSignal): Promise<JobInfo> {
    return request<JobInfo>(`/jobs/${id}`, { signal });
  },

  cancelJob(id: string, csrf: string): Promise<JobInfo> {
    return request<JobInfo>(`/jobs/${id}`, { method: 'DELETE' }, csrf);
  },

  previewThumbUrl(root: number, path: string, edge = 2048): string {
    const query = new URLSearchParams({
      root: String(root),
      path,
      fit: 'short',
      edge: String(edge),
    });
    return `${BASE}/thumb?${query.toString()}`;
  },

  previewUrl(root: number, path: string): string {
    const query = new URLSearchParams({ root: String(root), path, inline: 'true' });
    return `${BASE}/download?${query.toString()}`;
  },

  changePassword(currentPassword: string, newPassword: string, csrf: string): Promise<void> {
    return request<void>(
      '/me/password',
      {
        method: 'POST',
        body: JSON.stringify({
          current_password: currentPassword,
          new_password: newPassword,
        }),
      },
      csrf,
    );
  },

  listShares(
    cursor?: string,
    limit = 100,
  ): Promise<{ shares: ShareInfo[]; next_cursor: string | null }> {
    const query = new URLSearchParams({ limit: String(limit) });
    if (cursor) query.set('cursor', cursor);
    return request<{ shares: ShareInfo[]; next_cursor: string | null }>(
      `/shares?${query.toString()}`,
    );
  },

  createShare(
    payload: {
      root: number;
      path?: string;
      paths?: string[];
      password?: string;
      expires_at?: number | null;
      read_only?: boolean;
    },
    csrf: string,
  ): Promise<{ id: number; token: string; url: string; expires_at: number | null }> {
    return request<{ id: number; token: string; url: string; expires_at: number | null }>(
      '/shares',
      { method: 'POST', body: JSON.stringify(payload) },
      csrf,
    );
  },

  revokeShare(id: number, csrf: string): Promise<void> {
    return request<void>(`/shares/${id}`, { method: 'DELETE' }, csrf);
  },

  purgeShare(id: number, csrf: string): Promise<void> {
    return request<void>(`/shares/${id}/purge`, { method: 'DELETE' }, csrf);
  },

  updateShare(
    id: number,
    payload: {
      read_only?: boolean;
      expires_at?: number | null;
      clear_expires?: boolean;
      password?: string;
      clear_password?: boolean;
      revoked?: boolean;
    },
    csrf: string,
  ): Promise<void> {
    return request<void>(`/shares/${id}`, { method: 'PUT', body: JSON.stringify(payload) }, csrf);
  },

  rotateShare(id: number, csrf: string): Promise<{ token: string; url: string }> {
    return request<{ token: string; url: string }>(
      `/shares/${id}/rotate`,
      { method: 'POST' },
      csrf,
    );
  },

  listAudit(
    limit = 100,
    cursor?: string,
    signal?: AbortSignal,
  ): Promise<{ entries: AuditEntry[]; next_cursor: string | null }> {
    const query = new URLSearchParams({ limit: String(limit) });
    if (cursor) query.set('cursor', cursor);
    return request<{
      entries: AuditEntry[];
      next_cursor: string | null;
    }>(`/admin/audit?${query.toString()}`, { signal });
  },

  getPreferences(): Promise<PreferencesResponse> {
    return request<PreferencesResponse>('/me/preferences');
  },

  updatePreferences(
    patch: Partial<{
      theme: 'light' | 'dark';
      language: 'zh-CN' | 'en';
      show_hidden: boolean;
      show_thumbnails: boolean;
      upload_conflict: 'rename' | 'overwrite' | 'skip';
      terminal_enabled: boolean;
      editor_enabled: boolean;
      clear_custom_css: boolean;
      custom_css_root_id: number;
      custom_css_path: string;
    }>,
    csrf: string,
  ): Promise<PreferencesResponse> {
    return request<PreferencesResponse>(
      '/me/preferences',
      { method: 'PATCH', body: JSON.stringify(patch) },
      csrf,
    );
  },

  avatarUrl(version: number): string {
    return `${BASE}/me/avatar?v=${version}`;
  },

  customCssUrl(version: number): string {
    return `${BASE}/me/custom-css?v=${version}`;
  },

  async uploadAvatar(file: Blob, csrf: string): Promise<PreferencesResponse> {
    const response = await fetch(`${BASE}/me/avatar`, {
      method: 'POST',
      credentials: 'include',
      headers: {
        'Content-Type': file.type,
        'X-CSRF-Token': csrf,
      },
      body: await file.arrayBuffer(),
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
    return response.json() as Promise<PreferencesResponse>;
  },

  deleteAvatar(csrf: string): Promise<void> {
    return request<void>('/me/avatar', { method: 'DELETE' }, csrf);
  },

  listTokens(): Promise<{ tokens: ApiTokenInfo[] }> {
    return request<{ tokens: ApiTokenInfo[] }>('/auth/tokens');
  },

  createToken(
    name: string,
    scope: 'read' | 'write',
    csrf: string,
  ): Promise<{ token: string; id: number; name: string }> {
    return request<{ token: string; id: number; name: string }>(
      '/auth/tokens',
      { method: 'POST', body: JSON.stringify({ name, scope }) },
      csrf,
    );
  },

  revokeToken(id: number, csrf: string): Promise<void> {
    return request<void>(`/auth/tokens/${id}`, { method: 'DELETE' }, csrf);
  },

  listServerPaths(): Promise<{ paths: ServerPath[] }> {
    return request<{ paths: ServerPath[] }>('/admin/server-paths');
  },

  addServerPath(
    payload: { path: string; alias: string },
    csrf: string,
  ): Promise<{ id: number; real_path: string }> {
    return request<{ id: number; real_path: string }>(
      '/admin/server-paths',
      { method: 'POST', body: JSON.stringify(payload) },
      csrf,
    );
  },

  updateServerPath(id: number, alias: string, csrf: string): Promise<{ alias: string }> {
    return request<{ alias: string }>(
      `/admin/server-paths/${id}`,
      { method: 'PUT', body: JSON.stringify({ alias }) },
      csrf,
    );
  },

  deleteServerPath(id: number, csrf: string): Promise<void> {
    return request<void>(`/admin/server-paths/${id}`, { method: 'DELETE' }, csrf);
  },

  usage(signal?: AbortSignal): Promise<Usage> {
    return request<Usage>('/usage', { signal });
  },

  index(
    root: number,
    path: string,
    recursive: boolean,
    csrf: string,
  ): Promise<{ job_id: string; status: JobStatus; indexing: boolean; covered_by: string | null }> {
    return request<{
      job_id: string;
      status: JobStatus;
      indexing: boolean;
      covered_by: string | null;
    }>('/index', { method: 'POST', body: JSON.stringify({ root, path, recursive }) }, csrf);
  },

  indexStatus(
    root: number,
    path = '',
    signal?: AbortSignal,
  ): Promise<{
    status: string;
    entry_count: number;
    indexed_at: number | null;
    total_size_bytes: number;
    partial: boolean;
    job: JobInfo | null;
  }> {
    const parameters = new URLSearchParams({ root: String(root), path });
    return request(`/index/status?${parameters.toString()}`, { signal });
  },

  search(
    root: number,
    query: string,
    limit = 100,
    signal?: AbortSignal,
    filters: {
      kind?: string;
      minSize?: number;
      maxSize?: number;
      modifiedAfter?: number;
      modifiedBefore?: number;
      isDir?: boolean;
    } = {},
  ): Promise<{ entries: SearchEntry[] }> {
    const parameters = new URLSearchParams({
      root: String(root),
      q: query,
      limit: String(limit),
    });
    if (filters.kind) parameters.set('kind', filters.kind);
    if (filters.minSize !== undefined) parameters.set('min_size', String(filters.minSize));
    if (filters.maxSize !== undefined) parameters.set('max_size', String(filters.maxSize));
    if (filters.modifiedAfter !== undefined) {
      parameters.set('modified_after', String(filters.modifiedAfter));
    }
    if (filters.modifiedBefore !== undefined) {
      parameters.set('modified_before', String(filters.modifiedBefore));
    }
    if (filters.isDir !== undefined) parameters.set('is_dir', String(filters.isDir));
    return request<{ entries: SearchEntry[] }>(`/search?${parameters.toString()}`, { signal });
  },

  createUpload(
    root: number,
    path: string,
    filename: string,
    totalSize: number,
    csrf: string,
  ): Promise<UploadCreated> {
    return request<UploadCreated>(
      '/uploads',
      {
        method: 'POST',
        body: JSON.stringify({
          root,
          path,
          filename,
          total_size: totalSize,
        }),
      },
      csrf,
    );
  },

  async uploadChunk(
    uploadId: string,
    index: number,
    chunk: Blob,
    csrf: string,
    signal?: AbortSignal,
  ): Promise<void> {
    const response = await fetch(`${BASE}/uploads/${uploadId}/chunks/${index}`, {
      method: 'PUT',
      credentials: 'include',
      headers: {
        'Content-Type': 'application/octet-stream',
        'X-CSRF-Token': csrf,
      },
      body: chunk,
      signal,
    });

    if (!response.ok) {
      throw await apiErrorFromResponse(response);
    }
  },

  async uploadBrandingIcon(
    file: File,
    csrf: string,
  ): Promise<{ version: number; icon_url: string }> {
    const response = await fetch(`${BASE}/admin/branding/icon`, {
      method: 'POST',
      credentials: 'include',
      headers: {
        'Content-Type': file.type || 'application/octet-stream',
        'X-CSRF-Token': csrf,
      },
      body: file,
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
    return response.json() as Promise<{ version: number; icon_url: string }>;
  },

  deleteBrandingIcon(csrf: string): Promise<{ deleted: boolean }> {
    return request('/admin/branding/icon', { method: 'DELETE' }, csrf);
  },

  async cancelUpload(uploadId: string, csrf: string): Promise<void> {
    const response = await fetch(`${BASE}/uploads/${uploadId}`, {
      method: 'DELETE',
      credentials: 'include',
      headers: { 'X-CSRF-Token': csrf },
    });
    if (!response.ok) throw await apiErrorFromResponse(response);
  },

  completeUpload(uploadId: string, csrf: string): Promise<{ path: string; size: number }> {
    return request<{ path: string; size: number }>(
      `/uploads/${uploadId}/complete`,
      { method: 'POST' },
      csrf,
    );
  },

  deletePreview(root: number, paths: string[], csrf: string): Promise<DeletePreview> {
    return request<DeletePreview>(
      '/fs/delete/preview',
      { method: 'POST', body: JSON.stringify({ root, paths }) },
      csrf,
    );
  },

  deletePaths(
    token: string,
    confirmName: string | undefined,
    csrf: string,
    confirmCode?: string,
  ): Promise<void> {
    return request<void>(
      '/fs/delete',
      {
        method: 'POST',
        body: JSON.stringify({
          token,
          confirm_name: confirmName,
          confirm_code: confirmCode,
        }),
      },
      csrf,
    );
  },

  async downloadZip(root: number, paths: string[], csrf: string): Promise<Blob> {
    const response = await fetch(`${BASE}/download/zip`, {
      method: 'POST',
      credentials: 'include',
      headers: {
        'Content-Type': 'application/json',
        'X-CSRF-Token': csrf,
      },
      body: JSON.stringify({ root, paths }),
    });

    if (!response.ok) {
      throw await apiErrorFromResponse(response);
    }

    return response.blob();
  },

  getSettings(): Promise<SettingsResponse> {
    return request<SettingsResponse>('/admin/settings');
  },

  updateSetting(key: string, value: unknown, csrf: string): Promise<SettingsResponse> {
    return request<SettingsResponse>(
      '/admin/settings',
      { method: 'PUT', body: JSON.stringify({ key, value }) },
      csrf,
    );
  },

  resetSetting(key: string, csrf: string): Promise<SettingsResponse> {
    return request<SettingsResponse>(
      `/admin/settings/${encodeURIComponent(key)}`,
      { method: 'DELETE' },
      csrf,
    );
  },
};
