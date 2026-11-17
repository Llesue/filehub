<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import FileTypeIcon from './FileTypeIcon.svelte';
  import Icon from './Icon.svelte';
  import PreviewModal from './PreviewModal.svelte';
  import { api, ApiError, type Entry, type ShareEntry, type ShareMeta } from '../api';
  import { language, t } from '../i18n';
  import { branding, brandMark } from '../branding';
  import { playAudio, type AudioTrack } from '../audio';

  let { token }: { token: string } = $props();

  type UploadStatus = 'queued' | 'uploading' | 'done' | 'error' | 'cancelled';

  interface UploadTask {
    id: number;
    file: File;
    name: string;
    status: UploadStatus;
    progress: number;
    error: string;
    uploadId: string | null;
    chunkSize: number;
  }

  let data = $state<ShareMeta | null>(null);
  let path = $state('');
  let ticket = $state<string | undefined>(undefined);
  let password = $state('');
  let needsPassword = $state(false);
  let loading = $state(true);
  let loadingMore = $state(false);
  let error = $state('');
  let view = $state<'grid' | 'list'>('grid');
  let selected = $state<string[]>([]);
  let preview = $state<{ items: Entry[]; index: number } | null>(null);
  let zipInfo = $state<{
    checked: boolean;
    allowed: boolean;
    files: number;
    bytes: number;
    message: string;
  }>({ checked: false, allowed: false, files: 0, bytes: 0, message: '' });
  let uploads = $state<UploadTask[]>([]);
  let uploadBusy = false;
  let uploadSeq = 0;
  const clickTimers: Record<string, ReturnType<typeof setTimeout>> = {};

  let entries = $derived(data?.entries ?? []);
  let crumbs = $derived(path.split('/').filter(Boolean));
  let zipAllowed = $derived(Boolean(data?.is_dir) && zipInfo.allowed);
  let selectedCount = $derived(selected.length);

  onMount(() => {
    const stored = localStorage.getItem('filehub.share.view');
    if (stored === 'list' || stored === 'grid') view = stored;
    void load();
  });

  onDestroy(() => {
    for (const timer of Object.values(clickTimers)) clearTimeout(timer);
  });

  async function load(cursor?: string): Promise<void> {
    if (cursor) loadingMore = true;
    else loading = true;
    error = '';
    try {
      const result = await api.publicShare(token, path, ticket, cursor);
      if (cursor && data) {
        data = {
          ...result,
          entries: [...(data.entries ?? []), ...(result.entries ?? [])],
        };
      } else {
        data = result;
        selected = [];
        void refreshZipInfo();
      }
      needsPassword = false;
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        needsPassword = true;
      } else if (err instanceof ApiError && err.code === 'SHARE_REVOKED') {
        data = null;
        error = t($language, '共享已停止');
      } else if (err instanceof ApiError && err.code === 'SHARE_EXPIRED') {
        data = null;
        error = t($language, '共享已过期');
      } else if (err instanceof ApiError && err.status === 404) {
        data = null;
        error = t($language, '分享不存在或链接无效');
      } else {
        error = err instanceof Error ? err.message : t($language, '无法打开分享');
      }
    } finally {
      loading = false;
      loadingMore = false;
    }
  }

  async function unlock(): Promise<void> {
    error = '';
    try {
      const result = await api.unlockShare(token, password);
      ticket = result.ticket;
      needsPassword = false;
      await load();
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '密码错误');
    }
  }

  function navigate(next: string): void {
    path = next;
    selected = [];
    preview = null;
    zipInfo = { checked: false, allowed: false, files: 0, bytes: 0, message: '' };
    void load();
  }

  function join(base: string, name: string): string {
    return base ? `${base}/${name}` : name;
  }

  function entryKey(entry: ShareEntry): string {
    return entry.item_id !== undefined ? `item:${entry.item_id}` : entry.name;
  }

  function downloadUrlFor(entry: ShareEntry, inline = false): string {
    return entry.item_id !== undefined
      ? api.publicDownloadUrl(token, '', ticket, inline, entry.item_id)
      : api.publicDownloadUrl(token, join(path, entry.name), ticket, inline);
  }

  function thumbUrlFor(entry: ShareEntry, width: number): string {
    return entry.item_id !== undefined
      ? api.publicThumbUrl(token, '', width, ticket, entry.item_id)
      : api.publicThumbUrl(token, join(path, entry.name), width, ticket);
  }

  async function refreshZipInfo(): Promise<void> {
    if (!data?.is_dir) return;
    try {
      const result = await api.publicZipPreflight(
        token,
        data.collection ? { ticket } : { path, ticket },
      );
      zipInfo = {
        checked: true,
        allowed: result.allowed,
        files: result.files ?? 0,
        bytes: result.bytes ?? 0,
        message: result.message ?? '',
      };
    } catch (err) {
      console.warn('share zip preflight failed', err);
      zipInfo = { checked: true, allowed: false, files: 0, bytes: 0, message: '' };
    }
  }

  function setView(next: 'grid' | 'list'): void {
    view = next;
    localStorage.setItem('filehub.share.view', next);
  }

  function toggleSelection(key: string): void {
    selected = selected.includes(key)
      ? selected.filter((item) => item !== key)
      : [...selected, key];
  }

  function clearSelection(): void {
    selected = [];
  }

  function handleEntryClick(entry: ShareEntry): void {
    if (entry.is_dir) {
      navigate(join(path, entry.name));
      return;
    }
    const key = entryKey(entry);
    const existing = clickTimers[key];
    if (existing) clearTimeout(existing);
    clickTimers[key] = setTimeout(() => {
      delete clickTimers[key];
      toggleSelection(key);
    }, 220);
  }

  function handleEntryDoubleClick(entry: ShareEntry): void {
    if (entry.is_dir) return;
    const key = entryKey(entry);
    const timer = clickTimers[key];
    if (timer) {
      clearTimeout(timer);
      delete clickTimers[key];
    }
    openPreview(entry);
  }

  function isImage(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?)$/i.test(name);
  }

  function isVideo(name: string): boolean {
    return /\.(mp4|mov|mkv|webm|avi|m4v)$/i.test(name);
  }

  function isAudio(name: string): boolean {
    return /\.(mp3|wav|ogg|m4a|flac|aac)$/i.test(name);
  }

  function previewable(entry: ShareEntry): boolean {
    return !entry.is_dir && (entry.has_preview || isImage(entry.name) || isVideo(entry.name));
  }

  function asEntry(entry: ShareEntry): Entry {
    return {
      item_id: entry.item_id,
      name: entry.name,
      is_dir: entry.is_dir,
      is_symlink: Boolean(entry.is_symlink),
      size: entry.size,
      modified: entry.modified,
      created_at: entry.created_at ?? 0,
      mime_type: entry.mime_type ?? '',
      has_preview: Boolean(entry.has_preview),
    };
  }

  function openPreview(entry: ShareEntry): void {
    const items = entries.filter(previewable);
    const key = entryKey(entry);
    const index = items.findIndex((item) => entryKey(item) === key);
    if (index < 0) return;
    preview = { items: items.map(asEntry), index };
  }

  function download(entry?: ShareEntry): void {
    window.location.href = entry
      ? downloadUrlFor(entry)
      : api.publicDownloadUrl(token, path, ticket);
  }

  function downloadSelected(): void {
    const targets = selected
      .map((key) => entries.find((entry) => entryKey(entry) === key))
      .filter((entry): entry is ShareEntry => Boolean(entry && !entry.is_dir));
    if (targets.length === 0) return;
    if (targets.length === 1) {
      download(targets[0]);
      return;
    }
    submitZip(targets);
  }

  function submitZip(targets: ShareEntry[]): void {
    const form = document.createElement('form');
    form.method = 'POST';
    form.action = api.publicZipFormUrl(token);
    form.target = '_blank';
    const add = (name: string, value: string): void => {
      const input = document.createElement('input');
      input.type = 'hidden';
      input.name = name;
      input.value = value;
      form.appendChild(input);
    };
    add('path', path);
    if (ticket) add('ticket', ticket);
    for (const target of targets) {
      if (target.item_id !== undefined) add('item_ids', String(target.item_id));
      else add('paths', join(path, target.name));
    }
    document.body.appendChild(form);
    form.submit();
    form.remove();
  }

  function playEntry(entry: ShareEntry): void {
    const tracks: AudioTrack[] = entries
      .filter((item) => !item.is_dir && isAudio(item.name))
      .map((item) => ({
        name: item.name,
        url: api.publicDownloadUrl(token, join(path, item.name), ticket, true),
        key: `${token}:${join(path, item.name)}`,
      }));
    const index = tracks.findIndex((track) => track.name === entry.name);
    if (index >= 0) playAudio(tracks, index);
  }

  function formatSize(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
    return `${(size / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  function selectedLabel(): string {
    const total = entries
      .filter((entry) => selected.includes(entryKey(entry)))
      .reduce((sum, entry) => sum + entry.size, 0);
    return `${t($language, '已选 {count} 项', { count: selectedCount })} · ${formatSize(total)}`;
  }

  function addFiles(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    for (const file of files) {
      uploads = [
        ...uploads,
        {
          id: (uploadSeq += 1),
          file,
          name: file.name,
          status: 'queued',
          progress: 0,
          error: '',
          uploadId: null,
          chunkSize: 8 * 1024 * 1024,
        },
      ];
    }
    void processQueue();
  }

  async function processQueue(): Promise<void> {
    if (uploadBusy) return;
    uploadBusy = true;
    try {
      while (true) {
        const task = uploads.find((item) => item.status === 'queued');
        if (!task) break;
        await runUpload(task);
      }
    } finally {
      uploadBusy = false;
    }
  }

  async function runUpload(task: UploadTask): Promise<void> {
    task.status = 'uploading';
    task.progress = 0;
    task.error = '';
    try {
      const created = await api.createShareUpload(token, {
        path,
        filename: task.name,
        total_size: task.file.size,
        ticket,
      });
      task.uploadId = created.upload_id;
      task.chunkSize = created.chunk_size;
      let index = 0;
      while (index < created.chunks) {
        if (isCancelled(task)) {
          await api.cancelShareUpload(token, created.upload_id, ticket);
          return;
        }
        const start = index * created.chunk_size;
        const end = Math.min(start + created.chunk_size, task.file.size);
        await api.uploadShareChunk(
          token,
          created.upload_id,
          index,
          task.file.slice(start, end),
          ticket,
        );
        index += 1;
        task.progress = end / task.file.size;
      }
      await api.completeShareUpload(token, created.upload_id, ticket);
      task.status = 'done';
      task.progress = 1;
      await load();
    } catch (err) {
      if (isCancelled(task)) return;
      task.status = 'error';
      task.error = err instanceof Error ? err.message : t($language, '上传失败');
    }
  }

  function isCancelled(task: UploadTask): boolean {
    return task.status === 'cancelled';
  }

  function cancelTask(task: UploadTask): void {
    task.status = 'cancelled';
    if (task.uploadId) {
      void api.cancelShareUpload(token, task.uploadId, ticket).catch((err) => {
        console.warn('cancel share upload failed', err);
      });
    }
  }

  function retryTask(task: UploadTask): void {
    if (task.uploadId) {
      void api.cancelShareUpload(token, task.uploadId, ticket).catch(() => undefined);
    }
    task.status = 'queued';
    task.progress = 0;
    task.error = '';
    task.uploadId = null;
    void processQueue();
  }

  function clearFinishedUploads(): void {
    uploads = uploads.filter((task) => task.status === 'uploading' || task.status === 'queued');
  }
</script>

<svelte:head>
  <title>{$branding.name} · {t($language, '分享')}</title>
</svelte:head>

<main class="share-shell">
  <section class="share-card">
    <header class="share-header">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">
          {#if $branding.iconUrl}<img src={$branding.iconUrl} alt="" />{:else}{brandMark(
              $branding.name,
            )}{/if}
        </span>
        <span>{$branding.name} · {t($language, '分享')}</span>
      </div>
      {#if data}
        <h1>{data.name}</h1>
      {/if}
    </header>

    {#if needsPassword}
      <form
        class="unlock"
        onsubmit={(event) => {
          event.preventDefault();
          void unlock();
        }}
      >
        <p>{t($language, '该分享需要密码')}</p>
        <input
          bind:value={password}
          type="password"
          placeholder={t($language, '输入分享密码')}
          autocomplete="current-password"
        />
        <button type="submit">{t($language, '解锁')}</button>
      </form>
    {:else if loading}
      <p class="muted">{t($language, '加载中…')}</p>
    {:else if error}
      <p class="error" role="alert">{error}</p>
    {:else if data}
      {#if data.is_dir}
        <div class="share-toolbar">
          <nav class="path" aria-label={t($language, '分享路径')}>
            <button type="button" onclick={() => navigate('')}>{t($language, '根目录')}</button>
            {#each crumbs as segment, index (index)}
              <span>/</span>
              <button type="button" onclick={() => navigate(crumbs.slice(0, index + 1).join('/'))}
                >{segment}</button
              >
            {/each}
          </nav>
          <div class="toolbar-actions">
            <div class="segmented" role="group" aria-label={t($language, '视图')}>
              <button
                type="button"
                class:active={view === 'grid'}
                aria-pressed={view === 'grid'}
                aria-label={t($language, '网格视图')}
                title={t($language, '网格视图')}
                onclick={() => setView('grid')}><Icon name="grid" size={17} /></button
              >
              <button
                type="button"
                class:active={view === 'list'}
                aria-pressed={view === 'list'}
                aria-label={t($language, '列表视图')}
                title={t($language, '列表视图')}
                onclick={() => setView('list')}><Icon name="list" size={17} /></button
              >
            </div>
            {#if !data.read_only}
              <label class="upload-button">
                {t($language, '上传文件')}
                <input type="file" multiple onchange={addFiles} />
              </label>
            {/if}
          </div>
        </div>

        {#if selectedCount > 0}
          <div class="selection-bar">
            <span>{selectedLabel()}</span>
            <div class="selection-actions">
              <button type="button" onclick={downloadSelected}>{t($language, '下载选中')}</button>
              {#if zipAllowed}
                <button type="button" onclick={() => submitZip([])}
                  >{t($language, '打包下载全部')}</button
                >
              {/if}
              <button type="button" class="ghost" onclick={clearSelection}
                >{t($language, '清除选择')}</button
              >
            </div>
          </div>
        {:else if zipInfo.checked && data.is_dir && entries.length > 0}
          <div class="zip-hint">
            {#if zipAllowed}
              <button type="button" class="ghost" onclick={() => submitZip([])}
                >{t($language, '打包下载全部')}</button
              >
              <span>{t($language, '约 {size}', { size: formatSize(zipInfo.bytes) })}</span>
            {:else if zipInfo.message}
              <span class="muted">{t($language, '目录过大，未提供打包下载')}</span>
            {/if}
          </div>
        {/if}

        {#if view === 'grid'}
          <div class="share-grid">
            {#each entries as entry (entry.name)}
              <div
                class="share-card-item"
                class:selected={selected.includes(entryKey(entry))}
                role="button"
                tabindex="0"
                onclick={() => handleEntryClick(entry)}
                ondblclick={() => handleEntryDoubleClick(entry)}
                onkeydown={(event) => {
                  if (event.key === 'Enter' && entry.is_dir) navigate(join(path, entry.name));
                }}
              >
                <input
                  class="card-check"
                  type="checkbox"
                  aria-label={t($language, '选择 {name}', { name: entry.name })}
                  checked={selected.includes(entryKey(entry))}
                  disabled={entry.is_dir}
                  onclick={(event) => event.stopPropagation()}
                  onchange={() => toggleSelection(entryKey(entry))}
                />
                <div class="card-icon">
                  <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={44} />
                  {#if previewable(entry)}
                    <img
                      class="thumb"
                      src={thumbUrlFor(entry, 320)}
                      alt=""
                      loading="lazy"
                      decoding="async"
                      onerror={(event) => {
                        (event.currentTarget as HTMLImageElement).style.display = 'none';
                      }}
                    />
                    <button
                      type="button"
                      class="preview-button"
                      aria-label={t($language, '预览')}
                      title={t($language, '预览')}
                      onclick={(event) => {
                        event.stopPropagation();
                        openPreview(entry);
                      }}><Icon name="eye" size={16} /></button
                    >
                  {:else if !entry.is_dir && isAudio(entry.name)}
                    <button
                      type="button"
                      class="preview-button"
                      aria-label={t($language, '播放')}
                      title={t($language, '播放')}
                      onclick={(event) => {
                        event.stopPropagation();
                        playEntry(entry);
                      }}>▶</button
                    >
                  {/if}
                </div>
                <span class="entry-name" title={entry.name}>{entry.name}</span>
                <span class="entry-meta">{formatSize(entry.size)}</span>
              </div>
            {/each}
            {#if entries.length === 0}
              <p class="muted">{t($language, '这个目录是空的')}</p>
            {/if}
          </div>
        {:else}
          <div class="entries">
            {#each entries as entry (entry.name)}
              <div
                class="entry"
                class:selected={selected.includes(entryKey(entry))}
                role="button"
                tabindex="0"
                onclick={() => handleEntryClick(entry)}
                ondblclick={() => handleEntryDoubleClick(entry)}
                onkeydown={(event) => {
                  if (event.key === 'Enter' && entry.is_dir) navigate(join(path, entry.name));
                }}
              >
                <input
                  type="checkbox"
                  aria-label={t($language, '选择 {name}', { name: entry.name })}
                  checked={selected.includes(entryKey(entry))}
                  disabled={entry.is_dir}
                  onclick={(event) => event.stopPropagation()}
                  onchange={() => toggleSelection(entryKey(entry))}
                />
                <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={22} />
                <span class="entry-name" title={entry.name}>{entry.name}</span>
                <span class="entry-size">{entry.is_dir ? '—' : formatSize(entry.size)}</span>
                {#if previewable(entry)}
                  <button
                    type="button"
                    class="text-button"
                    onclick={(event) => {
                      event.stopPropagation();
                      openPreview(entry);
                    }}>{t($language, '预览')}</button
                  >
                {:else if !entry.is_dir && isAudio(entry.name)}
                  <button
                    type="button"
                    class="text-button"
                    onclick={(event) => {
                      event.stopPropagation();
                      playEntry(entry);
                    }}>{t($language, '播放')}</button
                  >
                {/if}
              </div>
            {/each}
            {#if entries.length === 0}
              <p class="muted">{t($language, '这个目录是空的')}</p>
            {/if}
          </div>
        {/if}

        {#if data.next_cursor}
          <div class="load-more">
            <button
              type="button"
              disabled={loadingMore}
              onclick={() => void load(data?.next_cursor ?? undefined)}
              >{loadingMore ? t($language, '加载中…') : t($language, '加载更多')}</button
            >
          </div>
        {/if}
      {:else}
        <div class="file">
          {#if data.has_preview && isImage(data.name)}
            <img
              class="share-preview"
              src={api.publicThumbUrl(token, '', 2048, ticket)}
              alt={data.name}
            />
          {:else if data.has_preview && isVideo(data.name)}
            <video
              class="share-preview"
              src={api.publicDownloadUrl(token, '', ticket, true)}
              controls><track kind="captions" /></video
            >
          {:else}
            <FileTypeIcon name={data.name} size={42} />
          {/if}
          <p>{data.size !== undefined ? formatSize(data.size) : ''}</p>
          <button
            type="button"
            class="download"
            onclick={() => window.open(api.publicDownloadUrl(token, '', ticket), '_blank')}
            >{t($language, '下载文件')}</button
          >
        </div>
      {/if}

      {#if uploads.length > 0}
        <div class="upload-queue">
          <div class="upload-queue-head">
            <span>{t($language, '上传')}</span>
            <button type="button" class="text-button" onclick={clearFinishedUploads}
              >{t($language, '清除已完成')}</button
            >
          </div>
          {#each uploads as task (task.id)}
            <div class="upload-row">
              <span class="upload-name" title={task.name}>{task.name}</span>
              <div class="upload-track">
                <div class="upload-fill" style={`width:${Math.round(task.progress * 100)}%`}></div>
              </div>
              <span class="upload-state">
                {#if task.status === 'done'}
                  {t($language, '完成')}
                {:else if task.status === 'error'}
                  <span class="error">{task.error}</span>
                {:else if task.status === 'cancelled'}
                  {t($language, '已取消')}
                {:else}
                  {Math.round(task.progress * 100)}%
                {/if}
              </span>
              {#if task.status === 'uploading' || task.status === 'queued'}
                <button type="button" class="text-button" onclick={() => cancelTask(task)}
                  >{t($language, '取消')}</button
                >
              {:else if task.status === 'error' || task.status === 'cancelled'}
                <button type="button" class="text-button" onclick={() => retryTask(task)}
                  >{t($language, '重试')}</button
                >
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  </section>
</main>

{#if preview}
  <PreviewModal
    root={0}
    currentPath=""
    items={preview.items}
    index={preview.index}
    onClose={() => (preview = null)}
    onIndexChange={(index) => (preview = preview ? { ...preview, index } : null)}
    srcFor={(entry) =>
      entry.item_id !== undefined
        ? api.publicDownloadUrl(token, '', ticket, true, entry.item_id)
        : api.publicDownloadUrl(token, join(path, entry.name), ticket, true)}
    thumbSrcFor={(entry) =>
      entry.item_id !== undefined
        ? api.publicThumbUrl(token, '', 2048, ticket, entry.item_id)
        : api.publicThumbUrl(token, join(path, entry.name), 2048, ticket)}
    downloadHref={(entry) =>
      entry.item_id !== undefined
        ? api.publicDownloadUrl(token, '', ticket, false, entry.item_id)
        : api.publicDownloadUrl(token, join(path, entry.name), ticket)}
  />
{/if}

<style>
  .share-shell {
    display: grid;
    min-height: 100dvh;
    place-items: center;
    padding: 24px;
    background: var(--bg);
  }

  .share-card {
    width: min(100%, 1100px);
    padding: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  .share-header {
    margin-bottom: 20px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 18px;
    font-size: 16px;
    font-weight: 600;
  }

  .brand-mark img {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    object-fit: cover;
  }

  .brand-mark {
    display: grid;
    width: 24px;
    height: 24px;
    place-items: center;
    border-radius: 7px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 14.5px;
  }

  h1 {
    margin: 0;
    color: var(--text);
    font-size: 24px;
    font-weight: 500;
  }

  .unlock {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: 320px;
    color: var(--text-2);
    font-size: 14.5px;
  }

  .unlock input {
    height: 40px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
  }

  .unlock button,
  .download {
    height: 36px;
    padding: 0 16px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
  }

  .share-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 12px;
  }

  .toolbar-actions {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 8px;
  }

  .path {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 13.5px;
  }

  .path button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font: inherit;
  }

  .path button:hover {
    color: var(--text);
  }

  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .segmented button {
    display: grid;
    width: 34px;
    height: 32px;
    place-items: center;
    border: 0;
    background: var(--surface);
    color: var(--text-2);
  }

  .segmented button.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .upload-button {
    padding: 6px 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
    cursor: pointer;
    font-size: 13px;
  }

  .upload-button input {
    display: none;
  }

  .selection-bar,
  .zip-hint {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-bottom: 12px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 13.5px;
  }

  .selection-actions {
    display: flex;
    gap: 6px;
  }

  .selection-actions button,
  .zip-hint button,
  .load-more button,
  .upload-queue button {
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
  }

  .selection-actions button:not(.ghost),
  .zip-hint button:not(.ghost) {
    border-color: var(--accent);
    background: var(--accent);
    color: var(--on-accent);
  }

  .ghost {
    background: transparent;
  }

  .share-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 12px;
  }

  .share-card-item {
    position: relative;
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface);
    cursor: default;
  }

  .share-card-item.selected,
  .entry.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .card-check {
    position: absolute;
    top: 8px;
    left: 8px;
    z-index: 2;
  }

  .card-icon {
    position: relative;
    display: grid;
    overflow: hidden;
    aspect-ratio: 4 / 3;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .thumb {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .preview-button {
    position: absolute;
    right: 6px;
    bottom: 6px;
    z-index: 1;
    display: grid;
    width: 28px;
    height: 28px;
    place-items: center;
    border: 1px solid rgb(255 255 255 / 40%);
    border-radius: 50%;
    background: rgb(0 0 0 / 45%);
    color: #fff;
  }

  .entry-name {
    overflow: hidden;
    color: var(--text);
    font-size: 13.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-meta {
    overflow: hidden;
    color: var(--text-2);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entries {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .entry {
    display: grid;
    grid-template-columns: 20px 24px minmax(0, 1fr) 90px auto;
    align-items: center;
    gap: 10px;
    min-height: 44px;
    padding: 0 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
  }

  .entry:hover {
    background: var(--hover);
  }

  .entry-size {
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
  }

  .load-more {
    display: flex;
    justify-content: center;
    margin-top: 16px;
  }

  .share-preview {
    max-width: 100%;
    max-height: 45vh;
    object-fit: contain;
  }

  .file {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 32px;
    color: var(--text-2);
  }

  .upload-queue {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 18px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }

  .upload-queue-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-2);
    font-size: 13px;
  }

  .upload-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 140px 110px auto;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .upload-name {
    overflow: hidden;
    color: var(--text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .upload-track {
    overflow: hidden;
    height: 6px;
    border-radius: 99px;
    background: var(--surface-2);
  }

  .upload-fill {
    height: 100%;
    border-radius: 99px;
    background: var(--accent);
    transition: width 0.2s ease;
  }

  .upload-state {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .text-button {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
  }

  .muted {
    color: var(--text-2);
    font-size: 14.5px;
  }

  .error {
    color: var(--danger);
    font-size: 13.5px;
  }

  @media (max-width: 640px) {
    .share-shell {
      padding: 12px;
    }

    .share-card {
      padding: 16px;
    }

    .share-toolbar {
      flex-wrap: wrap;
    }

    .share-grid {
      grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
      gap: 8px;
    }

    .entry {
      grid-template-columns: 20px 24px minmax(0, 1fr) auto;
    }

    .entry-size {
      display: none;
    }

    .upload-row {
      grid-template-columns: minmax(0, 1fr) 90px auto;
    }

    .upload-track {
      display: none;
    }
  }
</style>
