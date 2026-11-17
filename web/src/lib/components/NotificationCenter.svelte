<script lang="ts">
  import Icon from './Icon.svelte';
  import { language, localeOf, t } from '../i18n';
  import { clearNotifications, markAllRead, notifications, unreadCount } from '../notifications';

  let open = $state(false);
  let root = $state<HTMLDivElement | null>(null);

  function toggle(): void {
    open = !open;
  }

  function closeOutside(event: MouseEvent): void {
    if (!open) return;
    const target = event.target;
    if (target instanceof Node && root?.contains(target)) return;
    open = false;
  }

  function onKeyDown(event: KeyboardEvent): void {
    if (event.key === 'Escape') open = false;
  }

  function timeLabel(at: number, lang: 'zh-CN' | 'en'): string {
    try {
      return new Date(at).toLocaleTimeString(localeOf(lang), {
        hour: '2-digit',
        minute: '2-digit',
      });
    } catch {
      return '';
    }
  }
</script>

<div class="notification-center" bind:this={root}>
  <button
    type="button"
    class="icon-button notification-button"
    aria-label={t($language, '通知')}
    title={t($language, '通知')}
    aria-expanded={open}
    onclick={toggle}
  >
    <Icon name="bell" size={19} />
    {#if $unreadCount > 0}
      <span class="notification-badge">{$unreadCount > 9 ? '9+' : $unreadCount}</span>
    {/if}
  </button>
  {#if open}
    <div class="notification-popover" role="dialog" aria-label={t($language, '通知')}>
      <header class="notification-head">
        <strong>{t($language, '通知')}</strong>
        {#if $unreadCount > 0}
          <button type="button" class="notification-action" onclick={markAllRead}>
            {t($language, '全部已读')}
          </button>
        {/if}
      </header>
      {#if $notifications.length === 0}
        <p class="notification-empty">{t($language, '暂无通知')}</p>
      {:else}
        <ul class="notification-list">
          {#each $notifications as item (item.id)}
            <li class:unread={!item.read}>
              <span class="notification-dot" aria-hidden="true"></span>
              <div class="notification-body">
                <span class="notification-message">{item.message}</span>
                <span class="notification-meta">{timeLabel(item.at, $language)}</span>
              </div>
            </li>
          {/each}
        </ul>
        <footer class="notification-foot">
          <button type="button" class="notification-action" onclick={clearNotifications}>
            {t($language, '清空通知')}
          </button>
        </footer>
      {/if}
    </div>
  {/if}
</div>

<svelte:window onclick={closeOutside} onkeydown={onKeyDown} />

<style>
  .notification-center {
    position: relative;
  }

  .notification-button {
    position: relative;
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .notification-button:hover {
    background: var(--hover);
  }

  .notification-badge {
    position: absolute;
    top: -4px;
    right: -4px;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: var(--radius-pill);
    background: var(--danger);
    color: var(--on-accent);
    font-size: 10px;
    font-weight: 700;
    line-height: 16px;
    text-align: center;
  }

  .notification-popover {
    position: absolute;
    top: calc(100% + 10px);
    right: 0;
    z-index: 40;
    width: min(340px, calc(100vw - 32px));
    max-height: 420px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .notification-head,
  .notification-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 14px;
  }

  .notification-head {
    border-bottom: 1px solid var(--border);
  }

  .notification-head strong {
    font-size: 14px;
  }

  .notification-action {
    padding: 2px 4px;
    border: 0;
    background: none;
    color: var(--accent-text);
    font-size: 12px;
    cursor: pointer;
  }

  .notification-action:hover {
    text-decoration: underline;
  }

  .notification-empty {
    margin: 0;
    padding: 28px 14px;
    color: var(--text-2);
    font-size: 13px;
    text-align: center;
  }

  .notification-list {
    max-height: 320px;
    margin: 0;
    padding: 4px 0;
    overflow-y: auto;
    list-style: none;
  }

  .notification-list li {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 14px;
    border-bottom: 1px solid var(--border);
  }

  .notification-list li:last-child {
    border-bottom: 0;
  }

  .notification-dot {
    flex: none;
    width: 6px;
    height: 6px;
    margin-top: 7px;
    border-radius: 50%;
    background: var(--border);
  }

  .notification-list li.unread .notification-dot {
    background: var(--accent);
  }

  .notification-body {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 3px;
  }

  .notification-message {
    overflow-wrap: anywhere;
    color: var(--text);
    font-size: 13px;
    line-height: 1.45;
  }

  .notification-list li.unread .notification-message {
    font-weight: 600;
  }

  .notification-meta {
    color: var(--text-2);
    font-size: 11px;
  }

  .notification-foot {
    justify-content: flex-end;
    border-top: 1px solid var(--border);
  }
</style>
