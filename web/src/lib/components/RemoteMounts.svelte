<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type RemoteGrant, type RemoteMount } from '../api';
  import { language, t } from '../i18n';

  let { csrf }: { csrf: string } = $props();

  let mounts = $state<RemoteMount[]>([]);
  let url = $state('');
  let token = $state('');
  let grants = $state<RemoteGrant[]>([]);
  let selected = $state<number | null>(null);
  let alias = $state('');
  let busy = $state(false);
  let error = $state('');

  onMount(() => {
    void load();
  });

  function message(value: unknown): string {
    if (value instanceof ApiError) return value.message;
    if (value instanceof Error) return value.message;
    return String(value);
  }

  async function load(): Promise<void> {
    try {
      mounts = (await api.remoteMounts()).mounts;
    } catch (value) {
      error = message(value);
    }
  }

  async function preview(): Promise<void> {
    busy = true;
    error = '';
    grants = [];
    selected = null;
    alias = '';
    try {
      const result = await api.remotePreview(url.trim(), token.trim(), csrf);
      grants = result.grants;
      const first = grants[0];
      if (grants.length === 1 && first) {
        selected = first.id;
        alias = first.label;
      }
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function mount(): Promise<void> {
    if (selected === null) return;
    busy = true;
    error = '';
    try {
      await api.remoteCreateMount(url.trim(), token.trim(), selected, alias.trim(), csrf);
      url = '';
      token = '';
      grants = [];
      selected = null;
      alias = '';
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function remove(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      await api.remoteDeleteMount(id, csrf);
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function test(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      const result = await api.remoteTestMount(id, csrf);
      if (!result.ok) {
        error = result.message ?? t($language, '连接失败');
      }
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }
</script>

<section class="pref-card">
  <h2>{t($language, '远程目录')}</h2>
  <p class="hint">{t($language, '挂载另一个 FileHub 实例授权的目录。')}</p>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <div class="form">
    <label>
      <span>{t($language, '地址')}</span>
      <input bind:value={url} placeholder="https://nas.example.com" autocomplete="off" />
    </label>
    <label>
      <span>{t($language, '令牌')}</span>
      <input bind:value={token} type="password" placeholder="fhfed_..." autocomplete="off" />
    </label>
    <button
      type="button"
      disabled={busy || !url.trim() || !token.trim()}
      onclick={() => void preview()}
    >
      {t($language, '预览目录')}
    </button>
  </div>

  {#if grants.length > 0}
    <div class="grants">
      {#each grants as grant (grant.id)}
        <label class="grant">
          <input type="radio" bind:group={selected} value={grant.id} />
          <span>{grant.label}</span>
          <em class:rw={grant.mode === 'rw'}>
            {grant.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
          </em>
        </label>
      {/each}
      <label class="alias">
        <span>{t($language, '挂载名称')}</span>
        <input bind:value={alias} />
      </label>
      <button type="button" disabled={busy || selected === null} onclick={() => void mount()}>
        {t($language, '挂载')}
      </button>
    </div>
  {/if}

  {#if mounts.length > 0}
    <div class="list">
      {#each mounts as item (item.id)}
        <div class="row">
          <div class="meta">
            <strong>{item.alias}</strong>
            <span class="url">{item.base_url}</span>
            <em class:rw={item.mode === 'rw'}>
              {item.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
            </em>
            <span class="state" class:offline={item.state !== 'online'}>
              {item.state === 'online' ? t($language, '在线') : t($language, '离线')}
            </span>
          </div>
          <div class="actions">
            <button type="button" disabled={busy} onclick={() => void test(item.id)}>
              {t($language, '测试连接')}
            </button>
            <button
              type="button"
              class="danger"
              disabled={busy}
              onclick={() => void remove(item.id)}
            >
              {t($language, '断开挂载')}
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .pref-card {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 18px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
  }

  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  .hint {
    margin: 0 0 12px;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .error {
    margin: 0 0 10px;
    color: var(--danger);
    font-size: 12.5px;
  }

  .form,
  .grants {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: flex-end;
  }

  .form label,
  .alias {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 220px;
    flex: 1;
    font-size: 12.5px;
    color: var(--text-2);
  }

  input[type='password'],
  input:not([type]) {
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  button {
    padding: 7px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .grants {
    margin-top: 12px;
  }

  .grant {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .grant em,
  .meta em {
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .grant em.rw,
  .meta em.rw {
    color: var(--danger);
  }

  .list {
    margin-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 12px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .meta {
    display: flex;
    gap: 10px;
    align-items: center;
    min-width: 0;
  }

  .meta .url {
    color: var(--text-2);
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state {
    font-size: 11.5px;
    color: var(--accent);
  }

  .state.offline {
    color: var(--danger);
  }

  .actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .danger {
    color: var(--danger);
  }
</style>
