<script lang="ts">
  import { language, localeOf, t } from '../i18n';
  import { focusTrap } from '../focusTrap';
  import type { FileProperties } from '../api';
  import Icon from './Icon.svelte';
  import FileName from './FileName.svelte';

  let {
    data,
    loading,
    error,
    onClose,
  }: {
    data: FileProperties | null;
    loading: boolean;
    error: string;
    onClose: () => void;
  } = $props();

  let overlayPressed = false;

  function absolute(seconds: number | undefined | null): string {
    if (!seconds) return '—';
    return new Date(seconds * 1000).toLocaleString(localeOf($language));
  }

  function formatSize(size: number | undefined): string {
    if (size === undefined) return '—';
    if (size < 1024) return `${size} B`;
    if (size < 1024 ** 2) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 ** 3) return `${(size / 1024 ** 2).toFixed(1)} MB`;
    return `${(size / 1024 ** 3).toFixed(1)} GB`;
  }

  function kind(): string {
    if (!data) return '—';
    if (data.is_symlink) return t($language, '链接');
    return data.is_dir ? t($language, '目录') : t($language, '文件');
  }

  function exifLabel(key: string): string {
    const labels: Record<string, string> = {
      make: '制造商',
      model: '型号',
      lens: '镜头',
      taken_at: '拍摄时间',
      exposure: '曝光',
      f_number: '光圈',
      iso: 'ISO',
      focal_length: '焦距',
      orientation: '方向',
    };
    return t($language, labels[key] ?? key);
  }

  const exif = $derived(data?.image?.exif ?? null);
</script>

<div
  class="properties-overlay"
  role="presentation"
  onpointerdown={(event) => (overlayPressed = event.target === event.currentTarget)}
  onclick={(event) => {
    if (overlayPressed && event.target === event.currentTarget) onClose();
    overlayPressed = false;
  }}
>
  <dialog
    open
    class="properties-panel"
    aria-labelledby="properties-title"
    use:focusTrap
    onkeydown={(event) => {
      if (event.key === 'Escape') onClose();
    }}
  >
    <header>
      <h2 id="properties-title">{t($language, '详情')}</h2>
      <button type="button" class="icon-button" aria-label={t($language, '关闭')} onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    {#if loading}
      <p class="muted">{t($language, '加载中…')}</p>
    {:else if error}
      <p class="error">{error}</p>
    {:else if data}
      <dl>
        <dt>{t($language, '名称')}</dt>
        <dd><FileName name={data.name} isDir={data.is_dir} /></dd>
        <dt>{t($language, '路径')}</dt>
        <dd class="mono">{data.path || '/'}</dd>
        <dt>{t($language, '类型')}</dt>
        <dd>{kind()}</dd>
        <dt>{t($language, '大小')}</dt>
        <dd>{formatSize(data.size)}</dd>
        <dt>{t($language, 'MIME')}</dt>
        <dd class="mono">{data.mime_type || '—'}</dd>
        {#if data.mode}
          <dt>{t($language, '权限')}</dt>
          <dd class="mono">{data.mode}</dd>
        {/if}
        <dt>{t($language, '创建时间')}</dt>
        <dd>{absolute(data.created)}</dd>
        <dt>{t($language, '修改时间')}</dt>
        <dd>{absolute(data.modified)}</dd>
        <dt>{t($language, '访问时间')}</dt>
        <dd>{absolute(data.accessed)}</dd>
        {#if data.is_symlink}
          <dt>{t($language, '链接状态')}</dt>
          <dd>
            {data.symlink_state === 'ok'
              ? t($language, '可用')
              : data.symlink_state === 'denied'
                ? t($language, '无权限')
                : t($language, '断链')}
          </dd>
          {#if data.symlink_target}
            <dt>{t($language, '目标')}</dt>
            <dd class="mono">{data.symlink_target}</dd>
          {/if}
        {/if}
        {#if data.image}
          <dt>{t($language, '图片尺寸')}</dt>
          <dd>{data.image.width} × {data.image.height}</dd>
          <dt>{t($language, '格式')}</dt>
          <dd class="mono">{data.image.format}</dd>
          {#if exif}
            {#each Object.entries(exif) as [key, value] (key)}
              <dt>{exifLabel(key)}</dt>
              <dd>{String(value)}</dd>
            {/each}
          {/if}
        {/if}
      </dl>
    {/if}
  </dialog>
</div>

<style>
  .properties-overlay {
    position: fixed;
    inset: 0;
    z-index: 85;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .properties-panel {
    width: min(560px, 100%);
    max-height: min(80vh, 720px);
    overflow-y: auto;
    padding: 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-2);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }

  dd {
    min-width: 0;
  }

  h2 {
    margin: 0;
    font-size: 20px;
  }

  .icon-button {
    border: 0;
    background: transparent;
    color: var(--text-2);
  }

  dl {
    display: grid;
    grid-template-columns: minmax(90px, 30%) minmax(0, 1fr);
    gap: 8px 14px;
    margin: 0;
    font-size: 13.5px;
  }

  dt {
    color: var(--text-2);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .mono {
    font-family: var(--font-mono);
    font-size: 12.5px;
  }

  .muted {
    color: var(--text-2);
  }

  .error {
    color: var(--danger);
  }
</style>
