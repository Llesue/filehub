<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Entry, type Root } from '../api';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    roots,
    initialRoot,
    initialPath,
    onSelect,
    onClose,
  }: {
    roots: Root[];
    initialRoot: number | null;
    initialPath: string;
    onSelect: (rootId: number, path: string) => void;
    onClose: () => void;
  } = $props();

  let rootId = $state(0);
  let path = $state('');
  let entries = $state<Entry[]>([]);
  let error = $state('');
  let loading = $state(false);

  onMount(() => {
    rootId = initialRoot ?? roots[0]?.id ?? 0;
    path = initialPath;
    void load();
  });

  async function load(): Promise<void> {
    if (!rootId) return;
    loading = true;
    error = '';
    try {
      entries = (await api.list(rootId, path, 'name', 'asc', false)).entries;
    } catch (err) {
      error = err instanceof Error ? err.message : t($language, '目录加载失败');
    } finally {
      loading = false;
    }
  }

  function enter(entry: Entry): void {
    if (entry.is_dir) {
      path = path ? `${path}/${entry.name}` : entry.name;
      void load();
    } else if (entry.name.toLowerCase().endsWith('.css')) {
      onSelect(rootId, path ? `${path}/${entry.name}` : entry.name);
    }
  }
</script>

<div class="dialog-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <section
    class="css-picker"
    aria-label={t($language, '选择 CSS 文件')}
    onclick={(event) => event.stopPropagation()}
  >
    <header>
      <h2>{t($language, '选择自定义 CSS')}</h2>
      <button type="button" class="icon-button" aria-label={t($language, '关闭')} onclick={onClose}
        ><Icon name="close" size={18} /></button
      >
    </header>
    <select
      value={rootId}
      onchange={(event) => {
        rootId = Number((event.currentTarget as HTMLSelectElement).value);
        path = '';
        void load();
      }}
    >
      {#each roots as root (root.id)}<option value={root.id}>{root.alias}</option>{/each}
    </select>
    <div class="crumbs">/{path}</div>
    <div class="picker-list">
      {#if loading}<p class="muted">{t($language, '加载中…')}</p>{:else if error}<p class="error">
          {error}
        </p>{:else}
        {#each entries as entry (entry.name)}
          {#if entry.is_dir || entry.name.toLowerCase().endsWith('.css')}
            <button type="button" class="row" onclick={() => enter(entry)}>
              <Icon name={entry.is_dir ? 'folder' : 'file'} size={18} />
              <span>{entry.name}</span>
              {#if !entry.is_dir}<span class="tag">CSS</span>{/if}
            </button>
          {/if}
        {/each}
      {/if}
    </div>
  </section>
</div>

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }
  .css-picker {
    display: grid;
    width: min(520px, 100%);
    gap: 12px;
    padding: 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 20px;
  }
  select {
    height: 38px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }
  .crumbs {
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .picker-list {
    max-height: 320px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }
  .row {
    display: flex;
    width: 100%;
    align-items: center;
    gap: 10px;
    height: 40px;
    padding: 0 12px;
    border: 0;
    border-bottom: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .row:hover {
    background: var(--hover);
  }
  .tag {
    margin-left: auto;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 11px;
  }
  .muted {
    margin: 12px;
    color: var(--text-2);
    font-size: 13px;
  }
  .error {
    margin: 12px;
    color: var(--danger);
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
