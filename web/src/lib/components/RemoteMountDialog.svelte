<script lang="ts">
  import { api, ApiError, type RemoteMount } from '../api';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';
  import RemoteMountWizard from './RemoteMountWizard.svelte';

  let {
    open,
    csrf,
    onClose,
    onMounted,
  }: {
    open: boolean;
    csrf: string;
    onClose: () => void;
    onMounted: (rootId: number | null) => void;
  } = $props();

  let view = $state<'list' | 'wizard'>('list');
  let mounts = $state<RemoteMount[]>([]);
  let busy = $state(false);
  let error = $state('');
  let reconnectId = $state<number | null>(null);
  let reconnectToken = $state('');
  let confirmRemove = $state<number | null>(null);
  let renameId = $state<number | null>(null);
  let renameAlias = $state('');
  let latency = $state<Record<number, number>>({});

  $effect(() => {
    if (open) {
      view = 'list';
      confirmRemove = null;
      reconnectId = null;
      reconnectToken = '';
      void load();
    }
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

  async function test(id: number): Promise<void> {
    busy = true;
    error = '';
    try {
      const result = await api.remoteTestMount(id, csrf);
      if (!result.ok) {
        error = result.message ?? t($language, '连接失败');
      } else {
        latency = { ...latency, [id]: result.latency_ms };
      }
      await load();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function remove(id: number): Promise<void> {
    if (confirmRemove !== id) {
      confirmRemove = id;
      return;
    }
    busy = true;
    error = '';
    try {
      await api.remoteDeleteMount(id, csrf);
      confirmRemove = null;
      await load();
      onMounted(null);
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function rename(id: number): Promise<void> {
    if (!renameAlias.trim()) return;
    busy = true;
    error = '';
    try {
      await api.remoteRenameMount(id, renameAlias.trim(), csrf);
      renameId = null;
      renameAlias = '';
      await load();
      onMounted(null);
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function reconnect(id: number): Promise<void> {
    if (!reconnectToken.trim()) return;
    busy = true;
    error = '';
    try {
      await api.remoteReauthMount(id, reconnectToken.trim(), csrf);
      reconnectId = null;
      reconnectToken = '';
      await load();
      onMounted(null);
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  function latencyClass(value: number): string {
    if (value <= 100) return 'good';
    if (value <= 300) return 'warn';
    return 'bad';
  }

  function cacheLabel(bytes: number | undefined): string {
    if (!bytes) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB'];
    let value = bytes;
    let unit = 0;
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024;
      unit += 1;
    }
    return `${value.toFixed(unit === 0 ? 0 : 1)} ${units[unit]}`;
  }

  function stateLabel(state: RemoteMount['state']): string {
    switch (state) {
      case 'online':
        return t($language, '在线');
      case 'offline':
        return t($language, '离线');
      case 'token_invalid':
        return t($language, '令牌无效');
      case 'grant_revoked':
        return t($language, '授权已撤销');
      case 'unsupported':
        return t($language, '对端不支持');
    }
  }

  function close(): void {
    view = 'list';
    onClose();
  }
</script>

{#if open}
  {#if view === 'wizard'}
    <RemoteMountWizard
      open={true}
      {csrf}
      onClose={() => {
        view = 'list';
        void load();
      }}
      onMounted={(rootId) => {
        onMounted(rootId);
        void load();
      }}
    />
  {:else}
    <div
      class="backdrop"
      role="presentation"
      onclick={(event) => {
        if (event.target === event.currentTarget) close();
      }}
    >
      <div class="dialog" role="dialog" aria-modal="true" aria-label={t($language, '远程目录')}>
        <header>
          <h2>{t($language, '远程目录')}</h2>
          <button type="button" class="icon" aria-label={t($language, '关闭')} onclick={close}>
            <Icon name="close" size={16} />
          </button>
        </header>

        <p class="hint">{t($language, '挂载另一个 FileHub 实例授权的目录。')}</p>

        {#if error}
          <p class="error">{error}</p>
        {/if}

        {#if mounts.length === 0}
          <p class="empty">{t($language, '还没有挂载远程目录。')}</p>
        {:else}
          <div class="list">
            {#each mounts as item (item.id)}
              <div class="row">
                <div class="meta">
                  <strong>{item.alias}</strong>
                  <span class="url">{item.base_url}</span>
                  <em class:rw={item.mode === 'rw'}>
                    {item.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
                  </em>
                  <span class="state" class:bad={item.state !== 'online'}>
                    {stateLabel(item.state)}
                  </span>
                  {#if latency[item.id] !== undefined}
                    <span class="latency {latencyClass(latency[item.id] ?? 0)}">
                      {latency[item.id]} ms
                    </span>
                  {/if}
                  <span class="latency">{t($language, '缩略图缓存')} {cacheLabel(item.thumb_cache_bytes)}</span>
                </div>
                <div class="actions">
                  <button type="button" disabled={busy} onclick={() => void test(item.id)}>
                    {t($language, '测试连接')}
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onclick={() => {
                      renameId = renameId === item.id ? null : item.id;
                      renameAlias = item.alias;
                      reconnectId = null;
                      confirmRemove = null;
                    }}
                  >
                    {t($language, '改名')}
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onclick={() => {
                      reconnectId = reconnectId === item.id ? null : item.id;
                      reconnectToken = '';
                      renameId = null;
                      confirmRemove = null;
                    }}
                  >
                    {t($language, '重新连接')}
                  </button>
                  <button
                    type="button"
                    class="danger"
                    disabled={busy}
                    onclick={() => void remove(item.id)}
                  >
                    {confirmRemove === item.id
                      ? t($language, '确认断开')
                      : t($language, '断开挂载')}
                  </button>
                </div>
              </div>
              {#if confirmRemove === item.id}
                <p class="warn">{t($language, '只删除本地挂载记录，不会删除对方文件。')}</p>
              {/if}
              {#if renameId === item.id}
                <div class="reconnect">
                  <input bind:value={renameAlias} autocomplete="off" />
                  <button
                    type="button"
                    disabled={busy || !renameAlias.trim()}
                    onclick={() => void rename(item.id)}
                  >
                    {t($language, '保存')}
                  </button>
                </div>
              {/if}
              {#if reconnectId === item.id}
                <div class="reconnect">
                  <input
                    type="password"
                    bind:value={reconnectToken}
                    placeholder="fhfed_..."
                    autocomplete="off"
                  />
                  <button
                    type="button"
                    disabled={busy || !reconnectToken.trim()}
                    onclick={() => void reconnect(item.id)}
                  >
                    {t($language, '保存令牌')}
                  </button>
                </div>
              {/if}
            {/each}
          </div>
        {/if}

        <footer>
          <button type="button" class="ghost" onclick={close}>
            {t($language, '关闭')}
          </button>
          <button type="button" class="primary" onclick={() => (view = 'wizard')}>
            {t($language, '添加挂载')}
          </button>
        </footer>
      </div>
    </div>
  {/if}
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 80;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.35);
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: min(680px, calc(100vw - 32px));
    max-height: calc(100vh - 64px);
    padding: 20px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
  }

  header {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }

  .icon {
    margin-left: auto;
    border: none;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }

  .hint,
  .empty,
  .warn {
    margin: 0;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 12.5px;
  }

  .warn {
    padding: 0 10px 8px;
    color: var(--warning, var(--danger));
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    min-width: 0;
  }

  .meta .url {
    overflow: hidden;
    max-width: 280px;
    color: var(--text-2);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta em {
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .meta em.rw {
    color: var(--success, #2e7d32);
  }

  .state {
    font-size: 11.5px;
    color: var(--success, #2e7d32);
  }

  .state.bad {
    color: var(--danger);
  }

  .latency {
    font-size: 11.5px;
    color: var(--text-2);
  }

  .latency.good {
    color: var(--success, #2e7d32);
  }

  .latency.warn {
    color: var(--warning, #b7791f);
  }

  .latency.bad {
    color: var(--danger);
  }

  .actions {
    display: flex;
    flex: 0 0 auto;
    gap: 6px;
  }

  .reconnect {
    display: flex;
    gap: 8px;
    padding: 0 10px 8px;
  }

  .reconnect input {
    flex: 1;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  button {
    padding: 7px 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }

  button.primary {
    background: var(--accent);
    color: #fff;
  }

  button.ghost {
    background: var(--surface-2);
  }

  button.danger {
    color: var(--danger);
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (max-width: 640px) {
    .dialog {
      width: 100vw;
      height: 100vh;
      max-height: none;
      border-radius: 0;
    }

    .row {
      align-items: flex-start;
      flex-direction: column;
    }

    .actions {
      flex-wrap: wrap;
    }
  }
</style>
