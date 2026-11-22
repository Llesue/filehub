export type Role = 'admin' | 'user';

export interface UserPreferences {
  theme: 'light' | 'dark' | 'system';
  language: 'zh-CN' | 'en';
  show_hidden: boolean;
  show_thumbnails: boolean;
  upload_conflict: 'rename' | 'overwrite' | 'skip';
  terminal_enabled: boolean;
  editor_enabled: boolean;
  remote_mount_enabled: boolean;
  avatar_version: number;
}

export interface SystemFeatures {
  thumbnails: boolean;
  preview_edge?: number;
  editor: boolean;
  terminal: boolean;
}

export interface PreferencesResponse {
  preferences: UserPreferences;
  system: SystemFeatures;
  capabilities: Capabilities;
}

export interface UserFeatures {
  terminal: boolean;
  zip: boolean;
  unzip: boolean;
  federation_mount?: boolean;
  federation_share?: boolean;
}

export interface RemoteGrant {
  id: number;
  label: string;
  mode: 'ro' | 'rw';
}

export interface RemoteGrantInfo {
  id: number;
  label: string;
  path: string;
  mode: 'ro' | 'rw';
  revoked: boolean;
  available?: boolean;
  token_count: number;
  created_at: number;
}

export interface RemoteTokenInfo {
  id: number;
  name: string;
  prefix: string;
  created_at: number;
  expires_at: number | null;
  revoked: boolean;
  last_used_at: number | null;
  grant_ids: number[];
}

export interface RemoteMountAddress {
  url: string;
  state: RemoteMount['state'] | 'unknown';
  last_ok_at: number | null;
  last_error: string | null;
  last_error_code?: string | null;
  active: boolean;
}

export interface RemoteMount {
  id: number;
  root_id: number;
  alias: string;
  base_url: string;
  addresses?: RemoteMountAddress[];
  grant_id: number;
  grant_label: string;
  mode: 'ro' | 'rw';
  capabilities: string[];
  owner_name?: string;
  avatar_version?: number;
  thumb_cache_bytes?: number;
  state: 'online' | 'offline' | 'token_invalid' | 'grant_revoked' | 'unsupported';
  last_ok_at: number | null;
  last_error: string | null;
  last_error_code?: string | null;
  created_at: number;
}

export interface Capabilities {
  terminal: boolean;
  zip: boolean;
  unzip: boolean;
}

export interface User {
  id: number;
  username: string;
  role: Role;
  quota_bytes: number | null;
  disabled: boolean;
  features: UserFeatures;
  created_at: number;
  updated_at: number;
  last_login_at: number | null;
  online?: boolean;
}

export interface AuthResponse {
  user: User;
  remote?: { mount: boolean; share: boolean };
  csrf_token: string;
  auth: 'session' | 'token';
}

export interface SetupStatus {
  initialized: boolean;
}

export interface SettingsResponse {
  platform: string;
  settings: Record<string, unknown>;
  sources: Record<string, string>;
  restart_required: string[];
}

export interface Root {
  id: number;
  alias: string;
  read_only: boolean;
  kind?: 'local' | 'remote';
  mount_id?: number | null;
  shared?: boolean;
}

export interface VersionInfo {
  version: string;
  repository: string;
  releases: string;
  check_enabled: boolean;
  latest: string | null;
  update_available: boolean;
  checked_at: number | null;
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
  shared?: boolean;
  symlink_state?: 'ok' | 'broken' | 'denied';
  symlink_target?: string | null;
}

export interface ListResponse {
  root: Root;
  path: string;
  read_only: boolean;
  read_only_reason?: 'policy' | 'filesystem' | null;
  entries: Entry[];
  next_cursor: string | null;
  focus_index?: number;
  focus_offset?: number;
  total?: number;
  stale?: boolean;
}

export interface UploadCreated {
  upload_id: string;
  chunk_size: number;
  chunks: number;
  expires_at: number;
}

export interface UploadStatus {
  upload_id: string;
  total_size: number;
  chunk_size: number;
  received: number;
  received_chunks: number[];
  expires_at: number;
}

export interface ServerPath {
  id: number;
  alias: string;
  real_path: string;
  created_at: number;
}

export interface AudioTrackInfo {
  index: number;
  codec: string;
  channels: number;
  layout: string;
  language: string;
  default: boolean;
  supported: boolean;
}

export interface AudioInfo {
  ffmpeg: boolean;
  tracks: AudioTrackInfo[];
  default_track: number | null;
  has_supported_track: boolean;
}

