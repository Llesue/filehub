<script lang="ts">
  import { onMount } from 'svelte';
  import type { Entry } from '../api';
  import { classifyFile } from '../fileKind';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';

  let {
    rootId,
    currentPath,
    entries,
    dirSizes,
    onClose,
  }: {
    rootId: number;
    currentPath: string;
    entries: Entry[];
    dirSizes: Record<string, { size: number; stale: boolean } | null>;
    onClose: () => void;
  } = $props();

  let closeButton = $state<HTMLButtonElement | null>(null);

  function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    if (bytes < 1024 ** 4) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    return `${(bytes / 1024 ** 4).toFixed(1)} TB`;
  }

  function pathOf(entry: Entry): string {
    return currentPath ? `${currentPath}/${entry.name}` : entry.name;
  }

  function dirSize(entry: Entry): { size: number; stale: boolean } | null {
    return dirSizes[`${rootId}:${pathOf(entry)}`] ?? null;
  }

  let summary = $derived.by(() => {
    let files = 0;
    let dirs = 0;
    let bytes = 0;
    let unknownDirs = 0;
    const types: Record<string, number> = {};
    for (const entry of entries) {
      const kind = classifyFile(entry.name, entry.is_dir, entry.mime_type);
      const label = entry.is_dir
        ? t($language, '目录')
        : kind === 'image'
          ? t($language, '图片')
          : kind === 'video'
            ? t($language, '视频')
            : kind === 'audio'
              ? t($language, '音频')
              : kind === 'pdf' || kind === 'text'
                ? t($language, '文档')
                : t($language, '其他');
      types[label] = (types[label] ?? 0) + 1;
      if (entry.is_dir) {
        dirs += 1;
        const size = dirSize(entry);
        if (size) bytes += size.size;
        else unknownDirs += 1;
      } else {
        files += 1;
        bytes += entry.size;
      }
    }
    const largestFiles = entries
      .filter((entry) => !entry.is_dir)
      .slice()
      .sort((left, right) => right.size - left.size)
      .slice(0, 8);
    const largestDirs = entries
      .filter((entry) => entry.is_dir)
      .map((entry) => ({ entry, size: dirSize(entry) }))
      .filter((item): item is { entry: Entry; size: { size: number; stale: boolean } } =>
        Boolean(item.size),
      )
      .sort((left, right) => right.size.size - left.size.size)
      .slice(0, 8);
    return {
      files,
      dirs,
      bytes,
      unknownDirs,
      types: Object.entries(types).sort((left, right) => right[1] - left[1]),
      largestFiles,
      largestDirs,
    };
  });

  onMount(() => closeButton?.focus());
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === 'Escape') onClose();
  }}
/>

<div class="overlay" role="presentation" onclick={onClose}>
  <!-- svelte-ignore a11y_no_noninteractive_element_to_interactive_role -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <section
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="dir-stats-title"
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2 id="dir-stats-title">{t($language, '目录概览')}</h2>
      <button type="button" class="icon-button" bind:this={closeButton} onclick={onClose}>
        <Icon name="close" size={16} />
      </button>
    </header>

    <div class="summary">
      <div><strong>{summary.dirs}</strong><span>{t($language, '目录')}</span></div>
      <div><strong>{summary.files}</strong><span>{t($language, '文件')}</span></div>
      <div>
        <strong>{formatSize(summary.bytes)}</strong><span>{t($language, '已知大小')}</span>
      </div>
      {#if summary.unknownDirs > 0}
        <div>
          <strong>{summary.unknownDirs}</strong><span>{t($language, '目录大小未知')}</span>
        </div>
      {/if}
    </div>

    <div class="types">
      {#each summary.types as [label, count] (label)}
        <span class="tag">{label} {count}</span>
      {/each}
    </div>

    <div class="lists">
      <section>
        <h3>{t($language, '最大文件')}</h3>
        {#if summary.largestFiles.length === 0}
          <p class="empty">{t($language, '没有文件')}</p>
        {:else}
          {#each summary.largestFiles as entry (entry.name)}
            <div class="row"><span title={entry.name}>{entry.name}</span><em>{formatSize(entry.size)}</em></div>
          {/each}
        {/if}
      </section>
      <section>
        <h3>{t($language, '最大目录')}</h3>
        {#if summary.largestDirs.length === 0}
          <p class="empty">{t($language, '目录大小未知')}</p>
        {:else}
          {#each summary.largestDirs as item (item.entry.name)}
            <div class="row">
              <span title={item.entry.name}>{item.entry.name}</span>
              <em>{formatSize(item.size.size)}{item.size.stale ? ' *' : ''}</em>
            </div>
          {/each}
        {/if}
      </section>
    </div>
  </section>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    background: var(--scrim);
  }

  .dialog {
    width: min(720px, calc(100vw - 32px));
    max-height: min(80vh, 720px);
    overflow: auto;
    padding: 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  h2 {
    margin: 0;
    font-size: 17px;
  }

  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    margin: 14px 0;
  }

  .summary div {
    display: flex;
    min-width: 110px;
    flex-direction: column;
    padding: 8px 10px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .summary strong {
    font-size: 16px;
  }

  .summary span {
    color: var(--text-2);
    font-size: 12px;
  }

  .types {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-bottom: 12px;
  }

  .tag {
    padding: 2px 8px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 12px;
  }

  .lists {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 16px;
  }

  h3 {
    margin: 0 0 6px;
    font-size: 13px;
    color: var(--text-2);
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    padding: 4px 0;
    border-bottom: 1px solid var(--border);
    font-size: 13px;
  }

  .row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row em {
    flex: 0 0 auto;
    color: var(--text-2);
    font-style: normal;
  }

  .empty {
    color: var(--text-2);
    font-size: 13px;
  }

  .icon-button {
    display: grid;
    width: 28px;
    height: 28px;
    padding: 0;
    place-items: center;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
  }

  .icon-button:hover {
    background: var(--surface-2);
    color: var(--text);
  }
</style>
