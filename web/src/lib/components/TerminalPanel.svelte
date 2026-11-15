<script lang="ts">
  import { onMount } from 'svelte';
  import { api } from '../api';
  import Icon from './Icon.svelte';
  import { overlayGuard } from '../overlayGuard';

  let {
    root,
    cwd,
    csrf,
    onClose,
    onRefresh,
  }: {
    root: number;
    cwd: string;
    csrf: string;
    onClose: () => void;
    onRefresh: () => void;
  } = $props();

  type Line = { kind: 'system' | 'input' | 'output' | 'error' | 'prompt'; text: string };

  let workdir = $state('');
  let input = $state('');
  let lines = $state<Line[]>([]);
  let busy = $state(false);
  let pendingDelete = $state<string | null>(null);
  let inputElement = $state<HTMLInputElement | null>(null);
  let socket = $state<WebSocket | null>(null);
  let connected = $state(false);
  let offsetX = $state(0);
  let offsetY = $state(0);
  let dragging = false;
  let dragStart = { x: 0, y: 0 };

  function formatBytes(value: number | null | undefined): string {
    if (value === null || value === undefined) return '不限';
    if (value < 1024) return `${value} B`;
    if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
    if (value < 1024 * 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MB`;
    return `${(value / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  async function loadInfo(): Promise<void> {
    try {
      const [usage, listing] = await Promise.all([
        api.usage(),
        api.list(root, cwd, 'name', 'asc', false),
      ]);
      const directories = listing.entries.filter((entry) => entry.is_dir).length;
      const files = listing.entries.length - directories;
      const diskTotal = usage.disks.reduce((sum, disk) => sum + disk.total, 0);
      const diskUsed = usage.disks.reduce((sum, disk) => sum + disk.used, 0);
      lines = [
        { kind: 'system', text: `当前目录：/${workdir}` },
        {
          kind: 'system',
          text: `用户配额：${formatBytes(usage.used_bytes)} / ${formatBytes(usage.quota_bytes)}`,
        },
        { kind: 'system', text: `物理磁盘：${formatBytes(diskUsed)} / ${formatBytes(diskTotal)}` },
        { kind: 'system', text: `本目录：${directories} 个目录，${files} 个文件` },
      ];
    } catch (error) {
      lines = [
        { kind: 'error', text: error instanceof Error ? error.message : '无法读取终端信息' },
      ];
    }
  }

  type ServerMessage =
    | { type: 'ready'; cwd: string }
    | { type: 'result'; output: string; cwd: string }
    | { type: 'error'; message: string };

  function append(kind: Line['kind'], text: string): void {
    lines = [...lines, { kind, text }];
  }

  function send(message: unknown): boolean {
    if (socket?.readyState !== WebSocket.OPEN) {
      append('error', '终端连接尚未建立。');
      return false;
    }
    socket.send(JSON.stringify(message));
    return true;
  }

  function connect(): void {
    socket?.close();
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    socket = new WebSocket(`${protocol}//${window.location.host}/api/v1/terminal/ws`);
    socket.addEventListener('open', () => {
      send({ type: 'init', root, cwd: workdir, csrf });
    });
    socket.addEventListener('message', (event) => {
      let message: ServerMessage;
      try {
        message = JSON.parse(String(event.data)) as ServerMessage;
      } catch {
        append('error', '终端返回了无法解析的数据。');
        return;
      }
      if (message.type === 'ready') {
        connected = true;
        workdir = message.cwd;
        append('system', '已建立 WebSocket 终端连接。');
        focusInput();
      } else if (message.type === 'result') {
        if (message.output) append('output', message.output);
        workdir = message.cwd;
        busy = false;
        onRefresh();
        focusInput();
      } else {
        append('error', message.message);
        busy = false;
        focusInput();
      }
    });
    socket.addEventListener('close', () => {
      connected = false;
      busy = false;
      append('system', '终端连接已断开。');
    });
    socket.addEventListener('error', () => {
      connected = false;
    });
  }

  onMount(() => {
    workdir = cwd;
    void loadInfo();
    connect();
    requestAnimationFrame(() => inputElement?.focus());
    return () => socket?.close();
  });

  function focusInput(): void {
    requestAnimationFrame(() => inputElement?.focus());
  }

  function startDrag(event: PointerEvent): void {
    if ((event.target as HTMLElement).closest('button')) return;
    dragging = true;
    dragStart = { x: event.clientX - offsetX, y: event.clientY - offsetY };
  }

  function drag(event: PointerEvent): void {
    if (!dragging) return;
    offsetX = event.clientX - dragStart.x;
    offsetY = event.clientY - dragStart.y;
  }

  function execute(line: string, confirm: boolean): void {
    busy = true;
    if (!send({ type: 'exec', line, confirm })) busy = false;
  }

  async function run(): Promise<void> {
    const line = input.trim();
    if (!line || busy) return;
    input = '';
    lines = [...lines, { kind: 'input', text: `$ ${line}` }];

    if (line === 'exit' || line === 'quit') {
      onClose();
      return;
    }

    if (pendingDelete) {
      const command = pendingDelete;
      pendingDelete = null;
      if (line.toLowerCase() === 'y') {
        execute(command, true);
      } else {
        lines = [...lines, { kind: 'system', text: '已取消删除。' }];
        focusInput();
      }
      return;
    }

    if (/^rm(?:\s|$)/.test(line) && !/\s-y(?:\s|$)/.test(line)) {
      pendingDelete = line;
      lines = [...lines, { kind: 'prompt', text: '确认删除？输入 y 确认，其他内容取消。' }];
      focusInput();
      return;
    }

    execute(line, false);
  }
</script>

<svelte:window
  onkeydown={(event) => {
    if (event.key === 'Escape') onClose();
  }}
  onpointermove={drag}
  onpointerup={() => (dragging = false)}
/>

<div class="terminal-overlay" role="presentation" use:overlayGuard={onClose}>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <section
    class="terminal-window"
    style={`transform:translate(${offsetX}px, ${offsetY}px)`}
    aria-label="终端"
    onclick={(event) => {
      event.stopPropagation();
      inputElement?.focus();
    }}
  >
    <header class="titlebar" role="presentation" onpointerdown={startDrag}>
      <div class="tab">
        <span class="tab-dot"></span>
        <span>FileHub Terminal</span>
      </div>
      <button type="button" class="close" aria-label="关闭终端" onclick={onClose}>
        <Icon name="close" size={16} />
      </button>
    </header>
    <div class="terminal-body">
      <div class="scrollback">
        {#each lines as line, index (index)}
          <pre class={line.kind}>{line.text}</pre>
        {/each}
      </div>
      <form
        class="prompt-line"
        onsubmit={(event) => {
          event.preventDefault();
          void run();
        }}
      >
        <span class="prompt">PS /{workdir}&gt;</span>
        <input
          bind:this={inputElement}
          bind:value={input}
          disabled={busy || !connected}
          spellcheck="false"
        />
        {#if !connected}
          <button type="button" class="reconnect" onclick={connect}>重连</button>
        {/if}
      </form>
    </div>
    <footer class="statusbar">
      <span>/{workdir}</span>
      <span>{busy ? 'running' : 'ready'}</span>
    </footer>
  </section>
</div>

<style>
  .terminal-overlay {
    position: fixed;
    inset: 0;
    z-index: 75;
    display: grid;
    place-items: center;
    padding: 24px;
    background: transparent;
  }

  .terminal-window {
    display: flex;
    width: min(1040px, 94vw);
    height: min(720px, 88vh);
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--terminal-border);
    border-radius: 10px;
    background: var(--terminal-bg);
    color: var(--terminal-fg);
    box-shadow: 0 24px 80px rgb(0 0 0 / 45%);
    font-family: var(--font-mono);
  }

  .titlebar {
    display: flex;
    height: 38px;
    flex: 0 0 38px;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px 0 10px;
    background: var(--terminal-titlebar);
    cursor: move;
    touch-action: none;
    user-select: none;
  }

  .tab {
    display: flex;
    height: 30px;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    border-radius: 6px 6px 0 0;
    background: var(--terminal-bg);
    color: var(--terminal-fg);
    font-size: 12.5px;
  }

  .tab-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
  }

  .close {
    display: grid;
    width: 30px;
    height: 30px;
    place-items: center;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--terminal-muted);
  }

  .close:hover {
    background: var(--terminal-hover);
    color: var(--terminal-fg);
  }

  .terminal-body {
    display: flex;
    min-height: 0;
    flex: 1;
    flex-direction: column;
    padding: 14px 16px 8px;
    overflow: auto;
  }

  .scrollback {
    flex: 0 0 auto;
  }

  pre {
    margin: 0 0 3px;
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-mono);
    font-size: 13.5px;
    line-height: 1.55;
  }

  pre.input {
    color: var(--accent);
  }

  pre.output {
    color: var(--terminal-fg);
  }

  pre.error {
    color: var(--danger);
  }

  pre.system,
  pre.prompt {
    color: var(--terminal-muted);
  }

  pre.prompt {
    color: var(--file-slide);
  }

  .reconnect {
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--terminal-border);
    border-radius: 5px;
    background: transparent;
    color: var(--terminal-muted);
    font-size: 12px;
  }

  .prompt-line {
    display: flex;
    flex: 0 0 auto;
    align-items: baseline;
    gap: 8px;
    padding-top: 2px;
  }

  .prompt {
    flex: 0 0 auto;
    color: var(--accent);
    font-size: 13.5px;
    white-space: nowrap;
  }

  .prompt-line input {
    min-width: 0;
    flex: 1;
    padding: 0;
    border: 0;
    outline: 0;
    background: transparent;
    color: var(--terminal-fg);
    font-family: var(--font-mono);
    font-size: 13.5px;
  }

  .statusbar {
    display: flex;
    height: 26px;
    flex: 0 0 26px;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    background: var(--terminal-titlebar);
    color: var(--terminal-muted);
    font-size: 11.5px;
  }
</style>
