<script lang="ts">
  import { onMount } from 'svelte';
  import FileTypeIcon from './FileTypeIcon.svelte';
  import { api, ApiError, type ShareEntry, type ShareMeta } from '../api';
  import { language, t } from '../i18n';

  let { token }: { token: string } = $props();

  let data = $state<ShareMeta | null>(null);
  let path = $state('');
  let ticket = $state<string | undefined>(undefined);
  let password = $state('');
  let needsPassword = $state(false);
  let loading = $state(true);
  let error = $state('');
  let previewUrl = $state<string | null>(null);
  let uploading = $state(false);

  onMount(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    error = '';
    try {
      data = await api.publicShare(token, path, ticket);
      needsPassword = false;
    } catch (err) {
      if (err instanceof ApiError && err.status === 401) {
        needsPassword = true;
      } else {
        error = err instanceof Error ? err.message : t($language, '无法打开分享');
      }
    } finally {
      loading = false;
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
    void load();
  }

  function join(base: string, name: string): string {
    return base ? `${base}/${name}` : name;
  }

  async function download(entry?: ShareEntry): Promise<void> {
    try {
      const target = entry ? join(path, entry.name) : path;
      const blob = await api.downloadShared(token, target, ticket);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = entry?.name ?? data?.name ?? 'download';
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '下载失败');
    }
  }

  function isImage(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?)$/i.test(name);
  }

  function isVideo(name: string): boolean {
    return /\.(mp4|mov|mkv|webm|avi|m4v)$/i.test(name);
  }

  async function upload(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    if (!input.files || input.files.length === 0) return;
    uploading = true;
    error = '';
    try {
      for (const file of Array.from(input.files)) {
        await api.uploadShared(token, path, file.name, file, ticket);
      }
      input.value = '';
      await load();
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '上传失败');
    } finally {
      uploading = false;
    }
  }

  function formatSize(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
    return `${(size / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }
</script>

<svelte:head>
  <title>{t($language, 'FileHub 分享')}</title>
</svelte:head>

<main class="share-shell">
  <section class="share-card">
    <header class="share-header">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">F</span>
        <span>{t($language, 'FileHub 分享')}</span>
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
            {#each path.split('/').filter(Boolean) as segment, index (index)}
              <span>/</span>
              <button
                type="button"
                onclick={() =>
                  navigate(
                    path
                      .split('/')
                      .slice(0, index + 1)
                      .join('/'),
                  )}
              >
                {segment}
              </button>
            {/each}
          </nav>
          {#if !data.read_only}
            <label class="upload-button">
              {uploading ? t($language, '上传中…') : t($language, '上传文件')}
              <input
                type="file"
                multiple
                onchange={(event) => void upload(event)}
                disabled={uploading}
              />
            </label>
          {/if}
        </div>
        <div class="entries">
          {#each data.entries ?? [] as entry (entry.name)}
            <button
              type="button"
              class="entry"
              onclick={() =>
                entry.is_dir ? navigate(join(path, entry.name)) : void download(entry)}
            >
              <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={22} />
              <span class="entry-name">{entry.name}</span>
              <span class="entry-size">{entry.is_dir ? '—' : formatSize(entry.size)}</span>
            </button>
          {/each}
          {#if (data.entries ?? []).length === 0}
            <p class="muted">{t($language, '这个目录是空的')}</p>
          {/if}
        </div>
      {:else}
        <div class="file">
          {#if previewUrl && isImage(data.name)}
            <img class="share-preview" src={previewUrl} alt={data.name} />
          {:else if previewUrl && isVideo(data.name)}
            <video class="share-preview" src={previewUrl} controls><track kind="captions" /></video>
          {:else}
            <FileTypeIcon name={data.name} size={42} />
          {/if}
          <p>{data.size !== undefined ? formatSize(data.size) : ''}</p>
          <button type="button" class="download" onclick={() => void download()}
            >{t($language, '下载文件')}</button
          >
        </div>
      {/if}
    {/if}
  </section>
</main>

<style>
  .share-shell {
    display: grid;
    min-height: 100dvh;
    place-items: center;
    padding: 24px;
    background: var(--bg);
  }

  .share-card {
    width: min(100%, 760px);
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

  .path {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-bottom: 12px;
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

  .share-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 12px;
  }

  .share-toolbar .path {
    margin-bottom: 0;
  }

  .upload-button {
    flex: 0 0 auto;
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

  .entries {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .entry {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) 80px;
    align-items: center;
    gap: 10px;
    min-height: 42px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
  }

  .entry:hover {
    background: var(--hover);
  }

  .entry > :first-child {
    color: var(--text-2);
  }

  .entry-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-size {
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
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

  .muted {
    color: var(--text-2);
    font-size: 14.5px;
  }

  .error {
    color: var(--danger);
    font-size: 14.5px;
  }

  @media (max-width: 480px) {
    .share-shell {
      padding: 12px;
    }

    .share-card {
      padding: 18px;
    }
  }
</style>