export interface ShareInfo {
  id: number;
  kind?: 'single' | 'collection';
  item_count?: number;
  real_path: string;
  is_dir: boolean;
  has_password: boolean;
  expires_at: number | null;
  expired: boolean;
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
  line_ending?: 'lf' | 'crlf' | 'mixed';
  writable?: boolean;
  read_only_reason?: 'file' | 'directory' | null;
  encoding?: string;
  bom?: boolean;
  lossy?: boolean;
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
  zip_allowed?: boolean;
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
  dir_size?: number | null;
  dir_size_source?: string | null;
  dir_size_stale?: boolean;
  entry_count?: number | null;
  rootId: number;
  root_id?: number;
}

export interface DeletePreview {
  token: string;
  expires_at: number;
  items: Array<{ relative: string; is_dir: boolean; size?: number }>;
  confirm_code: string;
  requires_code?: boolean;
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

  setup(payload: {
    token: string;
    username: string;
    password: string;
    language?: 'zh-CN' | 'en';
  }): Promise<AuthResponse> {
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

  version(): Promise<VersionInfo> {
    return request<VersionInfo>('/version');
  },

  remoteProbe(url: string, csrf: string): Promise<{
    ok: boolean;
    product: string;
    version: number;
    server_version: string;
    instance_id: string;
    capabilities: string[];
    resolved_ips: string[];
    address_class: string;
    plaintext: boolean;
  }> {
    return request('/remote/probe', { method: 'POST', body: JSON.stringify({ url }) }, csrf);
  },

  remotePreview(
    url: string,
    token: string,
    csrf: string,
  ): Promise<{ grants: RemoteGrant[] }> {
    return request(
      '/remote/grants/preview',
      { method: 'POST', body: JSON.stringify({ url, token }) },
      csrf,
    );
  },

  remoteActivity(): Promise<{
    activity: Array<{ op: string; path: string | null; status: number; bytes: number; created_at: number }>;
  }> {
    return request('/remote/activity');
  },

  remoteGrants(): Promise<{ grants: RemoteGrantInfo[] }> {
    return request('/remote/grants');
  },

  remoteCreateGrant(
    root: number,
    path: string,
    label: string,
    mode: 'ro' | 'rw',
    csrf: string,
  ): Promise<{ id: number }> {
    return request(
      '/remote/grants',
      { method: 'POST', body: JSON.stringify({ root, path, label, mode }) },
      csrf,
    );
  },

  remoteRevokeGrant(id: number, csrf: string): Promise<void> {
    return request(`/remote/grants/${id}`, { method: 'DELETE' }, csrf);
  },

  remoteTokens(): Promise<{ tokens: RemoteTokenInfo[] }> {
    return request('/remote/tokens');
  },

  remoteCreateToken(
    name: string,
    grantIds: number[],
    expiresAt: number | null,
    csrf: string,
  ): Promise<{ id: number; token: string; prefix: string }> {
    return request(
      '/remote/tokens',
      { method: 'POST', body: JSON.stringify({ name, grant_ids: grantIds, expires_at: expiresAt }) },
      csrf,
    );
  },

  remoteRevokeToken(id: number, csrf: string): Promise<void> {
    return request(`/remote/tokens/${id}`, { method: 'DELETE' }, csrf);
  },

  remoteUpdateTokenGrants(
    id: number,
    grantIds: number[],
    csrf: string,
  ): Promise<{ id: number; grant_ids: number[] }> {
    return request(`/remote/tokens/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ grant_ids: grantIds }),
    }, csrf);
  },

  remoteRotateToken(
    id: number,
    csrf: string,
  ): Promise<{ id: number; token: string; prefix: string }> {
    return request(`/remote/tokens/${id}/rotate`, { method: 'POST' }, csrf);
  },

  remoteThumbWarm(
    root: number,
    path: string,
    paths: string[],
    csrf: string,
  ): Promise<{ queued: number; cached: number; skipped: number; disabled: boolean }> {
    return request(
      '/remote/thumb-warm',
      { method: 'POST', body: JSON.stringify({ root, path, paths }) },
      csrf,
    );
  },

  remoteRenameMount(id: number, alias: string, csrf: string): Promise<{ id: number; alias: string }> {
    return request(`/remote/mounts/${id}`, {
      method: 'PATCH',
      body: JSON.stringify({ alias }),
    }, csrf);
  },

  remoteMountGrants(id: number): Promise<{
    mount_id: number;
    base_url: string;
    grants: Array<{
      id: number;
      label: string;
      mode: 'ro' | 'rw';
      mounted: boolean;
      mount_id: number | null;
      current: boolean;
    }>;
  }> {
    return request(`/remote/mounts/${id}/grants`);
  },

  remoteAttachGrant(
    id: number,
    grantId: number,
    csrf: string,
  ): Promise<{ id: number; root_id: number; alias: string; mode: 'ro' | 'rw'; grant_id: number }> {
    return request(
      `/remote/mounts/${id}/grants/${grantId}`,
      { method: 'POST' },
      csrf,
    );
  },

  remoteAvatarUrl(id: number, version: number): string {
    return `${BASE}/remote/mounts/${id}/avatar?v=${version}`;
  },

  remoteMounts(): Promise<{ mounts: RemoteMount[] }> {
    return request('/remote/mounts');
  },

  remoteCreateMount(
    urls: string[],
    token: string,
    grantId: number,
    alias: string,
    csrf: string,
  ): Promise<{ id: number; root_id: number }> {
    return request(
      '/remote/mounts',
      {
        method: 'POST',
        body: JSON.stringify({
          url: urls[0] ?? '',
          urls,
          token,
          grant_id: grantId,
          alias,
        }),
      },
      csrf,
    );
  },

  remoteUpdateMountAddresses(
    id: number,
    urls: string[],
    csrf: string,
  ): Promise<{ ok: boolean; active_url: string }> {
    return request(
      `/remote/mounts/${id}/addresses`,
      {
        method: 'PUT',
        body: JSON.stringify({ urls }),
      },
      csrf,
    );
  },

  remoteDeleteMount(id: number, csrf: string): Promise<void> {
    return request(`/remote/mounts/${id}`, { method: 'DELETE' }, csrf);
  },

  remoteReauthMount(id: number, token: string, csrf: string): Promise<{ ok: boolean }> {
    return request(
      `/remote/mounts/${id}/reauth`,
      { method: 'POST', body: JSON.stringify({ token }) },
      csrf,
    );
  },

  remoteTestMount(
    id: number,
    csrf: string,
  ): Promise<{ ok: boolean; removed?: boolean; latency_ms: number; server_version?: string; message?: string }> {
    return request(`/remote/mounts/${id}/test`, { method: 'POST' }, csrf);
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
    focus?: string,
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
    if (focus) query.set('focus', focus);
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

  thumbUrl(root: number, path: string, width = 256, version?: number, wait = false): string {
    const query = new URLSearchParams({
      root: String(root),
      path,
      w: String(width),
    });
    if (version !== undefined) query.set('v', String(version));
    if (wait) query.set('wait', '1');
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

  editPresence(
    root: number,
    path: string,
    state: 'open' | 'close',
    client: string,
    csrf: string,
    keepalive = false,
  ): Promise<{
    ok: boolean;
    others: number;
    peers: Array<{ client: string; expires_at: number }>;
    expires_at: number;
  }> {
    return request(
      '/edit/presence',
      {
        method: 'POST',
        body: JSON.stringify({ root, path, state, client }),
        keepalive,
      },
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

  publicThumbUrl(
    token: string,
    path: string,
    width = 320,
    ticket?: string,
    item?: number,
    version?: number,
    wait = false,
  ): string {
    const query = new URLSearchParams({ path, w: String(width) });
    if (ticket) query.set('ticket', ticket);
    if (item !== undefined) query.set('item', String(item));
    if (version !== undefined) query.set('v', String(version));
    if (wait) query.set('wait', '1');
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
      mtime?: number;
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

  dirVersion(root: number, path: string): Promise<{ mtime: number }> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<{ mtime: number }>(`/fs/dir-version?${query.toString()}`);
  },

  getFile(root: number, path: string, encoding?: string): Promise<FileContent> {
    const query = new URLSearchParams({ root: String(root), path });
    if (encoding) query.set('encoding', encoding);
    return request<FileContent>(`/file?${query.toString()}`);
  },

  saveFile(
    root: number,
    path: string,
    content: string,
    etag: string,
    csrf: string,
    encoding?: string,
    bom?: boolean,
    force = false,
  ): Promise<{ etag: string; size: number }> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<{ etag: string; size: number }>(
      `/file?${query.toString()}`,
      {
        method: 'PUT',
        headers: force ? {} : { 'If-Match': etag },
        body: JSON.stringify({ content, encoding, bom, force }),
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
      feature_terminal?: boolean;
      feature_zip?: boolean;
      feature_unzip?: boolean;
    },
    csrf: string,
  ): Promise<{ user: User }> {
    return request<{ user: User }>(
      `/admin/users/${id}`,
      { method: 'PUT', body: JSON.stringify(payload) },
      csrf,
    );
  },

  audioInfo(root: number, path: string): Promise<AudioInfo> {
    const query = new URLSearchParams({ root: String(root), path });
    return request<AudioInfo>(`/media/audio-info?${query.toString()}`);
  },

  transcodeAudio(
    root: number,
    path: string,
    track: number,
    csrf: string,
  ): Promise<{ job_id: string | null; ready: boolean; path: string; track: number }> {
    return request(
      '/media/transcode',
      { method: 'POST', body: JSON.stringify({ root, path, track }) },
      csrf,
    );
  },

  transcodedUrl(root: number, path: string, track: number): string {
    const query = new URLSearchParams({
      root: String(root),
      path,
      track: String(track),
    });
    return `/api/v1/media/transcoded?${query.toString()}`;
  },

  archiveUnzip(
    root: number,
    path: string,
    target: string | undefined,
    csrf: string,
  ): Promise<{ job_id: string | null; status: string; name: string }> {
    return request(
      '/archive/unzip',
      {
        method: 'POST',
        body: JSON.stringify({ root, path, target }),
      },
      csrf,
    );
  },

  archiveZip(
    root: number,
    dir: string,
    paths: string[],
    name: string,
    csrf: string,
  ): Promise<{ job_id: string; status: string; created: boolean; name: string }> {
    return request(
      '/archive/zip',
      { method: 'POST', body: JSON.stringify({ root, dir, paths, name }) },
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
  ): Promise<{ updated: boolean }> {
    return request<{ updated: boolean }>(
      `/admin/users/${userId}/roots/${rootId}`,
      { method: 'PUT', body: JSON.stringify(payload) },
      csrf,
    );
  },

  userUsage(
    userId: number,
  ): Promise<{ used_bytes: number | null; quota_bytes: number | null; computing: boolean }> {
    return request<{
      used_bytes: number | null;
      quota_bytes: number | null;
      computing: boolean;
    }>(`/admin/users/${userId}/usage`);
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

  jobs(since = 0, signal?: AbortSignal): Promise<JobInfo[]> {
    return request<{ jobs: JobInfo[] }>(`/jobs?since=${Math.max(0, Math.floor(since))}`, {
      signal,
    }).then((value) => value.jobs);
  },

  cancelThumbScope(
    root: number,
    path: string,
    scope: 'dir' | 'file',
    csrf: string,
  ): Promise<{ cancelled: number }> {
    return request<{ cancelled: number }>(
      '/thumb/cancel',
      { method: 'POST', body: JSON.stringify({ root, path, scope }) },
      csrf,
    );
  },

  imageInfo(
    root: number,
    path: string,
    signal?: AbortSignal,
  ): Promise<{
    width: number | null;
    height: number | null;
    pixels: number | null;
    limit: number | null;
    previewable: boolean;
    reason: 'unsupported' | 'too_large' | 'max_source' | null;
  }> {
    const query = new URLSearchParams({ root: String(root), path });
    return request(`/fs/image-info?${query.toString()}`, { signal });
  },

  previewThumbUrl(root: number, path: string, edge = 2048): string {
    const query = new URLSearchParams({
      root: String(root),
      path,
      fit: 'long',
      edge: String(edge),
      // The lightbox has no per-image SSE reload hook, so keep its thumbnail
      // request synchronous instead of showing the async placeholder.
      wait: '1',
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

  shareUrl(id: number): Promise<{ token: string; url: string }> {
    return request<{ token: string; url: string }>(`/shares/${id}/url`);
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
      theme: 'light' | 'dark' | 'system';
      language: 'zh-CN' | 'en';
      show_hidden: boolean;
      show_thumbnails: boolean;
      upload_conflict: 'rename' | 'overwrite' | 'skip';
      terminal_enabled: boolean;
      editor_enabled: boolean;
      remote_mount_enabled: boolean;
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
    return `${BASE}/custom.css?v=${version}`;
  },

  saveCustomCss(content: string, csrf: string): Promise<{ version: number }> {
    return request('/admin/custom-css', { method: 'PUT', body: JSON.stringify({ content }) }, csrf);
  },

  clearCustomCss(csrf: string): Promise<{ version: number }> {
    return request('/admin/custom-css', { method: 'DELETE' }, csrf);
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

  searchAll(
    query: string,
    limit = 100,
    cursor: string | null = null,
    signal?: AbortSignal,
    filters: {
      kind?: string;
      minSize?: number;
      maxSize?: number;
      modifiedAfter?: number;
      modifiedBefore?: number;
      isDir?: boolean;
    } = {},
  ): Promise<{ entries: SearchEntry[]; next_cursor: string | null }> {
    const parameters = new URLSearchParams({ q: query, limit: String(limit) });
    if (cursor) parameters.set('cursor', cursor);
    if (filters.kind) parameters.set('kind', filters.kind);
    if (filters.minSize !== undefined) parameters.set('min_size', String(filters.minSize));
    if (filters.maxSize !== undefined) parameters.set('max_size', String(filters.maxSize));
    if (filters.modifiedAfter !== undefined) {
      parameters.set('modified_after', String(filters.modifiedAfter));
    }
    if (filters.modifiedBefore !== undefined) {
      parameters.set('modified_before', String(filters.modifiedBefore));
    }
    return request<{ entries: SearchEntry[]; next_cursor: string | null }>(
      `/search/all?${parameters.toString()}`,
      { signal },
    );
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
    mtime?: number,
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
          ...(mtime && mtime > 0 ? { mtime } : {}),
        }),
      },
      csrf,
    );
  },

  uploadStatus(uploadId: string, signal?: AbortSignal): Promise<UploadStatus> {
    return request<UploadStatus>(`/uploads/${uploadId}`, { signal });
  },

  uploadChunk(
    uploadId: string,
    index: number,
    chunk: Blob,
    csrf: string,
    signal?: AbortSignal,
    onProgress?: (loaded: number) => void,
  ): Promise<void> {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('PUT', `${BASE}/uploads/${uploadId}/chunks/${index}`);
      xhr.withCredentials = true;
      xhr.setRequestHeader('Content-Type', 'application/octet-stream');
      xhr.setRequestHeader('X-CSRF-Token', csrf);
      xhr.upload.onprogress = (event) => {
        if (event.lengthComputable) onProgress?.(event.loaded);
      };
      xhr.onload = () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          resolve();
          return;
        }
        let code = 'INTERNAL_ERROR';
        let message = xhr.statusText || 'Request failed';
        try {
          const parsed = JSON.parse(xhr.responseText) as {
            error?: { code?: string; message?: string };
          };
          code = parsed.error?.code ?? code;
          message = parsed.error?.message ?? message;
        } catch (error) {
          console.warn('failed to parse upload error response', error);
        }
        reject(new ApiError(xhr.status, code, message));
      };
      xhr.onerror = () => reject(new ApiError(0, 'NETWORK_ERROR', '网络连接中断'));
      xhr.onabort = () => reject(new DOMException('aborted', 'AbortError'));
      const abort = (): void => xhr.abort();
      signal?.addEventListener('abort', abort, { once: true });
      xhr.onloadend = () => signal?.removeEventListener('abort', abort);
      xhr.send(chunk);
    });
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

  updateSettings(values: Record<string, unknown>, csrf: string): Promise<SettingsResponse> {
    return request<SettingsResponse>(
      '/admin/settings',
      { method: 'PATCH', body: JSON.stringify({ values }) },
      csrf,
    );
  },

  listAllRoots(): Promise<{ roots: Record<string, AdminRoot[]> }> {
    return request('/admin/roots');
  },

  resetSettings(keys: string[], csrf: string): Promise<{ reset: number }> {
    return request(
      '/admin/settings/reset',
      { method: 'POST', body: JSON.stringify({ keys }) },
      csrf,
    );
  },

  verifyAdminPassword(password: string, csrf: string): Promise<void> {
    return request<void>(
      '/admin/verify-password',
      { method: 'POST', body: JSON.stringify({ password }) },
      csrf,
    );
  },

  restartServer(csrf: string): Promise<{ restarting: boolean }> {
    return request('/admin/restart', { method: 'POST' }, csrf);
  },

  resetSetting(key: string, csrf: string): Promise<SettingsResponse> {
    return request<SettingsResponse>(
      `/admin/settings/${encodeURIComponent(key)}`,
      { method: 'DELETE' },
      csrf,
    );
  },
};
