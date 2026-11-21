<script lang="ts">
  import { onMount } from 'svelte';
  import { api, ApiError, type RemoteGrant } from '../api';
  import { language, t } from '../i18n';
  import Icon from './Icon.svelte';

  let {
    open,
    csrf,
    onClose,
    onMounted,
  }: {
    open: boolean;
    csrf: string;
    onClose: () => void;
    onMounted: (rootId: number) => void;
  } = $props();

  let step = $state(1);
  let url = $state('');
  let token = $state('');
  let probe = $state<{
    server_version: string;
    resolved_ips: string[];
    address_class: string;
    plaintext: boolean;
  } | null>(null);
  let grants = $state<RemoteGrant[]>([]);
  let selected = $state<number | null>(null);
  let alias = $state('');
  let busy = $state(false);
  let error = $state('');
  let showToken = $state(false);
  let existingAliases = $state<string[]>([]);
  const selectedGrant = $derived(grants.find((grant) => grant.id === selected) ?? null);

  onMount(() => {
    void api
      .remoteMounts()
      .then((result) => {
        existingAliases = result.mounts.map((mount) => mount.alias);
      })
      .catch(() => {
        existingAliases = [];
      });
  });

  function uniqueAlias(base: string): string {
    if (!existingAliases.includes(base)) return base;
    let index = 2;
    while (existingAliases.includes(`${base} (${index})`)) index += 1;
    return `${base} (${index})`;
  }

  function message(value: unknown): string {
    if (value instanceof ApiError) return value.message;
    if (value instanceof Error) return value.message;
    return String(value);
  }

  function reset(): void {
    step = 1;
    url = '';
    token = '';
    probe = null;
    grants = [];
    selected = null;
    alias = '';
    error = '';
    busy = false;
    showToken = false;
  }

  async function probeAddress(jump: boolean): Promise<void> {
    busy = true;
    error = '';
    probe = null;
    try {
      const result = await api.remoteProbe(url.trim(), csrf);
      probe = {
        server_version: result.server_version,
        resolved_ips: result.resolved_ips,
        address_class: result.address_class,
        plaintext: result.plaintext,
      };
      if (jump) step = 2;
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  async function nextStep(): Promise<void> {
    if (!probe) {
      await probeAddress(false);
      if (!probe) return;
    }
    await checkToken();
  }

  async function checkToken(): Promise<void> {
    busy = true;
    error = '';
    grants = [];
    selected = null;
    try {
      const result = await api.remotePreview(url.trim(), token.trim(), csrf);
      grants = result.grants;
      const first = grants[0];
      if (grants.length === 1 && first) {
        selected = first.id;
        alias = uniqueAlias(first.label);
      }
      step = 3;
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
      const created = await api.remoteCreateMount(
        url.trim(),
        token.trim(),
        selected,
        alias.trim(),
        csrf,
      );
      onMounted(created.root_id);
      reset();
      onClose();
    } catch (value) {
      error = message(value);
    } finally {
      busy = false;
    }
  }

  function parseInvite(value: string): boolean {
    const urlMatch = value.match(/https?:\/\/[^\s]+/);
    const tokenMatch = value.match(/fhfed_[A-Za-z0-9_-]+/);
    if (!urlMatch || !tokenMatch) return false;
    url = urlMatch[0].replace(/[),.;]+$/, '');
    token = tokenMatch[0];
    return true;
  }

  function close(): void {
    reset();
    onClose();
  }
</script>

{#if open}
  <div class="backdrop">
    <div class="wizard" role="dialog" aria-modal="true" tabindex="-1" aria-label={t($language, '连接远程目录')}>
      <header>
        <h2>{t($language, '连接远程目录')}</h2>
        <span class="steps">1 — 2 — 3</span>
        <button type="button" class="icon" aria-label={t($language, '关闭')} onclick={close}>
          <Icon name="close" size={16} />
        </button>
      </header>

      {#if error}
        <p class="error">{error}</p>
      {/if}

      {#if step === 1}
        <label class="field">
          <span>{t($language, '地址')}</span>
          <input
            bind:value={url}
            placeholder="https://nas.example.com"
            autocomplete="off"
            oninput={(event) => {
              probe = null;
              const value = event.currentTarget.value;
              if (value.includes('fhfed_')) {
                parseInvite(value);
              }
            }}
            onblur={() => {
              if (!probe && url.trim() && !busy) void probeAddress(false);
            }}
          />
        </label>
        {#if probe}
          <div class="probe">
            <strong>FileHub {probe.server_version}</strong>
            <span>{probe.resolved_ips.join(', ')} · {probe.address_class}</span>
            {#if probe.plaintext}
              <em>{t($language, '明文 HTTP，同网段可能窃取令牌')}</em>
            {/if}
          </div>
        {/if}
        <label class="field">
          <span>{t($language, '令牌')}</span>
          <div class="secret">
            <input
              bind:value={token}
              type={showToken ? 'text' : 'password'}
              placeholder="fhfed_..."
              autocomplete="off"
              oninput={(event) => {
                const value = event.currentTarget.value;
                if (value.includes('https://') || value.includes('http://')) {
                  parseInvite(value);
                }
              }}
            />
            <button type="button" class="ghost" onclick={() => (showToken = !showToken)}>
              {showToken ? t($language, '隐藏') : t($language, '显示')}
            </button>
          </div>
        </label>
        <p class="hint">{t($language, '先校验地址，未保存的令牌不会发送。')}</p>
        <p class="hint">{t($language, '也可以直接粘贴对方给的邀请信息。')}</p>
        <footer>
          <button
            type="button"
            disabled={busy || !url.trim() || !token.trim()}
            onclick={() => void nextStep()}
          >
            {t($language, '下一步')}
          </button>
        </footer>
      {:else}
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
          {#if grants.length === 0}
            <p class="hint">{t($language, '这个令牌没有可用的目录。')}</p>
          {/if}
        </div>
        {#if selectedGrant?.mode === 'rw'}
          <p class="hint warn">{t($language, '对方可以修改、删除和覆盖文件。')}</p>
        {/if}
        <label class="field">
          <span>{t($language, '挂载名称')}</span>
          <input bind:value={alias} />
        </label>
        <footer>
          <button type="button" class="ghost" onclick={() => (step = 1)}>
            {t($language, '上一步')}
          </button>
          <button
            type="button"
            disabled={busy || selected === null || !alias.trim()}
            onclick={() => void mount()}
          >
            {t($language, '挂载')}
          </button>
        </footer>
      {/if}
    </div>
  </div>
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

  .wizard {
    display: flex;
    flex-direction: column;
    gap: 14px;
    width: min(520px, calc(100vw - 32px));
    padding: 20px;
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

  .steps {
    margin-left: auto;
    color: var(--text-2);
    font-size: 12px;
    letter-spacing: 2px;
  }

  .icon {
    border: none;
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text-2);
  }

  input {
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
  }

  .hint {
    margin: 0;
    color: var(--text-2);
    font-size: 12px;
  }

  .secret {
    display: flex;
    gap: 8px;
  }

  .secret input {
    flex: 1;
  }

  .secret button {
    flex: 0 0 auto;
  }

  .hint.warn {
    color: var(--danger);
  }

  .error {
    margin: 0;
    color: var(--danger);
    font-size: 12.5px;
  }

  .probe {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--surface-2);
    font-size: 12.5px;
  }

  .probe span {
    color: var(--text-2);
  }

  .probe em {
    color: var(--danger);
    font-style: normal;
  }

  .grants {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 240px;
    overflow: auto;
  }

  .grant {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
  }

  .grant em {
    font-style: normal;
    font-size: 11.5px;
    color: var(--text-2);
  }

  .grant em.rw {
    color: var(--danger);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  button {
    padding: 8px 14px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--accent);
    color: #fff;
    cursor: pointer;
  }

  button.ghost {
    background: var(--surface-2);
    color: var(--text);
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (max-width: 640px) {
    .wizard {
      width: 100vw;
      height: 100vh;
      max-height: none;
      border-radius: 0;
    }
  }
</style>
