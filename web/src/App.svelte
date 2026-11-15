<script lang="ts">
  /* global __APP_VERSION__ */
  import DOMPurify from 'dompurify';
  import { marked } from 'marked';
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import AccountDialog from './lib/components/AccountDialog.svelte';
  import NoticeDialog from './lib/components/NoticeDialog.svelte';
  import ProgressPanel from './lib/components/ProgressPanel.svelte';
  import ToastPanel from './lib/components/ToastPanel.svelte';
  import AdminPage from './lib/components/AdminPage.svelte';
  import AuditPage from './lib/components/AuditPage.svelte';
  import SharesPage from './lib/components/SharesPage.svelte';
  import ConfirmDialog from './lib/components/ConfirmDialog.svelte';
  import FileTypeIcon from './lib/components/FileTypeIcon.svelte';
  import NameDialog from './lib/components/NameDialog.svelte';
  import PreviewModal from './lib/components/PreviewModal.svelte';
  import PreferencesPage from './lib/components/PreferencesPage.svelte';
  import ShareDialog from './lib/components/ShareDialog.svelte';
  import TerminalPanel from './lib/components/TerminalPanel.svelte';
  import TransferDialog from './lib/components/TransferDialog.svelte';
  import AuthScreen from './lib/components/AuthScreen.svelte';
  import SharePage from './lib/components/SharePage.svelte';
  import Icon from './lib/components/Icon.svelte';
  import { initLanguage, language, localeOf, rootAlias, setLanguage, t } from './lib/i18n';
  import { showNotice, showNoticeAction } from './lib/notices';
  import { failProgress, finishProgress, startProgress, updateProgress } from './lib/progress';
  import {
    api,
    ApiError,
    type AuthResponse,
    type Entry,
    type Root,
    type SystemFeatures,
    type UserPreferences,
    type SearchEntry,
    type Usage,
    type User,
  } from './lib/api';
  import { applyTheme, resolveTheme, type Theme } from './lib/theme';

  let theme = $state<Theme>('light');
  let view = $state<'list' | 'thumb' | 'tile'>('list');
  let sortField = $state<'name' | 'size' | 'modified'>('name');
  let sortOrder = $state<'asc' | 'desc'>('asc');
  let sidebarOpen = $state(false);
  let accountOpen = $state(false);
  let loading = $state(true);
  let initialized = $state(false);
  let user = $state<User | null>(null);
  let preferences = $state<UserPreferences | null>(null);
  let systemFeatures = $state<SystemFeatures>({ thumbnails: true, editor: true, terminal: true });
  let csrf = $state('');
  let loggingOut = $state(false);
  let roots = $state<Root[]>([]);
  let activeRootId = $state<number | null>(null);
  let currentPath = $state('');
  let entries = $state<Entry[]>([]);
  let selected = $state<string[]>([]);
  let selectionAnchor = $state<string | null>(null);
  let filesLoading = $state(false);
  let filesLoadingMore = $state(false);
  let nextCursor = $state<string | null>(null);
  let filesError = $state('');
  let uploading = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);
  let folderInput = $state<HTMLInputElement | null>(null);
  const shareToken = (() => {
    const match = window.location.pathname.match(/^\/s\/([^/]+)/);
    const token = match?.[1];
    return token ? decodeURIComponent(token) : null;
  })();
  let searchQuery = $state('');
  let searchResults = $state<SearchEntry[]>([]);
  let searchActive = $state(false);
  let searchIndexing = $state(false);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let searchInput = $state<HTMLInputElement | null>(null);
  const indexedRoots = new SvelteSet<number>();
  let searchToken = 0;
  let editorOpen = $state(false);
  let editorName = $state('');
  let editorPath = $state('');
  let editorContent = $state('');
  let editorOriginalContent = $state('');
  let editorEtag = $state('');
  let editorError = $state('');
  let editorSaving = $state(false);
  let editorPreview = $state(false);
  let lineNumbersElement = $state<HTMLPreElement | null>(null);
  let usage = $state<Usage | null>(null);
  let marquee = $state<{ x1: number; y1: number; x2: number; y2: number } | null>(null);
  let marqueePreview = $state<string[]>([]);
  let suppressSelectionClear = false;
  let appView = $state<'files' | 'settings' | 'admin' | 'audit' | 'shares'>('files');
  let contextMenu = $state<{ x: number; y: number; entry: Entry } | null>(null);
  let blankMenu = $state<{ x: number; y: number } | null>(null);
  let terminalOpen = $state(false);
  let transfer = $state<{ mode: 'copy' | 'move'; entries: Entry[] } | null>(null);
  let dragPayload = $state<{ rootId: number; path: string; name: string } | null>(null);
  let nameDialog = $state<{
    mode: 'mkdir' | 'touch' | 'rename';
    entry: Entry | null;
    title: string;
    label: string;
    initialValue: string;
  } | null>(null);
  let preview = $state<{ items: Entry[]; index: number } | null>(null);
  let shareTarget = $state<Entry | null>(null);
  let confirmState = $state<{
    title: string;
    message: string;
    requireText: string | null;
    action: (value: string) => void;
  } | null>(null);
  let editorDirty = $derived(editorContent !== editorOriginalContent);
  let editorLineNumbers = $derived(
    Array.from({ length: editorContent.split('\n').length }, (_, index) => index + 1),
  );
  let previewHtml = $derived(
    editorPreview ? DOMPurify.sanitize(String(marked.parse(editorContent))) : '',
  );

  let activeRoot = $derived(roots.find((root) => root.id === activeRootId) ?? null);
  let pathSegments = $derived(currentPath.split('/').filter(Boolean));
  let quotaPercent = $derived(
    usage?.quota_bytes ? Math.min(100, (usage.used_bytes / usage.quota_bytes) * 100) : 0,
  );
  let quotaLabel = $derived(
    usage
      ? `${formatSize(usage.used_bytes)} / ${usage.quota_bytes ? formatSize(usage.quota_bytes) : '∞'}`
      : '—',
  );
  let pageTitle = $derived.by(() => {
    if (appView === 'shares') return t($language, 'FileHub - 分享管理');
    if (appView === 'settings') return t($language, 'FileHub - 设置');
    if (appView === 'admin') return t($language, 'FileHub - 系统管理');
    if (appView === 'audit') return t($language, 'FileHub - 审计日志');
    const current =
      pathSegments[pathSegments.length - 1] ||
      (activeRoot ? rootLabel(activeRoot.alias) : undefined);
    return current ? `FileHub - ${current}` : 'FileHub';
  });

  onMount(() => {
    initLanguage();
    try {
      theme = resolveTheme(
        localStorage.getItem('filehub.theme'),
        window.matchMedia('(prefers-color-scheme: dark)').matches,
      );
    } catch (error) {
      console.warn('could not read theme preference', error);
    }

    applyTheme(theme);
    if (!shareToken) {
      void bootstrap();
    } else {
      loading = false;
    }

    const onKeyDown = (event: KeyboardEvent) => {
      const typing = isTypingTarget(event.target);

      if (event.key === '/' && !typing) {
        event.preventDefault();
        searchInput?.focus();
        return;
      }
      if (
        (event.key === '`' || event.key === '~') &&
        (terminalOpen || !typing) &&
        activeRoot &&
        preferences?.terminal_enabled !== false &&
        systemFeatures.terminal
      ) {
        event.preventDefault();
        terminalOpen = !terminalOpen;
        return;
      }
      if (event.key === 'Escape') {
        if (editorOpen) {
          editorOpen = false;
        } else if (searchActive) {
          exitSearch();
        } else if (sidebarOpen) {
          sidebarOpen = false;
        } else {
          clearSelection();
        }
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's' && editorOpen) {
        event.preventDefault();
        void saveEditor();
        return;
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'a' && !typing) {
        event.preventDefault();
        selected = entries.map((entry) => entry.name);
        return;
      }
      if (event.key === 'Delete' && selected.length > 0 && !typing) {
        event.preventDefault();
        void deleteSelected();
      }
    };

    window.addEventListener('keydown', onKeyDown);
    const autoSaveTimer = window.setInterval(() => {
      if (editorOpen && editorDirty && !editorSaving) {
        void saveEditor(true);
      }
    }, 30_000);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      window.clearInterval(autoSaveTimer);
    };
  });

  function setTheme(next: Theme): void {
    theme = next;
    applyTheme(theme);
    try {
      localStorage.setItem('filehub.theme', theme);
    } catch (error) {
      console.warn('could not persist theme preference', error);
    }
  }

  function setThemePreference(next: Theme): void {
    setTheme(next);
    void savePreference({ theme: next });
  }

  function toggleTheme(): void {
    setThemePreference(theme === 'dark' ? 'light' : 'dark');
  }

  async function bootstrap(): Promise<void> {
    loading = true;

    try {
      const status = await api.setupStatus();
      initialized = status.initialized;

      if (!initialized) {
        return;
      }

      try {
        const auth = await api.me();
        user = auth.user;
        csrf = auth.csrf_token;
        await loadPreferences();
        await loadRoots();
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          console.warn('could not restore session', error);
        }
      }
    } catch (error) {
      console.warn('could not read setup status', error);
      initialized = true;
    } finally {
      loading = false;
    }
  }

  function handleAuth(auth: AuthResponse): void {
    user = auth.user;
    csrf = auth.csrf_token;
    initialized = true;
    loading = false;
    void loadPreferences();
    void loadRoots();
  }

  async function loadPreferences(): Promise<void> {
    try {
      const result = await api.getPreferences();
      systemFeatures = result.system;
      applyPreferences(result.preferences, false);
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) {
        console.warn('could not load preferences', error);
      }
    }
  }

  function applyPreferences(next: UserPreferences, reload = true): void {
    const hiddenChanged = preferences?.show_hidden !== next.show_hidden;
    preferences = next;
    setTheme(next.theme);
    setLanguage(next.language);
    if (!next.terminal_enabled) terminalOpen = false;
    if (reload && hiddenChanged && activeRootId !== null) {
      exitSearch();
      void loadList(activeRootId, currentPath);
    }
  }

  async function savePreference(patch: Parameters<typeof api.updatePreferences>[0]): Promise<void> {
    if (!user) return;
    try {
      const result = await api.updatePreferences(patch, csrf);
      systemFeatures = result.system;
      applyPreferences(result.preferences);
    } catch (error) {
      showNotice(error instanceof Error ? error.message : t($language, '偏好保存失败'));
    }
  }

  async function logout(): Promise<void> {
    if (loggingOut) return;

    loggingOut = true;
    try {
      await api.logout(csrf);
    } catch (error) {
      console.warn('logout request failed', error);
    } finally {
      user = null;
      preferences = null;
      csrf = '';
      roots = [];
      entries = [];
      currentPath = '';
      selected = [];
      activeRootId = null;
      loggingOut = false;
    }
  }

  async function loadRoots(): Promise<void> {
    try {
      const result = await api.roots();
      roots = result.roots;
      const next = roots.find((root) => root.id === activeRootId) ?? roots[0] ?? null;
      if (!next) {
        activeRootId = null;
        entries = [];
        filesError = t($language, '当前账号还没有可访问的目录');
        return;
      }
      await loadList(next.id, '');
    } catch (error) {
      handleFilesError(error);
    }
  }

  async function loadList(rootId: number, path: string): Promise<void> {
    filesLoading = true;
    filesError = '';
    nextCursor = null;
    try {
      const savedSort = localStorage.getItem(`sort:${rootId}:${path}`);
      if (savedSort) {
        const [field, order] = savedSort.split(':');
        if (field === 'name' || field === 'size' || field === 'modified') {
          sortField = field;
          sortOrder = order === 'desc' ? 'desc' : 'asc';
        }
      }
      const result = await api.list(rootId, path, sortField, sortOrder, false, undefined, 200);
      activeRootId = result.root.id;
      currentPath = result.path;
      entries = result.entries;
      nextCursor = result.next_cursor;
      selected = [];
      selectionAnchor = null;
      const remembered = localStorage.getItem(`view:${result.root.id}:${result.path}`);
      if (
        remembered === 'list' ||
        remembered === 'thumb' ||
        remembered === 'tile' ||
        remembered === 'grid'
      ) {
        view = remembered === 'grid' ? 'thumb' : remembered;
      }
      void loadUsage();
    } catch (error) {
      handleFilesError(error);
    } finally {
      filesLoading = false;
    }
  }

  async function loadMore(): Promise<void> {
    if (
      appView !== 'files' ||
      searchActive ||
      activeRootId === null ||
      nextCursor === null ||
      filesLoadingMore
    ) {
      return;
    }
    filesLoadingMore = true;
    const rootId = activeRootId;
    const path = currentPath;
    try {
      const result = await api.list(rootId, path, sortField, sortOrder, false, nextCursor, 200);
      if (rootId !== activeRootId || path !== currentPath) return;
      entries = [...entries, ...result.entries];
      nextCursor = result.next_cursor;
    } catch (error) {
      handleFilesError(error);
    } finally {
      filesLoadingMore = false;
    }
  }

  function entrySize(entry: Entry): string {
    return entry.is_dir ? '—' : formatSize(entry.size);
  }

  function handleContentScroll(event: Event): void {
    const element = event.currentTarget as HTMLElement;
    if (element.scrollHeight - element.scrollTop - element.clientHeight < 240) {
      void loadMore();
    }
  }

  function handleFilesError(error: unknown): void {
    if (error instanceof ApiError && error.status === 401) {
      user = null;
      csrf = '';
      return;
    }
    showNotice(error instanceof Error ? error.message : t($language, '文件列表加载失败'));
  }

  async function loadUsage(): Promise<void> {
    try {
      usage = await api.usage();
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) {
        console.warn('could not load usage', error);
      }
    }
  }

  function navigate(path: string): void {
    searchActive = false;
    searchQuery = '';
    searchResults = [];
    if (activeRootId !== null) {
      void loadList(activeRootId, path);
    }
  }

  function handleSearchInput(): void {
    if (searchTimer) {
      clearTimeout(searchTimer);
    }

    const query = searchQuery.trim();
    if (!query || activeRootId === null) {
      searchActive = false;
      searchResults = [];
      return;
    }

    searchTimer = setTimeout(() => {
      void runSearch(query);
    }, 250);
  }

  async function runSearch(query: string): Promise<void> {
    if (roots.length === 0) return;
    const token = ++searchToken;

    const collect = async (forceIndex: boolean): Promise<SearchEntry[]> => {
      const indexing: Promise<unknown>[] = [];
      const settled = await Promise.allSettled(
        roots.map(async (root) => {
          if (forceIndex || !indexedRoots.has(root.id)) {
            indexedRoots.add(root.id);
            indexing.push(
              api.index(root.id, '', true).catch((error) => {
                console.warn('background index failed', error);
              }),
            );
          }
          const result = await api.search(root.id, query, 100);
          return result.entries.map((entry) => ({ ...entry, rootId: root.id }));
        }),
      );
      if (indexing.length > 0) {
        searchIndexing = true;
        void Promise.allSettled(indexing).then(() => {
          searchIndexing = false;
          if (token === searchToken && searchQuery === query) void runSearch(query);
        });
      }
      return settled.flatMap((result) => (result.status === 'fulfilled' ? result.value : []));
    };

    try {
      const results = await collect(false);
      if (token !== searchToken) return;
      searchResults = results.slice(0, 200);
      searchActive = true;
    } catch (error) {
      if (token === searchToken) handleFilesError(error);
    }
  }

  function openSearchResult(entry: SearchEntry): void {
    const root = entry.rootId ?? activeRootId;
    if (root === null) return;

    searchActive = false;
    searchQuery = '';
    searchResults = [];

    activeRootId = root;
    if (entry.is_dir) {
      void loadList(root, entry.path);
    } else {
      const parent = entry.path.includes('/')
        ? entry.path.slice(0, entry.path.lastIndexOf('/'))
        : '';
      void (async () => {
        await loadList(root, parent);
        selected = [entry.name];
        selectionAnchor = entry.name;
      })();
    }
  }

  function exitSearch(): void {
    searchActive = false;
    searchQuery = '';
    searchResults = [];
    searchIndexing = false;
  }

  function navigateToSegment(index: number): void {
    navigate(pathSegments.slice(0, index + 1).join('/'));
  }

  function joinPath(base: string, name: string): string {
    return base ? `${base}/${name}` : name;
  }

  function rootLabel(alias: string): string {
    return rootAlias($language, alias);
  }

  function enterEntry(entry: Entry): void {
    if (entry.is_dir) {
      navigate(joinPath(currentPath, entry.name));
    } else if (isTextFile(entry.name)) {
      if (systemFeatures.editor && preferences?.editor_enabled !== false) void openEditor(entry);
      else showUnsupported(entry);
    } else if (isPdfName(entry.name)) {
      openPdf(entry);
    } else if (isLightboxPreview(entry.name)) {
      openPreview(entry);
    } else {
      showUnsupported(entry);
    }
  }

  function showUnsupported(entry: Entry): void {
    showNoticeAction(
      t($language, '无法在浏览器中预览「{name}」。', { name: entry.name }),
      t($language, '无法预览'),
      t($language, '下载文件'),
      () => downloadEntry(entry),
      'info',
    );
  }

  function isLightboxPreview(name: string): boolean {
    return isImageName(name) || isVideoName(name) || isAudioName(name);
  }

  function isPreviewable(name: string): boolean {
    return isLightboxPreview(name) || isPdfName(name);
  }

  function canPreviewEntry(entry: Entry): boolean {
    return !entry.is_dir && (isTextFile(entry.name) || isPreviewable(entry.name));
  }

  function openPdf(entry: Entry): void {
    if (activeRootId === null) return;
    window.open(
      api.previewUrl(activeRootId, joinPath(currentPath, entry.name)),
      '_blank',
      'noopener,noreferrer',
    );
  }

  function isAudioName(name: string): boolean {
    return /\.(mp3|wav|ogg|m4a|flac|aac)$/i.test(name);
  }

  function isPdfName(name: string): boolean {
    return /\.pdf$/i.test(name);
  }

  function isVideoName(name: string): boolean {
    return /\.(mp4|mov|mkv|webm|avi|m4v)$/i.test(name);
  }

  function handleEntryClick(event: MouseEvent, entry: Entry): void {
    if (suppressSelectionClear) {
      suppressSelectionClear = false;
      return;
    }
    if (event.shiftKey && selectionAnchor) {
      const anchor = entries.findIndex((item) => item.name === selectionAnchor);
      const current = entries.findIndex((item) => item.name === entry.name);
      if (anchor >= 0 && current >= 0) {
        const start = Math.min(anchor, current);
        const end = Math.max(anchor, current);
        selected = entries.slice(start, end + 1).map((item) => item.name);
      }
    } else if (event.metaKey || event.ctrlKey) {
      toggleSelection(entry.name);
      selectionAnchor = entry.name;
    } else if (selected.length === 1 && selected[0] === entry.name) {
      selected = [];
      selectionAnchor = null;
    } else {
      selected = [entry.name];
      selectionAnchor = entry.name;
    }
    if (window.matchMedia('(pointer: coarse)').matches) {
      enterEntry(entry);
    }
  }

  function menuPosition(x: number, y: number): { x: number; y: number } {
    const width = 260;
    const height = 380;
    return {
      x: Math.max(8, Math.min(x, window.innerWidth - width)),
      y: Math.max(8, Math.min(y, window.innerHeight - height)),
    };
  }

  function openContextMenu(event: MouseEvent, entry: Entry): void {
    event.preventDefault();
    event.stopPropagation();
    blankMenu = null;
    if (!isSelected(entry.name)) selected = [entry.name];
    contextMenu = { ...menuPosition(event.clientX, event.clientY), entry };
  }

  function openPreview(entry: Entry): void {
    const items = entries.filter((item) => !item.is_dir && isLightboxPreview(item.name));
    const index = items.findIndex((item) => item.name === entry.name);
    if (index >= 0) {
      preview = { items, index };
    }
  }

  function previewEntry(entry: Entry): void {
    if (isTextFile(entry.name)) {
      if (systemFeatures.editor && preferences?.editor_enabled !== false) void openEditor(entry);
      else showUnsupported(entry);
    } else if (isPdfName(entry.name)) {
      openPdf(entry);
    } else if (isLightboxPreview(entry.name)) {
      openPreview(entry);
    } else {
      showUnsupported(entry);
    }
  }

  function runContextAction(
    action: 'preview' | 'download' | 'share' | 'rename' | 'copy-to' | 'move-to' | 'delete',
  ): void {
    const entry = contextMenu?.entry;
    contextMenu = null;
    if (!entry) return;
    const targets =
      selected.includes(entry.name) && selected.length > 1
        ? entries.filter((item) => selected.includes(item.name))
        : [entry];
    if (action === 'preview') previewEntry(entry);
    if (action === 'download') downloadEntry(entry);
    if (action === 'share') shareTarget = entry;
    if (action === 'rename') {
      nameDialog = {
        mode: 'rename',
        entry,
        title: t($language, '重命名'),
        label: t($language, '新名称'),
        initialValue: entry.name,
      };
    }
    if (action === 'copy-to') transfer = { mode: 'copy', entries: targets };
    if (action === 'move-to') transfer = { mode: 'move', entries: targets };
    if (action === 'delete') void requestDelete(targets);
  }

  function openMkdir(): void {
    nameDialog = {
      mode: 'mkdir',
      entry: null,
      title: t($language, '新建目录'),
      label: t($language, '目录名称'),
      initialValue: '',
    };
  }

  async function submitName(value: string): Promise<void> {
    const dialog = nameDialog;
    nameDialog = null;
    if (!dialog || activeRootId === null) return;
    const rootId = activeRootId;
    try {
      if (dialog.mode === 'mkdir') {
        await api.mkdir(rootId, currentPath, value, csrf);
      } else if (dialog.mode === 'touch') {
        await api.touch(rootId, currentPath, value, csrf);
      } else if (dialog.entry) {
        await api.rename(rootId, joinPath(currentPath, dialog.entry.name), value, csrf);
      }
      await loadList(rootId, currentPath);
    } catch (error) {
      handleFilesError(error);
    }
  }

  function startDrag(event: DragEvent, entry: Entry): void {
    if (activeRootId === null) return;
    const path = joinPath(currentPath, entry.name);
    dragPayload = { rootId: activeRootId, path, name: entry.name };
    if (event.dataTransfer) {
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('text/plain', path);
    }
  }

  function allowDrop(event: DragEvent): void {
    event.preventDefault();
    event.stopPropagation();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'move';
  }

  async function dropInto(event: DragEvent, destRoot: number, destPath: string): Promise<void> {
    event.preventDefault();
    event.stopPropagation();
    const payload = dragPayload;
    dragPayload = null;
    if (!payload) return;
    if (payload.rootId === destRoot && payload.path === destPath) return;
    if (payload.rootId === destRoot && destPath.startsWith(`${payload.path}/`)) {
      showNotice(t($language, '不能把目录移动到它自己里面。'), 'warning', t($language, '移动失败'));
      return;
    }
    startProgress(t($language, '移动文件'), 1, payload.name);
    try {
      await api.transfer('move', payload.rootId, [payload.path], destRoot, destPath, csrf);
      updateProgress(1, t($language, '完成'));
      finishProgress();
      if (activeRootId !== null) await loadList(activeRootId, currentPath);
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '移动失败'));
      handleFilesError(error);
    }
  }

  async function submitTransfer(destRoot: number, destPath: string): Promise<void> {
    const pending = transfer;
    transfer = null;
    if (!pending || activeRootId === null) return;
    const rootId = activeRootId;
    startProgress(
      pending.mode === 'copy' ? t($language, '复制文件') : t($language, '移动文件'),
      pending.entries.length,
    );
    try {
      await api.transfer(
        pending.mode,
        rootId,
        pending.entries.map((entry) => joinPath(currentPath, entry.name)),
        destRoot,
        destPath,
        csrf,
      );
      updateProgress(pending.entries.length, t($language, '完成'));
      await loadList(rootId, currentPath);
      finishProgress();
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '操作失败'));
      handleFilesError(error);
    }
  }

  function isTypingTarget(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLElement &&
      (target.tagName === 'INPUT' ||
        target.tagName === 'TEXTAREA' ||
        target.tagName === 'SELECT' ||
        target.isContentEditable)
    );
  }

  function isTextFile(name: string): boolean {
    const lower = name.toLowerCase();
    const dot = lower.lastIndexOf('.');
    const extension = dot > 0 ? lower.slice(dot + 1) : '';
    const binaryExtensions = new Set([
      'jpg',
      'jpeg',
      'png',
      'gif',
      'webp',
      'bmp',
      'tif',
      'tiff',
      'avif',
      'heic',
      'heif',
      'mp4',
      'mov',
      'mkv',
      'webm',
      'avi',
      'm4v',
      'mp3',
      'wav',
      'ogg',
      'flac',
      'aac',
      'm4a',
      'zip',
      'rar',
      '7z',
      'tar',
      'gz',
      'bz2',
      'xz',
      'pdf',
      'doc',
      'docx',
      'xls',
      'xlsx',
      'ppt',
      'pptx',
      'exe',
      'dll',
      'so',
      'dylib',
      'bin',
      'woff',
      'woff2',
      'ttf',
      'otf',
    ]);
    return !binaryExtensions.has(extension);
  }

  function isMarkdown(name: string): boolean {
    return /\.(md|markdown)$/i.test(name);
  }

  function downloadEntry(entry: Entry): void {
    if (activeRootId === null) return;
    window.location.href = api.downloadUrl(activeRootId, joinPath(currentPath, entry.name));
  }

  function isSelected(name: string): boolean {
    return selected.includes(name);
  }

  function toggleSelection(name: string): void {
    selected = isSelected(name) ? selected.filter((item) => item !== name) : [...selected, name];
  }

  function clearSelection(): void {
    selected = [];
    selectionAnchor = null;
  }

  function handleContentClick(event: MouseEvent): void {
    if (appView !== 'files') return;
    if (suppressSelectionClear) {
      suppressSelectionClear = false;
      return;
    }
    const target = event.target as HTMLElement;
    if (target.closest('[data-entry-name]') || target.closest('button,input,a')) return;
    selected = [];
    contextMenu = null;
    blankMenu = null;
  }

  function handleBlankContextMenu(event: MouseEvent): void {
    if (appView !== 'files') return;
    const target = event.target as HTMLElement;
    if (
      target.closest('[data-entry-name]') ||
      target.closest('button,input,a') ||
      target.closest('.context-menu')
    ) {
      return;
    }
    event.preventDefault();
    contextMenu = null;
    blankMenu = menuPosition(event.clientX, event.clientY);
  }

  function startMarquee(event: MouseEvent): void {
    if (appView !== 'files') return;
    suppressSelectionClear = false;
    if (event.button !== 0 || isTypingTarget(event.target)) return;
    const target = event.target as HTMLElement;
    if (target.closest('[data-entry-name]') || target.closest('button,input,a')) return;
    marquee = { x1: event.clientX, y1: event.clientY, x2: event.clientX, y2: event.clientY };
  }

  function updateMarquee(event: MouseEvent): void {
    if (appView !== 'files' || !marquee) return;
    const next = { ...marquee, x2: event.clientX, y2: event.clientY };
    marquee = next;
    const left = Math.min(next.x1, next.x2);
    const right = Math.max(next.x1, next.x2);
    const top = Math.min(next.y1, next.y2);
    const bottom = Math.max(next.y1, next.y2);
    marqueePreview = Array.from(
      document.querySelectorAll<HTMLElement>('[data-entry-name]'),
    ).flatMap((element) => {
      const rect = element.getBoundingClientRect();
      const name = element.dataset.entryName;
      return name &&
        rect.left < right &&
        rect.right > left &&
        rect.top < bottom &&
        rect.bottom > top
        ? [name]
        : [];
    });
  }

  function finishMarquee(): void {
    if (appView !== 'files' || !marquee) return;
    const left = Math.min(marquee.x1, marquee.x2);
    const right = Math.max(marquee.x1, marquee.x2);
    const top = Math.min(marquee.y1, marquee.y2);
    const bottom = Math.max(marquee.y1, marquee.y2);
    const names: string[] = [];

    document.querySelectorAll<HTMLElement>('[data-entry-name]').forEach((element) => {
      const rect = element.getBoundingClientRect();
      const intersects =
        rect.left < right && rect.right > left && rect.top < bottom && rect.bottom > top;
      const name = element.dataset.entryName;
      if (intersects && name) {
        names.push(name);
      }
    });

    if (names.length > 0) {
      selected = Array.from(new Set([...selected, ...names]));
    }
    if (Math.abs(right - left) > 2 || Math.abs(bottom - top) > 2) {
      suppressSelectionClear = true;
    }
    marquee = null;
    marqueePreview = [];
  }

  function uploadClick(): void {
    if (!activeRoot || activeRoot.read_only || uploading) return;
    fileInput?.click();
  }

  async function handleFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    await uploadFiles(files);
  }

  async function handleFolderFiles(event: Event): Promise<void> {
    const input = event.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    await uploadFiles(files, true);
  }

  function folderUploadClick(): void {
    if (!activeRoot || activeRoot.read_only || uploading) return;
    folderInput?.click();
  }

  async function uploadFiles(files: File[], preservePaths = false): Promise<void> {
    if (activeRootId === null || files.length === 0) return;

    filesError = '';
    uploading = true;
    startProgress(t($language, '上传文件'), files.length);
    try {
      for (const [index, file] of files.entries()) {
        updateProgress(index, file.name);
        if (file.size === 0) {
          throw new Error(t($language, '空文件暂不支持上传：{name}', { name: file.name }));
        }
        const relative = preservePaths ? file.webkitRelativePath || file.name : file.name;
        const parts = relative.split('/').filter(Boolean);
        const filename = parts.pop() ?? file.name;
        const directory = parts.join('/');
        const targetPath = joinPath(currentPath, directory);
        if (directory) {
          await api.mkdir(activeRootId, currentPath, directory, csrf, true);
        }
        const created = await api.createUpload(activeRootId, targetPath, filename, file.size, csrf);

        for (let index = 0; index < created.chunks; index += 1) {
          const start = index * created.chunk_size;
          const end = Math.min(start + created.chunk_size, file.size);
          await api.uploadChunk(created.upload_id, index, file.slice(start, end), csrf);
        }
        await api.completeUpload(created.upload_id, csrf);
      }
      updateProgress(files.length, t($language, '{count} 个文件完成', { count: files.length }));
      await loadList(activeRootId, currentPath);
      finishProgress();
    } catch (error) {
      failProgress(error instanceof Error ? error.message : t($language, '上传失败'));
      handleFilesError(error);
    } finally {
      uploading = false;
    }
  }

  async function deleteEntry(entry: Entry): Promise<void> {
    await requestDelete([entry]);
  }

  async function requestDelete(items: Entry[]): Promise<void> {
    if (activeRootId === null || !activeRoot || activeRoot.read_only || items.length === 0) {
      return;
    }

    const rootId = activeRootId;
    const path = currentPath;
    try {
      const paths = items.map((entry) => joinPath(path, entry.name));
      const result = await api.deletePreview(rootId, paths, csrf);
      const totalSize = items.reduce((sum, entry) => sum + entry.size, 0);
      const needsCode =
        items.some((entry) => entry.is_dir) || items.length > 5 || totalSize > 10 * 1024 * 1024;
      const list = items
        .slice(0, 12)
        .map((entry) => `× ${entry.name}${entry.is_dir ? '/' : ''}`)
        .join('\n');
      const more =
        items.length > 12
          ? '\n' + t($language, '…以及另外 {count} 项', { count: items.length - 12 })
          : '';
      confirmState = {
        title: t($language, '永久删除'),
        message: t($language, '以下文件将被永久删除：\n{list}{more}', { list, more }),
        requireText: needsCode ? result.confirm_code : null,
        action: (value) => {
          void (async () => {
            startProgress(t($language, '删除文件'), items.length, t($language, '正在删除…'));
            try {
              await api.deletePaths(result.token, undefined, csrf, needsCode ? value : undefined);
              updateProgress(items.length, t($language, '完成'));
              finishProgress();
              confirmState = null;
              await loadList(rootId, path);
            } catch (error) {
              failProgress(error instanceof Error ? error.message : t($language, '删除失败'));
              handleFilesError(error);
              confirmState = null;
            }
          })();
        },
      };
    } catch (error) {
      handleFilesError(error);
    }
  }

  async function deleteSelected(): Promise<void> {
    await requestDelete(entries.filter((entry) => selected.includes(entry.name)));
  }

  function setSort(field: 'name' | 'size' | 'modified'): void {
    if (sortField === field) {
      sortOrder = sortOrder === 'asc' ? 'desc' : 'asc';
    } else {
      sortField = field;
      sortOrder = 'asc';
    }
    if (activeRootId !== null) {
      localStorage.setItem(`sort:${activeRootId}:${currentPath}`, `${sortField}:${sortOrder}`);
      void loadList(activeRootId, currentPath);
    }
  }

  function setView(next: 'list' | 'thumb' | 'tile'): void {
    view = next;
    if (activeRootId !== null) {
      localStorage.setItem(`view:${activeRootId}:${currentPath}`, next);
    }
  }

  async function openEditor(entry: Entry): Promise<void> {
    if (activeRootId === null) return;
    const path = joinPath(currentPath, entry.name);
    try {
      const probe = await api.probeFile(activeRootId, path);
      if (!probe.is_text) {
        showUnsupported(entry);
        return;
      }
      const file = await api.getFile(activeRootId, path);
      editorName = entry.name;
      editorPath = path;
      editorContent = file.content;
      editorOriginalContent = file.content;
      editorEtag = file.etag;
      editorError = '';
      editorPreview = false;
      editorOpen = true;
    } catch (error) {
      if (error instanceof ApiError && (error.status === 400 || error.status === 415)) {
        showUnsupported(entry);
      } else {
        handleFilesError(error);
      }
    }
  }

  async function saveEditor(auto = false): Promise<void> {
    if (activeRootId === null || !editorOpen) return;
    editorSaving = true;
    editorError = '';
    try {
      const saved = await api.saveFile(activeRootId, editorPath, editorContent, editorEtag, csrf);
      editorEtag = saved.etag;
      editorOriginalContent = editorContent;
      if (!auto) editorOpen = false;
      await loadList(activeRootId, currentPath);
    } catch (error) {
      if (error instanceof ApiError && error.status === 412) {
        editorError = t($language, '文件已被其他操作修改，请关闭后重新打开。');
      } else {
        editorError = error instanceof Error ? error.message : t($language, '保存失败');
      }
    } finally {
      editorSaving = false;
    }
  }

  async function downloadSelectedZip(): Promise<void> {
    if (activeRootId === null || selected.length === 0) return;

    try {
      const paths = selected.map((name) => joinPath(currentPath, name));
      const blob = await api.downloadZip(activeRootId, paths, csrf);
      const url = URL.createObjectURL(blob);
      const anchor = document.createElement('a');
      anchor.href = url;
      anchor.download = 'filehub.zip';
      anchor.click();
      URL.revokeObjectURL(url);
    } catch (error) {
      handleFilesError(error);
    }
  }

  function splitFileName(name: string): { stem: string; ext: string } {
    const dot = name.lastIndexOf('.');
    if (dot <= 0) return { stem: name, ext: '' };
    return { stem: name.slice(0, dot), ext: name.slice(dot) };
  }

  function formatSize(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
    return `${(size / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  function isImageName(name: string): boolean {
    return /\.(jpe?g|png|gif|webp|bmp|tiff?)$/i.test(name);
  }

  function hideBrokenThumb(event: Event): void {
    const image = event.currentTarget as HTMLImageElement;
    image.style.display = 'none';
  }

  function formatDate(seconds: number): string {
    if (!seconds) return '—';
    return new Intl.DateTimeFormat(localeOf($language), {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    }).format(new Date(seconds * 1000));
  }

  function formatRelativeDate(seconds: number): string {
    if (!seconds) return '—';
    const diff = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
    if (diff < 60) return t($language, '刚刚');
    if (diff < 3600) return t($language, '{count} 分钟前', { count: Math.floor(diff / 60) });
    if (diff < 86400) return t($language, '{count} 小时前', { count: Math.floor(diff / 3600) });
    if (diff < 7 * 86400) return t($language, '{count} 天前', { count: Math.floor(diff / 86400) });
    return formatDate(seconds);
  }
</script>

<svelte:head>
  <title>{pageTitle}</title>
  {#if preferences?.custom_css_path && preferences.custom_css_version > 0}
    <link rel="stylesheet" href={api.customCssUrl(preferences.custom_css_version)} />
  {/if}
</svelte:head>

<svelte:window
  onclick={() => {
    contextMenu = null;
    blankMenu = null;
  }}
  onscroll={() => {
    contextMenu = null;
    blankMenu = null;
  }}
/>

{#if shareToken}
  <SharePage token={shareToken} />
{:else if loading}
  <div class="boot-screen" aria-label={t($language, '正在加载')}>
    <div class="boot-mark" aria-hidden="true">F</div>
  </div>
{:else if !initialized}
  <AuthScreen mode="setup" onSuccess={handleAuth} />
{:else if !user}
  <AuthScreen mode="login" onSuccess={handleAuth} />
{:else}
  <div
    class="app"
    role="application"
    ondragover={(event) => event.preventDefault()}
    ondrop={(event) => {
      event.preventDefault();
      void uploadFiles(Array.from(event.dataTransfer?.files ?? []));
    }}
  >
    <header class="topbar">
      <button
        type="button"
        class="icon-button mobile-menu"
        aria-label={t($language, '打开菜单')}
        onclick={() => (sidebarOpen = true)}
      >
        <Icon name="menu" size={20} />
      </button>
      <div class="topbar-brand">
        <span class="brand-mark" aria-hidden="true">F</span>
        <span class="brand-name">FileHub</span>
      </div>

      {#if appView === 'files'}
        <div class="search">
          <Icon name="search" size={18} />
          <input
            bind:this={searchInput}
            bind:value={searchQuery}
            aria-label={t($language, '搜索文件或目录')}
            placeholder={t($language, '搜索文件或目录')}
            oninput={handleSearchInput}
          />
        </div>
      {:else}
        <div class="page-context"></div>
      {/if}

      <div class="top-actions">
        {#if appView === 'files'}
          <div class="segmented" role="group" aria-label={t($language, '视图')}>
            <button
              type="button"
              class:active={view === 'list'}
              aria-pressed={view === 'list'}
              aria-label={t($language, '列表视图')}
              title={t($language, '列表视图')}
              onclick={() => setView('list')}
            >
              <Icon name="list" size={18} />
            </button>
            <button
              type="button"
              class:active={view === 'thumb'}
              aria-pressed={view === 'thumb'}
              aria-label={t($language, '缩略图视图')}
              title={t($language, '缩略图视图')}
              onclick={() => setView('thumb')}
            >
              <Icon name="image" size={18} />
            </button>
            <button
              type="button"
              class:active={view === 'tile'}
              aria-pressed={view === 'tile'}
              aria-label={t($language, '平铺视图')}
              title={t($language, '平铺视图')}
              onclick={() => setView('tile')}
            >
              <Icon name="grid" size={18} />
            </button>
          </div>
        {/if}

        <button
          type="button"
          class="icon-button"
          aria-label={theme === 'dark'
            ? t($language, '切换到浅色模式')
            : t($language, '切换到深色模式')}
          title={theme === 'dark' ? t($language, '切换到浅色模式') : t($language, '切换到深色模式')}
          onclick={toggleTheme}
        >
          <Icon name={theme === 'dark' ? 'sun' : 'moon'} size={19} />
        </button>

        {#if appView === 'files'}
          <button
            type="button"
            class="upload-top"
            disabled={!activeRoot || activeRoot.read_only || uploading}
            title={activeRoot?.read_only ? t($language, '当前目录只读') : t($language, '上传文件')}
            aria-label={t($language, '上传')}
            onclick={uploadClick}
          >
            <Icon name="upload" size={18} />
            <span>{uploading ? t($language, '上传中…') : t($language, '上传')}</span>
          </button>
        {/if}

        <button
          type="button"
          class="top-avatar-button"
          aria-label={t($language, '账号菜单')}
          title={t($language, '账号菜单')}
          onclick={() => (accountOpen = true)}
        >
          {#if preferences && preferences.avatar_version > 0}
            <img src={api.avatarUrl(preferences.avatar_version)} alt="" />
          {:else}
            {user.username.slice(0, 1).toUpperCase()}
          {/if}
        </button>
      </div>
    </header>

    <div class="workspace">
      <aside class:open={sidebarOpen} class="sidebar" aria-label={t($language, '主导航')}>
        <nav class="nav" aria-label={t($language, '文件导航')}>
          {#each roots as root (root.id)}
            <button
              type="button"
              class:active={appView === 'files' && root.id === activeRootId}
              class="nav-item"
              onclick={() => {
                appView = 'files';
                activeRootId = root.id;
                void loadList(root.id, '');
              }}
            >
              <Icon name="folder" size={18} />
              <span>{rootLabel(root.alias)}</span>
            </button>
          {/each}
          {#if roots.length === 0}
            <span class="nav-empty">{t($language, '暂无目录')}</span>
          {/if}
          <div class="nav-sep"></div>
          <button
            type="button"
            class="nav-item"
            disabled={!activeRoot}
            title={activeRoot ? t($language, '终端') : t($language, '请先选择存储目录')}
            onclick={() => (terminalOpen = true)}
          >
            <Icon name="terminal" size={18} />
            <span>{t($language, '终端')}</span>
          </button>
          <button
            type="button"
            class:active={appView === 'shares'}
            class="nav-item"
            onclick={() => (appView = 'shares')}
          >
            <Icon name="share" size={18} />
            <span>{t($language, '分享管理')}</span>
          </button>
        </nav>

        <div class="sidebar-spacer"></div>

        <div class="quota-card" aria-label={t($language, '存储状态')}>
          <div class="quota-row">
            <div class="quota-label">
              <span>{t($language, '用户配额')}</span>
              <span class="quota-value">{quotaLabel}</span>
            </div>
            <div class="quota-track">
              <div class="quota-fill" style={`width:${quotaPercent}%`}></div>
            </div>
          </div>
          {#each usage?.disks ?? [] as disk (disk.name)}
            <div class="quota-row">
              <div class="quota-label">
                <span class="disk-name" title={disk.name}>{disk.name}</span>
                <span class="quota-value">{formatSize(disk.used)} / {formatSize(disk.total)}</span>
              </div>
              <div class="quota-track">
                <div
                  class="quota-fill disk"
                  style={`width:${disk.total ? Math.min(100, (disk.used / disk.total) * 100) : 0}%`}
                ></div>
              </div>
            </div>
          {/each}
        </div>
        <div class="app-version">v{__APP_VERSION__}</div>
      </aside>

      <input
        bind:this={fileInput}
        class="upload-input"
        type="file"
        multiple
        onchange={(event) => void handleFiles(event)}
      />
      <input
        bind:this={folderInput}
        class="upload-input"
        type="file"
        multiple
        webkitdirectory
        onchange={(event) => void handleFolderFiles(event)}
      />

      {#if sidebarOpen}
        <button
          type="button"
          class="overlay"
          aria-label={t($language, '关闭菜单')}
          onclick={() => (sidebarOpen = false)}
        ></button>
      {/if}

      <div class="main">
        {#if appView === 'files'}
          <div class="pathbar">
            <div class="path">
              <button type="button" class="path-link" onclick={() => navigate('')}>
                {activeRoot ? rootLabel(activeRoot.alias) : t($language, '文件')}
              </button>
              {#each pathSegments as segment, index (index)}
                <span class="sep">/</span>
                <button type="button" class="path-link" onclick={() => navigateToSegment(index)}>
                  {segment}
                </button>
              {/each}
            </div>
            {#if selected.length > 0}
              <div class="pathbar-actions">
                <span class="selection-count"
                  >{t($language, '已选 {count} 项', { count: selected.length })}</span
                >
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '下载 ZIP')}
                  title={t($language, '下载 ZIP')}
                  onclick={() => void downloadSelectedZip()}
                >
                  <Icon name="download" size={16} />
                </button>
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '复制到')}
                  title={t($language, '复制到')}
                  onclick={() =>
                    (transfer = {
                      mode: 'copy',
                      entries: entries.filter((entry) => selected.includes(entry.name)),
                    })}
                >
                  <Icon name="copy" size={16} />
                </button>
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '移动到')}
                  title={t($language, '移动到')}
                  onclick={() =>
                    (transfer = {
                      mode: 'move',
                      entries: entries.filter((entry) => selected.includes(entry.name)),
                    })}
                >
                  <Icon name="move" size={16} />
                </button>
                <button
                  type="button"
                  class="icon-button danger"
                  aria-label={t($language, '删除')}
                  title={t($language, '删除')}
                  onclick={() => void deleteSelected()}
                >
                  <Icon name="trash" size={16} />
                </button>
                <button
                  type="button"
                  class="icon-button"
                  aria-label={t($language, '取消选择')}
                  title={t($language, '取消选择')}
                  onclick={clearSelection}
                >
                  <Icon name="close" size={16} />
                </button>
              </div>
            {/if}
          </div>
        {/if}

        <main class="content" onscroll={handleContentScroll}>
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div
            class="content-drag"
            class:file-area={appView === 'files'}
            role="application"
            aria-label={t($language, '文件内容')}
            onmousedown={startMarquee}
            onmousemove={updateMarquee}
            onmouseup={finishMarquee}
            onclick={handleContentClick}
            oncontextmenu={handleBlankContextMenu}
          >
            {#if appView === 'settings'}
              {#if preferences}
                <PreferencesPage
                  {user}
                  {csrf}
                  {roots}
                  {preferences}
                  system={systemFeatures}
                  onPreferences={applyPreferences}
                />
              {/if}
            {:else if appView === 'admin'}
              <AdminPage {csrf} onAudit={() => (appView = 'audit')} />
            {:else if appView === 'audit'}
              <AuditPage onBack={() => (appView = 'admin')} />
            {:else if appView === 'shares'}
              <SharesPage {csrf} />
            {:else}
              {#if searchActive}
                <div class="search-header">
                  <span>{t($language, '搜索')} “{searchQuery}”</span>
                  {#if searchIndexing}
                    <span class="muted">{t($language, '索引中…')}</span>
                  {/if}
                  <button type="button" class="ghost" onclick={exitSearch}
                    >{t($language, '清除')}</button
                  >
                </div>
                {#if searchResults.length === 0}
                  <div class="files-state">{t($language, '没有找到匹配的文件')}</div>
                {:else}
                  <div class="search-results">
                    {#each searchResults as result (result.path)}
                      <button
                        type="button"
                        class="search-result"
                        onclick={() => openSearchResult(result)}
                      >
                        <Icon name={result.is_dir ? 'folder' : 'file'} size={18} />
                        <span class="file-name">{result.name}</span>
                        <span class="search-path">{result.path}</span>
                        <span class="search-size">
                          {result.is_dir ? '—' : formatSize(result.size)}
                        </span>
                      </button>
                    {/each}
                  </div>
                {/if}
              {:else if filesLoading}
                <div class="files-state">{t($language, '加载中…')}</div>
              {:else if filesError}
                <div class="files-state">
                  <p>{filesError}</p>
                  <button
                    type="button"
                    class="retry-button"
                    onclick={() => {
                      if (activeRootId !== null) void loadList(activeRootId, currentPath);
                    }}
                  >
                    {t($language, '重试')}
                  </button>
                </div>
              {:else}
                {#if entries.length === 0}
                  <section class="empty-state" aria-labelledby="empty-title">
                    <div class="empty-icon">
                      <Icon name="folder" size={52} />
                    </div>
                    <h1 id="empty-title">{t($language, '这个目录是空的')}</h1>
                    <p>{t($language, '上传文件或创建目录后，它们会显示在这里。')}</p>
                    <button
                      type="button"
                      class="upload-primary"
                      disabled={!activeRoot || activeRoot.read_only || uploading}
                      onclick={uploadClick}
                    >
                      <Icon name="upload" size={18} />
                      <span>{uploading ? t($language, '上传中…') : t($language, '上传文件')}</span>
                    </button>
                  </section>
                {:else if view === 'list'}
                  <div class="browser">
                    <table class="file-table">
                      <thead>
                        <tr>
                          <th class="check"></th>
                          <th
                            aria-sort={sortField === 'name'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('name')}
                            >
                              {t($language, '名称')}{sortField === 'name'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th
                            class="size"
                            aria-sort={sortField === 'size'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('size')}
                            >
                              {t($language, '大小')}{sortField === 'size'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th
                            class="modified"
                            aria-sort={sortField === 'modified'
                              ? sortOrder === 'asc'
                                ? 'ascending'
                                : 'descending'
                              : 'none'}
                          >
                            <button
                              type="button"
                              class="sort-button"
                              onclick={() => setSort('modified')}
                            >
                              {t($language, '修改时间')}{sortField === 'modified'
                                ? sortOrder === 'asc'
                                  ? ' ↑'
                                  : ' ↓'
                                : ''}
                            </button>
                          </th>
                          <th class="actions"></th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each entries as entry (entry.name)}
                          {@const nameParts = splitFileName(entry.name)}
                          <tr
                            data-entry-name={entry.name}
                            class:selected={isSelected(entry.name)}
                            class:marquee-selected={marqueePreview.includes(entry.name)}
                            draggable="true"
                            onclick={(event) => handleEntryClick(event, entry)}
                            ondblclick={() => enterEntry(entry)}
                            oncontextmenu={(event) => openContextMenu(event, entry)}
                            ondragstart={(event) => startDrag(event, entry)}
                            ondragend={() => (dragPayload = null)}
                            ondragover={(event) => {
                              if (entry.is_dir) allowDrop(event);
                            }}
                            ondrop={(event) => {
                              if (entry.is_dir) {
                                void dropInto(
                                  event,
                                  activeRootId ?? 0,
                                  joinPath(currentPath, entry.name),
                                );
                              }
                            }}
                          >
                            <td class="check" onclick={(event) => event.stopPropagation()}>
                              <input
                                type="checkbox"
                                aria-label={t($language, '选择 {name}', { name: entry.name })}
                                checked={isSelected(entry.name)}
                                onchange={() => toggleSelection(entry.name)}
                              />
                            </td>
                            <td class="name">
                              <span class="file-icon">
                                <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={22} />
                              </span>
                              <span class="file-name"
                                ><span class="name-stem">{nameParts.stem}</span><span
                                  class="name-ext">{nameParts.ext}</span
                                ></span
                              >
                              {#if entry.is_symlink}
                                <span class="tag">{t($language, '链接')}</span>
                              {/if}
                            </td>
                            <td class="size">{entrySize(entry)}</td>
                            <td class="modified">{formatRelativeDate(entry.modified)}</td>
                            <td class="actions">
                              <button
                                type="button"
                                class="row-action"
                                aria-label={t($language, '删除 {name}', { name: entry.name })}
                                title={t($language, '删除')}
                                disabled={!activeRoot || activeRoot.read_only}
                                onclick={(event) => {
                                  event.stopPropagation();
                                  void deleteEntry(entry);
                                }}
                              >
                                <Icon name="trash" size={16} />
                              </button>
                            </td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {:else if view === 'thumb'}
                  <div class="browser">
                    <div class="file-grid">
                      {#each entries as entry (entry.name)}
                        {@const nameParts = splitFileName(entry.name)}
                        <div
                          class="file-card"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={() => (dragPayload = null)}
                          ondragover={(event) => {
                            if (entry.is_dir) allowDrop(event);
                          }}
                          ondrop={(event) => {
                            if (entry.is_dir) {
                              void dropInto(
                                event,
                                activeRootId ?? 0,
                                joinPath(currentPath, entry.name),
                              );
                            }
                          }}
                          onkeydown={(event) => {
                            if (event.key === 'Enter' || event.key === ' ') {
                              event.preventDefault();
                              enterEntry(entry);
                            }
                          }}
                        >
                          <input
                            class="card-check"
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: entry.name })}
                            checked={isSelected(entry.name)}
                            onclick={(event) => event.stopPropagation()}
                            onchange={() => toggleSelection(entry.name)}
                          />
                          <div class="card-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={40} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && (isImageName(entry.name) || isVideoName(entry.name)) && activeRootId !== null}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  joinPath(currentPath, entry.name),
                                  320,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                onerror={hideBrokenThumb}
                              />
                            {/if}
                          </div>
                          <span class="card-name"
                            ><span class="name-stem">{nameParts.stem}</span><span class="name-ext"
                              >{nameParts.ext}</span
                            ></span
                          >
                          <span class="card-meta">
                            {entrySize(entry)} · {formatRelativeDate(entry.modified)}
                          </span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {:else}
                  <div class="browser">
                    <div class="tile-grid">
                      {#each entries as entry (entry.name)}
                        {@const nameParts = splitFileName(entry.name)}
                        <div
                          class="file-tile"
                          data-entry-name={entry.name}
                          class:selected={isSelected(entry.name)}
                          class:marquee-selected={marqueePreview.includes(entry.name)}
                          role="button"
                          tabindex="0"
                          draggable="true"
                          onclick={(event) => handleEntryClick(event, entry)}
                          ondblclick={() => enterEntry(entry)}
                          oncontextmenu={(event) => openContextMenu(event, entry)}
                          ondragstart={(event) => startDrag(event, entry)}
                          ondragend={() => (dragPayload = null)}
                          ondragover={(event) => {
                            if (entry.is_dir) allowDrop(event);
                          }}
                          ondrop={(event) => {
                            if (entry.is_dir) {
                              void dropInto(
                                event,
                                activeRootId ?? 0,
                                joinPath(currentPath, entry.name),
                              );
                            }
                          }}
                          onkeydown={(event) => {
                            if (event.key === 'Enter' || event.key === ' ') {
                              event.preventDefault();
                              enterEntry(entry);
                            }
                          }}
                        >
                          <input
                            class="card-check"
                            type="checkbox"
                            aria-label={t($language, '选择 {name}', { name: entry.name })}
                            checked={isSelected(entry.name)}
                            onclick={(event) => event.stopPropagation()}
                            onchange={() => toggleSelection(entry.name)}
                          />
                          <div class="tile-icon">
                            <FileTypeIcon name={entry.name} isDir={entry.is_dir} size={34} />
                            {#if systemFeatures.thumbnails && preferences?.show_thumbnails !== false && !entry.is_dir && (isImageName(entry.name) || isVideoName(entry.name)) && activeRootId !== null}
                              <img
                                class="thumb-image"
                                src={api.thumbUrl(
                                  activeRootId,
                                  joinPath(currentPath, entry.name),
                                  240,
                                )}
                                alt=""
                                loading="lazy"
                                decoding="async"
                                onerror={hideBrokenThumb}
                              />
                            {/if}
                          </div>
                          <span class="tile-info">
                            <span class="tile-name"
                              ><span class="name-stem">{nameParts.stem}</span><span class="name-ext"
                                >{nameParts.ext}</span
                              ></span
                            >
                            <span class="tile-meta">
                              {entrySize(entry)} · {formatRelativeDate(entry.modified)}
                            </span>
                          </span>
                        </div>
                      {/each}
                    </div>
                  </div>
                {/if}
                {#if nextCursor || filesLoadingMore}
                  <div class="load-more">
                    {filesLoadingMore ? t($language, '加载中…') : t($language, '继续滚动加载更多')}
                  </div>
                {/if}
              {/if}
            {/if}
          </div>
        </main>
      </div>
    </div>
  </div>

  <NoticeDialog />
  <ProgressPanel />
  <ToastPanel />

  {#if accountOpen}
    <AccountDialog
      {user}
      avatarUrl={preferences && preferences.avatar_version > 0
        ? api.avatarUrl(preferences.avatar_version)
        : ''}
      onSettings={() => {
        appView = 'settings';
        accountOpen = false;
      }}
      onAdmin={() => {
        appView = 'admin';
        accountOpen = false;
      }}
      onLogout={() => {
        accountOpen = false;
        void logout();
      }}
      onClose={() => (accountOpen = false)}
    />
  {/if}

  {#if contextMenu}
    <div
      class="context-menu"
      role="menu"
      tabindex="-1"
      style={`left:${contextMenu.x}px;top:${contextMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => {
        if (event.key === 'Escape') contextMenu = null;
      }}
    >
      {#if canPreviewEntry(contextMenu.entry)}
        <button type="button" role="menuitem" onclick={() => runContextAction('preview')}
          >{t($language, '预览')}</button
        >
      {/if}
      <button type="button" role="menuitem" onclick={() => runContextAction('download')}
        >{t($language, '下载')}</button
      >
      <button type="button" role="menuitem" onclick={() => runContextAction('share')}
        >{t($language, '分享')}</button
      >
      {#if !activeRoot?.read_only}
        <button type="button" role="menuitem" onclick={() => runContextAction('rename')}
          >{t($language, '重命名')}</button
        >
      {/if}
      <button type="button" role="menuitem" onclick={() => runContextAction('copy-to')}
        >{t($language, '复制到…')}</button
      >
      <button type="button" role="menuitem" onclick={() => runContextAction('move-to')}
        >{t($language, '移动到…')}</button
      >
      <div class="menu-sep"></div>
      <button
        type="button"
        role="menuitem"
        class="danger"
        onclick={() => runContextAction('delete')}>{t($language, '删除')}</button
      >
    </div>
  {/if}

  {#if blankMenu}
    <div
      class="context-menu"
      class:open-left={blankMenu.x > window.innerWidth / 2}
      role="menu"
      tabindex="-1"
      style={`left:${blankMenu.x}px;top:${blankMenu.y}px`}
      onclick={(event) => event.stopPropagation()}
      onkeydown={(event) => {
        if (event.key === 'Escape') blankMenu = null;
      }}
    >
      {#if !activeRoot?.read_only}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            blankMenu = null;
            openMkdir();
          }}>{t($language, '新建目录')}</button
        >
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            blankMenu = null;
            nameDialog = {
              mode: 'touch',
              entry: null,
              title: t($language, '新建文件'),
              label: t($language, '文件名称'),
              initialValue: '',
            };
          }}>{t($language, '新建文件')}</button
        >
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            blankMenu = null;
            folderUploadClick();
          }}>{t($language, '上传目录')}</button
        >
      {/if}
      <button
        type="button"
        role="menuitem"
        onclick={() => {
          blankMenu = null;
          terminalOpen = true;
        }}>{t($language, '打开终端')}</button
      >
      <div class="menu-sep"></div>
      <div class="menu-item">
        <button type="button" role="menuitem">
          {t($language, '排序方式')} <span class="menu-arrow">›</span>
        </button>
        <div class="submenu" role="menu">
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('name');
            }}
          >
            {t($language, '名称')}
            {sortField === 'name' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('size');
            }}
          >
            {t($language, '大小')}
            {sortField === 'size' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setSort('modified');
            }}
          >
            {t($language, '修改时间')}
            {sortField === 'modified' ? (sortOrder === 'asc' ? '↑' : '↓') : ''}
          </button>
        </div>
      </div>
      <div class="menu-item">
        <button type="button" role="menuitem">
          {t($language, '视图切换')} <span class="menu-arrow">›</span>
        </button>
        <div class="submenu" role="menu">
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('list');
            }}>{t($language, '列表')}</button
          >
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('thumb');
            }}>{t($language, '缩略图')}</button
          >
          <button
            type="button"
            onclick={() => {
              blankMenu = null;
              setView('tile');
            }}>{t($language, '平铺')}</button
          >
        </div>
      </div>
    </div>
  {/if}

  {#if confirmState}
    <ConfirmDialog
      open
      title={confirmState.title}
      message={confirmState.message}
      confirmLabel={t($language, '删除')}
      danger
      requireText={confirmState.requireText}
      onConfirm={(value) => {
        const action = confirmState?.action;
        confirmState = null;
        action?.(value);
      }}
      onCancel={() => (confirmState = null)}
    />
  {/if}

  {#if preview && activeRootId !== null}
    <PreviewModal
      root={activeRootId}
      {currentPath}
      items={preview.items}
      index={preview.index}
      onClose={() => (preview = null)}
      onIndexChange={(index) => (preview = preview ? { ...preview, index } : null)}
    />
  {/if}

  {#if nameDialog}
    <NameDialog
      open
      title={nameDialog.title}
      label={nameDialog.label}
      initialValue={nameDialog.initialValue}
      confirmLabel={nameDialog.mode === 'mkdir' ? t($language, '创建') : t($language, '重命名')}
      onConfirm={(value) => void submitName(value)}
      onClose={() => (nameDialog = null)}
    />
  {/if}

  {#if transfer && activeRootId !== null}
    <TransferDialog
      open
      mode={transfer.mode}
      count={transfer.entries.length}
      {roots}
      initialRoot={activeRootId}
      initialPath={currentPath}
      onConfirm={(rootId, path) => void submitTransfer(rootId, path)}
      onClose={() => (transfer = null)}
    />
  {/if}

  {#if terminalOpen && activeRootId !== null}
    <TerminalPanel
      root={activeRootId}
      cwd={currentPath}
      {csrf}
      onClose={() => (terminalOpen = false)}
      onRefresh={() => {
        if (activeRootId !== null) void loadList(activeRootId, currentPath);
      }}
    />
  {/if}

  {#if shareTarget && activeRootId !== null}
    <ShareDialog
      root={activeRootId}
      path={joinPath(currentPath, shareTarget.name)}
      name={shareTarget.name}
      isDir={shareTarget.is_dir}
      {csrf}
      onClose={() => (shareTarget = null)}
    />
  {/if}

  {#if marquee}
    <div
      class="marquee"
      style={`left:${Math.min(marquee.x1, marquee.x2)}px;top:${Math.min(
        marquee.y1,
        marquee.y2,
      )}px;width:${Math.abs(marquee.x2 - marquee.x1)}px;height:${Math.abs(
        marquee.y2 - marquee.y1,
      )}px`}
    ></div>
  {/if}

  {#if editorOpen}
    <div class="editor-overlay" role="presentation" onclick={() => (editorOpen = false)}>
      <dialog
        open
        class="editor-panel"
        aria-labelledby="editor-title"
        onclick={(event) => event.stopPropagation()}
      >
        <header class="editor-header">
          <strong id="editor-title">{editorName}</strong>
          <div class="editor-actions">
            {#if isMarkdown(editorName)}
              <button type="button" class="ghost" onclick={() => (editorPreview = !editorPreview)}>
                {editorPreview ? t($language, '编辑') : t($language, '预览')}
              </button>
            {/if}
            <button
              type="button"
              class="icon-button"
              aria-label={t($language, '关闭编辑器')}
              onclick={() => (editorOpen = false)}
            >
              <Icon name="close" size={18} />
            </button>
          </div>
        </header>

        {#if editorPreview}
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          <div class="markdown-preview">{@html previewHtml}</div>
        {:else}
          <div class="editor-body">
            <pre
              bind:this={lineNumbersElement}
              class="line-numbers">{#each editorLineNumbers as number (number)}<span>{number}</span
                >{/each}</pre>
            <textarea
              bind:value={editorContent}
              spellcheck="false"
              onscroll={(event) => {
                if (lineNumbersElement) {
                  lineNumbersElement.scrollTop = event.currentTarget.scrollTop;
                }
              }}></textarea>
          </div>
        {/if}

        {#if editorError}
          <p class="editor-error" role="alert">{editorError}</p>
        {/if}

        <footer class="editor-footer">
          <span
            >{editorDirty ? t($language, '有未保存修改') : t($language, '已保存')} · UTF-8 ·
            {editorContent.length} chars</span
          >
          <div>
            <button type="button" class="ghost" onclick={() => (editorOpen = false)}
              >{t($language, '取消')}</button
            >
            <button
              type="button"
              class="primary"
              disabled={editorSaving}
              onclick={() => void saveEditor()}
            >
              {editorSaving ? t($language, '保存中…') : t($language, '保存')}
            </button>
          </div>
        </footer>
      </dialog>
    </div>
  {/if}
{/if}

<style>
  .boot-screen {
    display: grid;
    min-height: 100dvh;
    place-items: center;
    background: var(--bg);
  }

  .boot-mark {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border-radius: 10px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 20px;
    font-weight: 600;
  }

  .app {
    display: flex;
    height: 100dvh;
    flex-direction: column;
    background: var(--surface);
  }

  .workspace {
    display: grid;
    min-height: 0;
    flex: 1;
    overflow: hidden;
    grid-template-columns: var(--sidebar-width) minmax(0, 1fr);
  }

  .sidebar {
    position: sticky;
    top: 0;
    display: flex;
    height: 100%;
    min-height: 0;
    flex-direction: column;
    overflow-y: auto;
    padding: 12px;
    background: var(--surface);
  }

  .brand-mark {
    display: grid;
    width: 24px;
    height: 24px;
    place-items: center;
    border-radius: 7px;
    background: var(--accent);
    color: var(--on-accent);
    font-size: 14.5px;
    font-weight: 600;
  }

  .brand-name {
    min-width: 0;
  }

  .topbar-brand {
    display: flex;
    width: calc(var(--sidebar-width) - 12px);
    flex: 0 0 auto;
    align-items: center;
    gap: 10px;
    font-size: 20px;
    font-weight: 600;
    letter-spacing: -0.02em;
  }

  .top-avatar-button img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .top-avatar-button {
    display: grid;
    width: 34px;
    height: 34px;
    overflow: hidden;
    flex: 0 0 auto;
    place-items: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    box-sizing: border-box;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: 13.5px;
    font-weight: 600;
  }

  .upload-top,
  .upload-primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 0;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--on-accent);
    font-weight: 500;
    white-space: nowrap;
  }

  .upload-top {
    height: 36px;
    padding: 0 14px;
  }

  .upload-primary {
    height: 36px;
    margin-top: 8px;
    padding: 0 16px;
  }

  .upload-top:hover:not(:disabled),
  .upload-primary:hover:not(:disabled) {
    filter: brightness(0.96);
  }

  .upload-top:disabled,
  .upload-primary:disabled {
    opacity: 0.5;
  }

  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: 14px;
  }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 36px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-2);
    font-size: 15px;
    font-weight: 400;
    text-align: left;
  }

  .nav-item :global(svg) {
    stroke-width: 2.1;
  }

  .nav-item:hover:not(:disabled) {
    background: var(--hover);
  }

  .nav-item.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: 700;
  }

  .nav-item.active span {
    font-weight: 700;
  }

  .nav-item:disabled {
    opacity: 0.45;
  }

  .nav-empty {
    padding: 8px 10px;
    color: var(--text-2);
    font-size: 13.5px;
  }

  .nav-sep {
    height: 1px;
    margin: 8px 10px;
    background: var(--border);
  }

  .sidebar-spacer {
    flex: 1;
  }

  .quota-card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .quota-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .quota-label {
    display: flex;
    justify-content: space-between;
    color: var(--text-2);
    font-size: 12.5px;
    font-weight: 600;
  }

  .disk-name {
    overflow: hidden;
    min-width: 0;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .quota-value {
    flex: 0 0 auto;
    color: var(--accent-text);
    font-variant-numeric: tabular-nums;
    font-weight: 700;
  }

  .quota-row:last-child .quota-value {
    color: var(--text);
  }

  .quota-track {
    height: 8px;
    overflow: hidden;
    border-radius: 99px;
    background: var(--border);
  }

  .quota-fill {
    width: 0;
    height: 100%;
    background: var(--accent);
  }

  .app-version {
    padding: 6px 2px 0;
    color: var(--text-2);
    font-size: 11.5px;
    text-align: left;
  }

  .quota-fill.disk {
    background: var(--file-video);
  }

  .main {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex-direction: column;
    overflow: hidden;
    background: var(--surface);
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: 12px;
    height: var(--topbar-height);
    padding: 0 20px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .search {
    display: flex;
    flex: 1;
    align-items: center;
    gap: 8px;
    max-width: 640px;
    height: 40px;
    padding: 0 12px;
    border-radius: var(--radius-pill);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .search:focus-within {
    background: var(--surface);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: transparent;
  }

  .search input::placeholder {
    color: var(--text-2);
    opacity: 0.8;
  }

  .sort-button {
    padding: 0;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
  }

  .sort-button:hover {
    color: var(--text);
  }

  .top-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }

  .segmented {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius);
    background: var(--surface-2);
  }

  .segmented button {
    display: grid;
    width: 32px;
    height: 30px;
    place-items: center;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .segmented button.active {
    background: var(--surface);
    box-shadow: var(--shadow-1);
    color: var(--text);
  }

  .icon-button {
    display: grid;
    width: 36px;
    height: 36px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
  }

  .icon-button:hover:not(:disabled) {
    background: var(--hover);
  }

  .mobile-menu {
    display: none;
  }

  .pathbar-actions {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 2px;
  }

  .selection-count {
    padding: 0 6px;
    color: var(--accent-text);
    font-size: 12.5px;
    white-space: nowrap;
  }

  .pathbar-actions .danger:hover {
    color: var(--danger);
  }

  .pathbar {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 40px;
    padding: 0 20px;
    background: transparent;
  }

  .path {
    display: flex;
    min-width: 0;
    flex: 1;
    align-items: center;
    gap: 8px;
    overflow: auto;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 14px;
    white-space: nowrap;
  }

  .sep {
    opacity: 0.5;
  }

  .path-link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-2);
    font: inherit;
    white-space: nowrap;
  }

  .path-link:hover {
    color: var(--text);
  }

  .content {
    display: block;
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--content-pad);
  }

  .content-drag {
    min-height: 100%;
  }

  .content-drag.file-area {
    user-select: none;
  }

  .browser {
    width: 100%;
  }

  .file-table {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: 15px;
    font-weight: 500;
  }

  .file-table th {
    height: 36px;
    padding: 0 12px;
    color: var(--text-2);
    font-size: 12.5px;
    font-weight: 500;
    text-align: left;
  }

  .file-table td {
    height: var(--row-height);
    padding: 0 12px;
    border-top: 1px solid var(--border);
    color: var(--text);
  }

  .file-table tbody tr {
    cursor: default;
  }

  .file-table tbody tr:hover {
    background: var(--hover);
  }

  .file-table tbody tr.selected {
    background: var(--accent-soft);
  }

  .file-table tbody tr.marquee-selected,
  .file-card.marquee-selected,
  .file-tile.marquee-selected {
    background: var(--hover);
  }

  .file-table .check {
    width: 36px;
    padding-right: 0;
  }

  .file-table .check input,
  .file-card input {
    accent-color: var(--accent);
  }

  .file-table .name {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }

  .file-icon {
    display: inline-flex;
    flex: 0 0 auto;
    color: var(--text-2);
  }

  .file-name {
    display: flex;
    min-width: 0;
    overflow: hidden;
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
  }

  .name-stem {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .name-ext {
    flex: 0 0 auto;
  }

  .file-icon :global(svg) {
    stroke-width: 2;
  }

  .tag {
    flex: 0 0 auto;
    padding: 1px 6px;
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text-2);
    font-size: 12px;
  }

  .file-table .size {
    width: 96px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
    text-align: right;
  }

  .file-table .modified {
    width: 160px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }

  .file-table .actions {
    width: 44px;
    text-align: right;
  }

  .row-action {
    display: inline-grid;
    width: 28px;
    height: 28px;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: transparent;
    color: var(--text-2);
    opacity: 0;
  }

  .file-table tbody tr:hover .row-action,
  .row-action:focus-visible {
    opacity: 1;
  }

  .row-action:hover:not(:disabled) {
    background: var(--hover);
    color: var(--danger);
  }

  .row-action:disabled {
    opacity: 0;
  }

  .search-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
    color: var(--text-2);
    font-size: 14px;
  }

  .search-header .ghost {
    height: 30px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .search-results {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .search-result {
    display: grid;
    grid-template-columns: 24px minmax(0, 1.2fr) minmax(0, 1fr) 72px;
    align-items: center;
    gap: 10px;
    min-height: 42px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
  }

  .search-result:hover {
    background: var(--hover);
  }

  .search-result > :first-child {
    color: var(--text-2);
  }

  .search-path {
    overflow: hidden;
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-size {
    color: var(--text-2);
    font-size: 12.5px;
    text-align: right;
  }

  .files-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    min-height: 50vh;
    color: var(--text-2);
    font-size: 14.5px;
    text-align: center;
  }

  .retry-button {
    height: 32px;
    padding: 0 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
  }

  .tile-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px;
  }

  .file-tile {
    position: relative;
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: default;
  }

  .file-tile:hover {
    background: var(--hover);
    box-shadow: var(--shadow-1);
  }

  .file-tile.selected {
    border-color: var(--accent);
    background: var(--accent-soft);
  }

  .file-tile .card-check {
    top: 6px;
    left: 6px;
  }

  .tile-icon {
    position: relative;
    display: grid;
    width: 64px;
    height: 64px;
    flex: 0 0 64px;
    overflow: hidden;
    place-items: center;
    border-radius: var(--radius);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .tile-info {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 3px;
    overflow: hidden;
  }

  .tile-name {
    display: flex;
    width: 100%;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.35;
    text-align: left;
    white-space: nowrap;
  }

  .tile-meta {
    overflow: hidden;
    color: var(--text-2);
    font-size: 12.5px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .file-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 16px;
  }

  .file-card {
    position: relative;
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 4px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    cursor: default;
  }

  .file-card:hover {
    background: var(--hover);
    box-shadow: var(--shadow-1);
  }

  .file-card:hover .card-icon,
  .file-tile:hover .tile-icon,
  .file-card.marquee-selected .card-icon,
  .file-tile.marquee-selected .tile-icon {
    background: var(--surface);
  }

  .file-card.selected {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .card-check {
    position: absolute;
    top: 10px;
    left: 10px;
    z-index: 1;
    opacity: 0;
  }

  .file-card:hover .card-check,
  .file-card.selected .card-check,
  .card-check:focus-visible {
    opacity: 1;
  }

  .card-icon {
    position: relative;
    display: grid;
    height: 96px;
    overflow: hidden;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    color: var(--text-2);
  }

  .thumb-image {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .card-name {
    display: flex;
    min-width: 0;
    overflow: hidden;
    color: var(--text);
    font-size: 15px;
    font-weight: 600;
    line-height: 1.4;
    white-space: nowrap;
  }

  .card-meta {
    color: var(--text-2);
    font-size: 12.5px;
  }

  .upload-input {
    display: none;
  }

  .context-menu {
    position: fixed;
    z-index: 55;
    display: flex;
    min-width: 160px;
    flex-direction: column;
    padding: 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .context-menu button {
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: 13.5px;
    text-align: left;
  }

  .context-menu button:hover {
    background: var(--hover);
  }

  .context-menu button.danger {
    color: var(--danger);
  }

  .menu-item {
    position: relative;
  }

  .menu-item > button {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }

  .menu-arrow {
    margin-left: auto;
    color: var(--text-2);
  }

  .context-menu.open-left .submenu {
    right: calc(100% - 2px);
    left: auto;
  }

  .submenu {
    position: absolute;
    top: 0;
    left: calc(100% - 2px);
    z-index: 56;
    display: none;
    min-width: 150px;
    flex-direction: column;
    padding: 5px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .menu-item:hover > .submenu,
  .menu-item:focus-within > .submenu {
    display: flex;
  }

  .submenu button {
    height: 32px;
    padding: 0 10px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text);
    font-size: 13.5px;
    text-align: left;
  }

  .submenu button:hover {
    background: var(--hover);
  }

  .menu-sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }

  .marquee {
    position: fixed;
    z-index: 40;
    border: 1px solid var(--accent);
    background: var(--accent-soft);
    opacity: 0.55;
    pointer-events: none;
  }

  .editor-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--scrim);
  }

  .editor-panel {
    display: flex;
    width: min(900px, 100%);
    height: min(80vh, 720px);
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    box-shadow: var(--shadow-2);
  }

  .editor-header,
  .editor-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-height: 52px;
    padding: 0 14px;
    border-bottom: 1px solid var(--border);
  }

  .editor-footer {
    border-top: 1px solid var(--border);
    border-bottom: 0;
    color: var(--text-2);
    font-size: 12.5px;
  }

  .editor-header strong {
    overflow: hidden;
    font-size: 16px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .editor-actions,
  .editor-footer > div {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .editor-body {
    display: flex;
    min-height: 0;
    flex: 1;
  }

  .line-numbers span {
    display: block;
  }

  .line-numbers {
    width: 52px;
    min-width: 52px;
    margin: 0;
    padding: 16px 8px 16px 0;
    overflow: hidden;
    border-right: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-2);
    font-family: var(--font-mono);
    font-size: 14.5px;
    line-height: 1.6;
    text-align: right;
    user-select: none;
  }

  .editor-panel textarea {
    min-width: 0;
    flex: 1;
    padding: 16px;
    border: 0;
    outline: 0;
    resize: none;
    background: var(--surface);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: 14.5px;
    line-height: 1.6;
  }

  .markdown-preview {
    flex: 1;
    overflow: auto;
    padding: 20px;
    color: var(--text);
    font-size: 16px;
    line-height: 1.7;
  }

  .markdown-preview :global(h1),
  .markdown-preview :global(h2),
  .markdown-preview :global(h3) {
    color: var(--text);
    line-height: 1.4;
  }

  .markdown-preview :global(code),
  .markdown-preview :global(pre) {
    background: var(--surface-2);
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
  }

  .markdown-preview :global(pre) {
    overflow: auto;
    padding: 12px;
  }

  .markdown-preview :global(a) {
    color: var(--accent-text);
  }

  .editor-footer .ghost,
  .editor-actions .ghost {
    height: 32px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
  }

  .editor-footer .primary {
    height: 32px;
    padding: 0 14px;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--on-accent);
  }

  .editor-error {
    margin: 0;
    padding: 8px 14px;
    color: var(--danger);
    font-size: 13.5px;
  }

  .load-more {
    padding: 18px;
    color: var(--text-2);
    font-size: 13px;
    text-align: center;
  }

  .empty-state {
    display: flex;
    min-height: calc(
      100dvh - var(--topbar-height) - var(--pathbar-height) - 2 * var(--content-pad)
    );
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    margin: auto;
    color: var(--text-2);
    text-align: center;
  }

  .empty-icon {
    display: grid;
    width: 88px;
    height: 88px;
    margin-bottom: 6px;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 20px;
    background: var(--surface);
    color: var(--text-2);
  }

  h1 {
    margin: 0;
    color: var(--text);
    font-size: 22px;
    font-weight: 500;
    line-height: 1.4;
  }

  p {
    max-width: 420px;
    margin: 0;
    font-size: 14.5px;
  }

  .overlay {
    display: none;
  }

  @media (max-width: 720px) {
    .file-grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
    }

    .card-icon {
      height: 84px;
    }

    .tile-grid {
      grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    }

    .file-table .size,
    .file-table .modified {
      display: none;
    }

    .editor-overlay {
      padding: 0;
    }

    .editor-panel {
      width: 100%;
      height: 100dvh;
      border-radius: 0;
    }
  }

  @media (max-width: 900px) {
    .workspace {
      grid-template-columns: 1fr;
    }

    .sidebar {
      position: fixed;
      inset: 0 auto 0 0;
      z-index: 30;
      width: 280px;
      transform: translateX(-100%);
      transition: transform var(--motion-base) var(--ease);
    }

    .sidebar.open {
      transform: translateX(0);
    }

    .mobile-menu {
      display: grid;
    }

    .overlay {
      position: fixed;
      inset: 0;
      z-index: 20;
      display: block;
      border: 0;
      background: var(--scrim);
    }

    .topbar {
      gap: 8px;
      padding: 0 12px;
    }

    .topbar-brand {
      width: auto;
    }

    .search {
      max-width: none;
    }

    .upload-top {
      padding: 0 10px;
    }

    .upload-top span {
      display: none;
    }

    .pathbar-actions {
      display: flex;
      flex: 0 0 auto;
      align-items: center;
      gap: 2px;
    }

    .selection-count {
      padding: 0 6px;
      color: var(--accent-text);
      font-size: 12.5px;
      white-space: nowrap;
    }

    .pathbar-actions .danger:hover {
      color: var(--danger);
    }

    .pathbar {
      padding: 0 16px;
    }

    .content {
      padding: 16px;
    }
  }

  @media (max-width: 520px) {
    .segmented {
      display: none;
    }
  }
</style>
