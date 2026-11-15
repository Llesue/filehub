<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type Entry, type Root } from '../api';
  import Icon from './Icon.svelte';

  let {
    open,
    mode,
    count,
    roots,
    initialRoot,
    initialPath,
    onConfirm,
    onClose,
  }: {
    open: boolean;
    mode: 'copy' | 'move';
    count: number;
    roots: Root[];
    initialRoot: number | null;
    initialPath: string;
    onConfirm: (rootId: number, path: string) => void;
    onClose: () => void;
  } = $props();

  let rootId = $state(0);
  let path = $state('');
  let directories = $state<Entry[]>([]);
  let loading = $state(false);
  let error = $state('');
  let overlayPressed = false;

  onMount(() => {
    rootId = initialRoot ?? roots[0]?.id ?? 0;
    path = initialPath;
    void loadDirectories();
  });

  async function loadDirectories(): Promise<void> {
    if (!rootId) return;
    loading = true;
    error = '';
    try {
      const result = await api.list(rootId, path, 'name', 'asc', false);
      directories = result.entries.filter((entry) => entry.is_dir);
    } catch (err) {
      error = err instanceof Error ? err.message : '目录加载失败';
      directories = [];
    } finally {
      loading = false;
    }
  }

  function changeRoot(event: Event): void {
    rootId = Number((event.currentTarget as HTMLSelectElement).value);
    path = '';
    void loadDirectories();
  }

  function enter(directory: Entry): void {
    path = path ? `${path}/${directory.name}` : directory.name;
    void loadDirectories();
  }

  function up(): void {
    path = path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : '';
    void loadDirectories();
  }

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (rootId) onConfirm(rootId, path);
  }
</script>

{#if open}
  <div
    class="dialog-overlay"
    role="presentation"
    onpointerdown={(event) => (overlayPressed = event.target === event.currentTarget)}
    onclick={(event) => {
      if (overlayPressed && event.target === event.currentTarget) onClose();
      overlayPressed = false;
    }}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <form class="transfer-dialog" onsubmit={submit} onclick={(event) => event.stopPropagation()}>
      <h2>{mode === 'copy' ? '复制到' : '移动到'}</h2>
      <p class="hint">共 {count} 项，点击选择目标目录。</p>
      <label>
        <span>存储目录</span>
        <select value={rootId} onchange={changeRoot}>
          {#each roots as root (root.id)}
            <option value={root.id}>{root.alias}</option>
          {/each}
        </select>
      </label>
      <div class="picker-path">
        <button
          type="button"
          class="path-button"
          onclick={() => {
            path = '';
            void loadDirectories();
          }}
        >
          /
        </button>
        {#each path.split('/').filter(Boolean) as segment, index (index)}
          <span>/</span>
          <button
            type="button"
            class="path-button"
            onclick={() => {
              path = path
                .split('/')
                .slice(0, index + 1)
                .join('/');
              void loadDirectories();
            }}>{segment}</button
          >
        {/each}
        {#if path}<button type="button" class="ghost small" onclick={up}>上一级</button>{/if}
      </div>
      <div class="picker-list">
        {#if loading}
          <p class="hint">加载中…</p>
        {:else if error}
          <p class="error">{error}</p>
        {:else if directories.length === 0}
          <p class="hint">当前目录没有子目录，可直接选择这里。</p>
        {:else}
          {#each directories as directory (directory.name)}
            <button
              type="button"
              class="directory-row"
              ondblclick={() => enter(directory)}
              onclick={() => enter(directory)}
            >
              <Icon name="folder" size={18} />
              <span>{directory.name}</span>
            </button>
          {/each}
        {/if}
      </div>
      <p class="hint">目标目录：<strong>/{path}</strong></p>
      <div class="dialog-actions">
        <button type="button" class="ghost" onclick={onClose}>取消</button>
        <button type="submit">{mode === 'copy' ? '复制到这里' : '移动到这里'}</button>
      </div>
    </form>
  </div>
{/if}

<style>
  .dialog-overlay {
    position: fixed;
    inset: 0;
    z-index: 70;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .transfer-dialog {
    display: grid;
    width: min(560px, 100%);
    gap: 12px;
    padding: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 600;
  }

  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 13.5px;
  }

  label {
    display: grid;
    gap: 6px;
    color: var(--text-2);
    font-size: 14px;
  }

  select {
    width: 100%;
    height: 40px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text);
  }

  .picker-path {
    display: flex;
    min-height: 32px;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12.5px;
  }

  .path-button {
    padding: 2px 4px;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--accent-text);
  }

  .path-button:hover {
    background: var(--hover);
  }

  .picker-list {
    max-height: 260px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .directory-row {
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

  .directory-row:hover {
    background: var(--hover);
  }

  .error {
    margin: 0;
    padding: 12px;
    color: var(--danger);
    font-size: 13px;
  }

  .small {
    height: 26px;
    margin-left: auto;
    padding: 0 8px;
    font-size: 12px;
  }

  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .dialog-actions button {
    height: 38px;
    padding: 0 16px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }

  .dialog-actions button.ghost {
    border-color: var(--border);
    background: transparent;
    color: var(--text);
  }
</style>
