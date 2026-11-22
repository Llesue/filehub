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

  let view = $state<'list' | 'wizard' | 'grants'>('list');
  let mounts = $state<RemoteMount[]>([]);
  let busy = $state(false);
  let error = $state('');
  let reconnectId = $state<number | null>(null);
  let reconnectToken = $state('');
  let confirmRemove = $state<number | null>(null);
  let renameId = $state<number | null>(null);
  let renameAlias = $state('');
  let latency = $state<Record<number, number>>({});
  let grantsMount = $state<RemoteMount | null>(null);
  let availableGrants = $state<
    Array<{
      id: number;
      label: string;
      mode: 'ro' | 'rw';
      mounted: boolean;
      mount_id: number | null;
      current: boolean;
    }>
  >([]);
  let grantsBusy = $state(false);
  let grantsError = $state('');
  let menuId = $state<number | null>(null);
  let addressEditId = $state<number | null>(null);
  let addressEditText = $state('');
  let addressEditBusy = $state(false);
  let addressEditError = $state('');
  let menuPosition = $state<{ left: number; top: number; maxHeight: number } | null>(null);
  const menuMount = $derived(menuId === null ? null : (mounts.find((item) => item.id === menuId) ?? null));

  $effect(() => {
    if (open) {
      view = 'list';
      confirmRemove = null;
      reconnectId = null;
      reconnectToken = '';
      menuId = null;
      addressEditId = null;
      addressEditError = '';
      void load();
    }
  });

  $effect(() => {
    if (menuId === null) return;
    const close = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('.row-actions') || target?.closest('.row-menu')) return;
      menuId = null;
      menuPosition = null;
    };
    window.addEventListener('pointerdown', close);
    return () => window.removeEventListener('pointerdown', close);
  });

  function message(value: unknown): string {
    if (value instanceof ApiError) return value.message;
    if (value instanceof Error) return value.message;
    return String(value);
  }

  $effect(() => {
    if (menuId === null) return;
    const close = () => {
      menuId = null;
      menuPosition = null;
    };
    const onScroll = (event: Event) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest('.row-menu')) return;
      close();
    };
    window.addEventListener('scroll', onScroll, true);
    window.addEventListener('resize', close);
    return () => {
      window.removeEventListener('scroll', onScroll, true);
      window.removeEventListener('resize', close);
    };
  });

  function openRowMenu(event: MouseEvent, mount: RemoteMount): void {
    event.stopPropagation();
    if (menuId === mount.id) {
      menuId = null;
      menuPosition = null;
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const width = 168;
    const margin = 8;
    const left = Math.max(margin, Math.min(rect.right - width, window.innerWidth - width - margin));
    const height = 168;
    const up = rect.bottom + height + margin > window.innerHeight;
    const top = up
      ? Math.max(margin, rect.top - height - 4)
      : rect.bottom + 4;
    menuId = mount.id;
    menuPosition = {
      left,
      top,
      maxHeight: Math.max(120, window.innerHeight - top - margin),
    };
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
        if (result.removed) {
          onMounted(null);
          await load();
          return;
        }
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

  async function openMountGrants(mount: RemoteMount): Promise<void> {
    grantsMount = mount;
    view = 'grants';
    grantsError = '';
    grantsBusy = true;
    try {
      const result = await api.remoteMountGrants(mount.id);
      availableGrants = result.grants;
    } catch (value) {
      grantsError = message(value);
      availableGrants = [];
    } finally {
      grantsBusy = false;
    }
  }

  async function attachGrant(grantId: number): Promise<void> {
    if (!grantsMount) return;
    grantsBusy = true;
    grantsError = '';
    try {
      const result = await api.remoteAttachGrant(grantsMount.id, grantId, csrf);
      onMounted(result.root_id);
      await openMountGrants(grantsMount);
      await load();
    } catch (value) {
      grantsError = message(value);
    } finally {
      grantsBusy = false;
    }
  }

  function activeAddress(mount: RemoteMount): string {
    return (
      mount.addresses?.find((address) => address.active)?.url ??
      mount.base_url
    );
  }

  function addressCount(mount: RemoteMount): number {
    return mount.addresses?.length ?? 0;
  }

  function addressTitle(mount: RemoteMount): string {
    const addresses = mount.addresses ?? [];
    if (addresses.length === 0) return mount.base_url;
    return addresses
      .map(
        (address) =>
          `${address.active ? '✓ ' : ''}${address.url}${
            address.state && address.state !== 'online' ? ` (${address.state})` : ''
          }`,
      )
      .join('\n');
  }

  function openAddressEditor(mount: RemoteMount): void {
    addressEditId = mount.id;
    addressEditText = (mount.addresses?.length
      ? mount.addresses.map((address) => address.url)
      : [mount.base_url]
    ).join('\n');
    addressEditError = '';
    menuId = null;
  }

  async function saveAddresses(mount: RemoteMount): Promise<void> {
    const urls = addressEditText
      .split(/[\n,]+/)
      .map((value) => value.trim())
      .filter(Boolean);
    if (urls.length === 0) {
      addressEditError = t($language, '至少需要一个地址');
      return;
    }
    addressEditBusy = true;
    addressEditError = '';
    try {
      await api.remoteUpdateMountAddresses(mount.id, urls, csrf);
      addressEditId = null;
      await load();
      const updated = mounts.find((item) => item.id === mount.id) ?? null;
      if (view === 'grants' && updated) {
        grantsMount = updated;
        await openMountGrants(updated);
      }
      onMounted(null);
    } catch (value) {
      addressEditError = message(value);
    } finally {
      addressEditBusy = false;
    }
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
  {:else if view === 'grants'}
    <div
      class="backdrop"
      role="presentation"
      onclick={(event) => {
        if (event.target === event.currentTarget) view = 'list';
      }}
    >
      <div class="dialog grants-dialog" role="dialog" aria-modal="true" aria-label={t($language, '令牌目录')}>
        <header>
          <button type="button" class="back" onclick={() => (view = 'list')}>
            ← {t($language, '返回')}
          </button>
          <h2>{t($language, '令牌目录')}</h2>
          <button type="button" class="icon" aria-label={t($language, '关闭')} onclick={close}>
            <Icon name="close" size={16} />
          </button>
        </header>
        <div class="grants-address">
          <p class="hint" title={grantsMount ? addressTitle(grantsMount) : ''}>
            {grantsMount ? activeAddress(grantsMount) : ''}
          </p>
          {#if grantsMount}
            <button
              type="button"
              class="pencil"
              aria-label={t($language, '编辑地址')}
              title={t($language, '编辑地址')}
              onclick={() => {
                if (grantsMount) openAddressEditor(grantsMount);
              }}
            >
              <Icon name="pencil" size={14} />
            </button>
          {/if}
        </div>
        {#if grantsMount && addressEditId === grantsMount.id}
          <div class="address-editor">
            <label for={`addresses-grants-${grantsMount.id}`}>{t($language, '地址列表')}</label>
            <textarea
              id={`addresses-grants-${grantsMount.id}`}
              bind:value={addressEditText}
              rows="3"
              placeholder="https://nas.example.com"
            ></textarea>
            <p class="hint">{t($language, '每行一个地址，第一行为优先；内网建议放第一行。')}</p>
            {#if addressEditError}
              <p class="error">{addressEditError}</p>
            {/if}
            <div class="address-actions">
              <button
                type="button"
                class="ghost"
                disabled={addressEditBusy}
                onclick={() => (addressEditId = null)}>{t($language, '取消')}</button
              >
              <button
                type="button"
                disabled={addressEditBusy}
                onclick={() => grantsMount && void saveAddresses(grantsMount)}>{t(
                  $language,
                  '保存地址',
                )}</button
              >
            </div>
          </div>
        {/if}
        {#if grantsError}
          <p class="error">{grantsError}</p>
        {/if}
        {#if grantsBusy}
          <p class="hint">{t($language, '加载中…')}</p>
        {:else if availableGrants.length === 0}
          <p class="hint">{t($language, '这个令牌没有可用的目录。')}</p>
        {:else}
          <div class="list">
            {#each availableGrants as grant (grant.id)}
              <div class="grant-row">
                <div class="grant-info">
                  <strong>{grant.label}</strong>
                  <span class="badge" class:rw={grant.mode === 'rw'}>
                    {grant.mode === 'rw' ? t($language, '读写') : t($language, '只读')}
                  </span>
                  {#if grant.current}
                    <span class="badge current">{t($language, '当前')}</span>
                  {:else if grant.mounted}
                    <span class="badge">{t($language, '已挂载')}</span>
                  {/if}
                </div>
                {#if !grant.mounted}
                  <button
                    type="button"
                    class="primary"
                    disabled={grantsBusy}
                    onclick={() => void attachGrant(grant.id)}
                  >
                    {t($language, '挂载')}
                  </button>
                {/if}
              </div>
            {/each}
          </div>
        {/if}
        <footer>
          <button type="button" onclick={() => (view = 'list')}>
            {t($language, '返回')}
          </button>
        </footer>
      </div>
    </div>
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
                <div class="mount-info">
                  <div class="title-line">
                    <strong>{item.alias}</strong>
                    <button
                      type="button"
                      class="url"
                      title={addressTitle(item)}
                      onclick={() => void openMountGrants(item)}
                    >{activeAddress(item)}{#if addressCount(item) > 1}<span class="address-count"
                          >+{addressCount(item) - 1}</span
                        >{/if}</button
                    >
                    <button
                      type="button"
                      class="pencil"
                      aria-label={t($language, '编辑地址')}
                      title={t($language, '编辑地址')}
                      onclick={() => openAddressEditor(item)}
                    >
                      <Icon name="pencil" size={14} />
                    </button>
                  </div>
                  <div class="detail-line">
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
                    <span class="latency"
                      >{t($language, '缩略图缓存')} {cacheLabel(item.thumb_cache_bytes)}</span
                    >
                    <div class="row-actions">
                      <button
                        type="button"
                        class="menu-trigger"
                        aria-label={t($language, '更多操作')}
                        title={t($language, '更多操作')}
                        onclick={(event) => openRowMenu(event, item)}>⋮</button
                      >
                    </div>
                  </div>
                </div>
              </div>
              {#if addressEditId === item.id}
                <div class="address-editor">
                  <label for={`addresses-${item.id}`}>{t($language, '地址列表')}</label>
                  <textarea
                    id={`addresses-${item.id}`}
                    bind:value={addressEditText}
                    rows="3"
                    placeholder="https://nas.example.com"
                  ></textarea>
                  <p class="hint">{t($language, '每行一个地址，第一行为优先；内网建议放第一行。')}</p>
                  {#if addressEditError}
                    <p class="error">{addressEditError}</p>
                  {/if}
                  <div class="address-actions">
                    <button
                      type="button"
                      class="ghost"
                      disabled={addressEditBusy}
                      onclick={() => (addressEditId = null)}>{t($language, '取消')}</button
                    >
                    <button
                      type="button"
                      disabled={addressEditBusy}
                      onclick={() => void saveAddresses(item)}>{t($language, '保存地址')}</button
                    >
                  </div>
                </div>
              {/if}
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
      {#if menuMount && menuPosition}
        <div
          class="row-menu fixed"
          role="menu"
          tabindex="-1"
          style={`left:${menuPosition.left}px;top:${menuPosition.top}px;max-height:${menuPosition.maxHeight}px`}
        >
          <button
            type="button"
            role="menuitem"
            disabled={busy}
            onclick={() => {
              const mount = menuMount;
              menuId = null;
              menuPosition = null;
              if (mount) void test(mount.id);
            }}>{t($language, '测试连接')}</button
          >
          <button
            type="button"
            role="menuitem"
            disabled={busy}
            onclick={() => {
              const mount = menuMount;
              menuId = null;
              menuPosition = null;
              if (mount) {
                renameId = mount.id;
                renameAlias = mount.alias;
                reconnectId = null;
                confirmRemove = null;
              }
            }}>{t($language, '改名')}</button
          >
          <button
            type="button"
            role="menuitem"
            disabled={busy}
            onclick={() => {
              const mount = menuMount;
              menuId = null;
              menuPosition = null;
              if (mount) {
                reconnectId = mount.id;
                reconnectToken = '';
                renameId = null;
                confirmRemove = null;
              }
            }}>{t($language, '重新连接')}</button
          >
          <button
            type="button"
            role="menuitem"
            class="danger"
            disabled={busy}
            onclick={() => {
              const mount = menuMount;
              menuId = null;
              menuPosition = null;
              if (mount) void remove(mount.id);
            }}>{confirmRemove === menuMount?.id
              ? t($language, '确认断开')
              : t($language, '断开挂载')}</button
          >
        </div>
      {/if}
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

  .mount-info {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }

  .title-line,
  .detail-line {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .detail-line em {
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .detail-line em.rw {
    color: var(--success, #2e7d32);
  }

  .title-line strong {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title-line .url {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    max-width: 360px;
    padding: 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: 12px;
    text-decoration: underline;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
  }

  .address-count {
    margin-left: 4px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 10.5px;
    text-decoration: none;
  }

  .pencil,
  .menu-trigger {
    display: inline-grid;
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    place-items: center;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }

  .pencil:hover,
  .menu-trigger:hover {
    background: var(--hover);
    color: var(--text);
  }

  .row-actions {
    position: relative;
    margin-left: auto;
  }

  .row-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 5;
    display: flex;
    min-width: 148px;
    flex-direction: column;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface);
    box-shadow: var(--shadow);
  }

  .row-menu.fixed {
    position: fixed;
    right: auto;
    z-index: 90;
    overflow-y: auto;
  }

  .row-menu button {
    min-height: 36px;
    padding: 0 10px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text);
    font-size: 13px;
    text-align: left;
  }

  .row-menu button:hover {
    background: var(--hover);
  }

  .row-menu button.danger {
    color: var(--danger);
  }

  .address-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }

  .address-editor label {
    color: var(--text-2);
    font-size: 12px;
  }

  .address-editor textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 12.5px;
    resize: vertical;
  }

  .address-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .grants-address {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .grants-address .hint {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  .grants-dialog {
    width: min(560px, calc(100vw - 32px));
  }

  .grant-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .grant-info {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }

  .badge {
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-2);
    font-size: 11px;
  }

  .badge.rw {
    color: var(--success, #2e7d32);
  }

  .badge.current {
    border-color: var(--accent);
    color: var(--accent-text);
  }

  .back {
    border: none;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
    font-size: 12.5px;
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
      height: 100dvh;
      max-height: none;
      border-radius: 0;
      padding-bottom: max(20px, env(safe-area-inset-bottom, 20px));
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
